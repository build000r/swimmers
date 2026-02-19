use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::scroll::guard::ScrollGuard;
use crate::session::replay_ring::ReplayRing;
use crate::state::detector::StateDetector;
use crate::types::{
    ControlEvent, SessionState, SessionStatePayload, SessionSummary, SessionTitlePayload,
    TerminalSnapshot, TransportHealth,
};

const CWD_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(750);

// ---------------------------------------------------------------------------
// Public command enum -- sent to the actor over its mpsc channel
// ---------------------------------------------------------------------------

/// Uniquely identifies a connected client's output subscription.
pub type ClientId = u64;

/// A framed chunk of terminal output with its sequence number.
#[derive(Debug, Clone)]
pub struct OutputFrame {
    pub seq: u64,
    pub data: Vec<u8>,
}

/// Commands that the rest of the system can send to a session actor.
#[derive(Debug)]
pub enum SessionCommand {
    /// Write raw bytes to the PTY (user input).
    WriteInput(Vec<u8>),

    /// Resize the PTY.
    Resize { cols: u16, rows: u16 },

    /// Clear the attention state.
    DismissAttention,

    /// Subscribe a new client to terminal output.
    /// The `resume_from_seq` lets the client request replay.
    Subscribe {
        client_id: ClientId,
        client_tx: mpsc::Sender<OutputFrame>,
        resume_from_seq: Option<u64>,
        ack: oneshot::Sender<SubscribeOutcome>,
    },

    /// Remove a client subscription.
    Unsubscribe { client_id: ClientId },

    /// Request a terminal text snapshot (reply via oneshot).
    GetSnapshot(oneshot::Sender<TerminalSnapshot>),

    /// Request plain captured pane text from tmux for preview use.
    GetPaneTail {
        lines: usize,
        reply: oneshot::Sender<String>,
    },

    /// Request a session summary (reply via oneshot).
    GetSummary(oneshot::Sender<SessionSummary>),

    /// Request replay cursor metadata for lifecycle acknowledgments.
    GetReplayCursor(oneshot::Sender<ReplayCursor>),

    /// Graceful shutdown -- detach from tmux, do NOT kill the tmux session.
    Shutdown,
}

/// Subscribe result returned to the websocket layer.
#[derive(Debug)]
pub enum SubscribeOutcome {
    Ok,
    ReplayTruncated {
        requested_resume_from_seq: u64,
        replay_window_start_seq: u64,
        latest_seq: u64,
    },
}

/// Lightweight replay cursor metadata for lifecycle acknowledgments.
#[derive(Debug, Clone, Copy)]
pub struct ReplayCursor {
    pub latest_seq: u64,
    pub replay_window_start_seq: u64,
}

// ---------------------------------------------------------------------------
// Actor handle -- cheaply cloneable reference to a running actor
// ---------------------------------------------------------------------------

/// A lightweight handle that other components hold to talk to a session actor.
#[derive(Debug, Clone)]
pub struct ActorHandle {
    pub session_id: String,
    pub tmux_name: String,
    pub cmd_tx: mpsc::Sender<SessionCommand>,
    /// Per-session broadcast channel for ControlEvents (session_state, session_title).
    /// Multiple WS clients can subscribe to the same session's events.
    event_tx: broadcast::Sender<ControlEvent>,
}

impl ActorHandle {
    pub async fn send(
        &self,
        cmd: SessionCommand,
    ) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.cmd_tx.send(cmd).await
    }

    /// Subscribe to this session's control events (state changes, title updates).
    pub fn subscribe_events(&self) -> broadcast::Receiver<ControlEvent> {
        self.event_tx.subscribe()
    }
}

// ---------------------------------------------------------------------------
// Session actor
// ---------------------------------------------------------------------------

pub struct SessionActor {
    session_id: String,
    tmux_name: String,
    #[allow(dead_code)]
    config: Arc<Config>,

    // PTY
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,

    // Processing pipeline
    state_detector: StateDetector,
    scroll_guard: ScrollGuard,
    replay_ring: ReplayRing,

    // Subscribers (client_id -> bounded sender)
    subscribers: HashMap<ClientId, mpsc::Sender<OutputFrame>>,

    // Inbound command channel
    cmd_rx: mpsc::Receiver<SessionCommand>,

    // Per-session event broadcast for ControlEvents (session_state changes).
    event_tx: broadcast::Sender<ControlEvent>,

    // Cols/rows for summary reporting
    cols: u16,
    rows: u16,

    // Working directory extracted from OSC 7 or OSC 0/2 title sequences
    cwd: String,

    // Last time we polled tmux for pane_current_path.
    last_cwd_refresh_at: Instant,

    // Detected coding tool name
    tool: Option<String>,

    // Timestamp of most recent terminal output observed by this actor.
    last_activity_at: chrono::DateTime<Utc>,

    // When true, the replay ring will be cleared on the first idle transition.
    // This strips tmux startup output (including DA query responses) before
    // any client subscribes.
    clear_replay_on_first_idle: bool,
}

impl SessionActor {
    /// Spawn a new session actor. If `attach` is true, attaches to an existing
    /// tmux session; otherwise creates a new one.
    ///
    /// Returns an `ActorHandle` that callers use to send commands to the actor.
    pub fn spawn(
        session_id: String,
        tmux_name: String,
        attach: bool,
        start_cwd: Option<String>,
        config: Arc<Config>,
    ) -> anyhow::Result<ActorHandle> {
        let pty_system = native_pty_system();

        let initial_size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(initial_size)
            .map_err(|e| anyhow::anyhow!("failed to open PTY: {}", e))?;

        // Build the tmux command. Clean TMUX / TMUX_PANE from the environment
        // so that tmux works even when the throngterm server itself runs inside
        // a tmux session.
        let mut cmd = if attach {
            let mut c = CommandBuilder::new("tmux");
            c.args(["attach-session", "-t", &tmux_name]);
            c
        } else {
            let mut c = CommandBuilder::new("tmux");
            c.args(["new-session", "-s", &tmux_name]);
            if let Some(dir) = start_cwd.as_deref() {
                c.args(["-c", dir]);
            }
            c
        };

        // Strip tmux-related env vars to avoid nesting issues.
        cmd.env_remove("TMUX");
        cmd.env_remove("TMUX_PANE");

        let _child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("failed to spawn tmux: {}", e))?;

        // We intentionally drop the slave side -- the master side is what we use.
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("failed to take PTY writer: {}", e))?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCommand>(256);
        let (event_tx, _) = broadcast::channel::<ControlEvent>(64);

        let replay_ring = ReplayRing::new(config.replay_buffer_size);
        let initial_cwd = start_cwd.unwrap_or_default();

        let actor = SessionActor {
            session_id: session_id.clone(),
            tmux_name: tmux_name.clone(),
            config: config.clone(),
            master: pair.master,
            writer,
            state_detector: StateDetector::new(),
            scroll_guard: ScrollGuard::new(),
            replay_ring,
            subscribers: HashMap::new(),
            cmd_rx,
            event_tx: event_tx.clone(),
            cols: 80,
            rows: 24,
            cwd: initial_cwd,
            last_cwd_refresh_at: Instant::now(),
            tool: None,
            last_activity_at: Utc::now(),
            clear_replay_on_first_idle: !attach,
        };

        // Spawn the actor's run loop on the Tokio runtime.
        tokio::spawn(actor.run());

        let handle = ActorHandle {
            session_id,
            tmux_name,
            cmd_tx,
            event_tx,
        };

        Ok(handle)
    }

    /// Main actor loop. Owns all mutable state for this session.
    async fn run(mut self) {
        info!(session_id = %self.session_id, tmux = %self.tmux_name, "session actor started");

        // Spawn a blocking task to read from the PTY. Output is forwarded
        // through a bounded channel.
        let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(256);
        let session_id_for_reader = self.session_id.clone();

        // Take a reader from the master PTY for the blocking read loop.
        let reader = match self.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                error!(session_id = %self.session_id, "failed to clone PTY reader: {}", e);
                return;
            }
        };

        tokio::task::spawn_blocking(move || {
            pty_read_loop(session_id_for_reader, reader, pty_tx);
        });

        // Prime cwd from tmux's authoritative pane path.
        self.maybe_refresh_cwd_from_tmux(true).await;

        let mut pty_closed = false;

        loop {
            // Compute the next timer deadline from both StateDetector and ScrollGuard.
            let next_timer = self.next_timer_deadline();

            tokio::select! {
                // --- PTY output ---
                result = pty_rx.recv(), if !pty_closed => {
                    match result {
                        Some(raw) => {
                            self.handle_pty_output(raw).await;
                        }
                        None => {
                            info!(session_id = %self.session_id, "PTY channel closed (process exit)");
                            pty_closed = true;
                            // Emit session_state with exit_reason = "process_exit"
                            let prev = self.state_detector.state();
                            let payload = SessionStatePayload {
                                state: SessionState::Exited,
                                previous_state: prev,
                                current_command: self.state_detector.current_command(),
                                transport_health: TransportHealth::Healthy,
                                exit_reason: Some("process_exit".to_string()),
                                at: Utc::now(),
                            };
                            let event = ControlEvent {
                                event: "session_state".to_string(),
                                session_id: self.session_id.clone(),
                                payload: serde_json::to_value(&payload).unwrap_or_default(),
                            };
                            let _ = self.event_tx.send(event);
                        }
                    }
                }

                // --- Inbound commands ---
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        SessionCommand::WriteInput(data) => {
                            if pty_closed {
                                debug!(session_id = %self.session_id, "ignoring write to exited PTY");
                            } else {
                                self.scroll_guard.notify_input();
                                if let Err(e) = self.writer.write_all(&data) {
                                    error!(session_id = %self.session_id, "PTY write error: {}", e);
                                }
                            }
                        }
                        SessionCommand::Resize { cols, rows } => {
                            self.cols = cols;
                            self.rows = rows;
                            let size = PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            };
                            if let Err(e) = self.master.resize(size) {
                                error!(session_id = %self.session_id, "PTY resize error: {}", e);
                            }
                        }
                        SessionCommand::DismissAttention => {
                            let state_before = self.state_detector.state();
                            self.state_detector.dismiss_attention();
                            if matches!(
                                self.maybe_emit_state_change(state_before),
                                Some(SessionState::Idle)
                            ) {
                                self.maybe_refresh_cwd_from_tmux(false).await;
                            }
                        }
                        SessionCommand::Subscribe {
                            client_id,
                            client_tx,
                            resume_from_seq,
                            ack,
                        } => {
                            let outcome = self
                                .handle_subscribe(client_id, client_tx, resume_from_seq)
                                .await;
                            let _ = ack.send(outcome);
                        }
                        SessionCommand::Unsubscribe { client_id } => {
                            self.subscribers.remove(&client_id);
                            debug!(session_id = %self.session_id, client_id, "client unsubscribed");
                        }
                        SessionCommand::GetSnapshot(reply) => {
                            let snap = TerminalSnapshot {
                                session_id: self.session_id.clone(),
                                latest_seq: self.replay_ring.latest_seq(),
                                truncated: false,
                                screen_text: self.replay_ring.snapshot(),
                            };
                            let _ = reply.send(snap);
                        }
                        SessionCommand::GetPaneTail { lines, reply } => {
                            let tmux_name = self.tmux_name.clone();
                            let text = match capture_pane_tail(&tmux_name, lines).await {
                                Ok(text) => text,
                                Err(e) => {
                                    debug!(
                                        session_id = %self.session_id,
                                        tmux_name = %tmux_name,
                                        "tmux capture-pane failed: {}",
                                        e
                                    );
                                    String::new()
                                }
                            };
                            let _ = reply.send(text);
                        }
                        SessionCommand::GetSummary(reply) => {
                            self.maybe_refresh_cwd_from_tmux(false).await;
                            let summary = self.build_summary();
                            let _ = reply.send(summary);
                        }
                        SessionCommand::GetReplayCursor(reply) => {
                            let _ = reply.send(ReplayCursor {
                                latest_seq: self.replay_ring.latest_seq(),
                                replay_window_start_seq: self.replay_ring.window_start_seq(),
                            });
                        }
                        SessionCommand::Shutdown => {
                            info!(session_id = %self.session_id, "shutdown requested, detaching");
                            break;
                        }
                    }
                }

                // --- Timer tick for state detector / scroll guard deadlines ---
                _ = Self::sleep_until_deadline(next_timer) => {
                    self.fire_timers().await;
                }

                // If both channels close, the actor exits.
                else => {
                    info!(session_id = %self.session_id, "all channels closed, actor exiting");
                    break;
                }
            }
        }

        info!(session_id = %self.session_id, "session actor stopped");
    }

    /// Sleep until the given deadline, or pend forever if there is no deadline.
    /// Used inside `tokio::select!` to wake the actor for timer-driven transitions.
    async fn sleep_until_deadline(deadline: Option<Instant>) {
        match deadline {
            Some(d) => {
                let now = Instant::now();
                if d > now {
                    tokio::time::sleep(d - now).await;
                }
                // If d <= now, return immediately so timers fire.
            }
            None => {
                // No deadline -- pend forever (other select branches will fire).
                std::future::pending::<()>().await;
            }
        }
    }

    /// Compute the earliest timer deadline across StateDetector and ScrollGuard.
    fn next_timer_deadline(&self) -> Option<Instant> {
        let state_deadline = self.state_detector.next_deadline();
        let scroll_deadline = self.scroll_guard.check_flush_deadline();
        match (state_deadline, scroll_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Fire any expired timers and process the results.
    async fn fire_timers(&mut self) {
        // Snapshot state before timers for change detection.
        let state_before = self.state_detector.state();

        // Check state detector timers (error auto-clear, idle -> attention).
        self.state_detector.check_timers(Instant::now());

        // Emit state change event if timers caused a transition.
        if matches!(
            self.maybe_emit_state_change(state_before),
            Some(SessionState::Idle)
        ) {
            self.maybe_refresh_cwd_from_tmux(false).await;
        }

        // Flush any coalesced scroll guard data.
        if let Some(flushed) = self.scroll_guard.flush() {
            let state_before = self.state_detector.state();
            self.state_detector.process_output(&flushed);
            if matches!(
                self.maybe_emit_state_change(state_before),
                Some(SessionState::Idle)
            ) {
                self.maybe_refresh_cwd_from_tmux(false).await;
            }
            self.last_activity_at = Utc::now();

            let seq = self.replay_ring.push(&flushed);
            let frame = OutputFrame { seq, data: flushed };
            self.broadcast(frame).await;
        }
    }

    /// Process raw PTY output through the pipeline:
    /// ScrollGuard -> StateDetector -> ReplayRing -> broadcast.
    ///
    /// ScrollGuard returns zero or more chunks (it may buffer for coalescing,
    /// flush a previous buffer alongside new data, or pass through directly).
    async fn handle_pty_output(&mut self, raw: Vec<u8>) {
        // Detect OSC title sequences in raw output before processing.
        self.detect_and_emit_title(&raw);

        let chunks = self.scroll_guard.process(&raw);

        for chunk in chunks {
            // Snapshot state before processing for change detection.
            let state_before = self.state_detector.state();

            // Feed the state detector with each chunk.
            self.state_detector.process_output(&chunk);

            // Emit state change event if processing caused a transition.
            if matches!(
                self.maybe_emit_state_change(state_before),
                Some(SessionState::Idle)
            ) {
                self.maybe_refresh_cwd_from_tmux(false).await;
            }

            // On new sessions, clear the replay ring when we first reach idle.
            // This strips tmux startup output (including DA query/response
            // sequences) so clients subscribing later get a clean prompt.
            if self.clear_replay_on_first_idle && self.state_detector.state() == SessionState::Idle
            {
                self.clear_replay_on_first_idle = false;
                self.replay_ring.clear();
                debug!(
                    session_id = %self.session_id,
                    "cleared replay ring on first idle (startup garbage removed)"
                );
            }

            self.last_activity_at = Utc::now();

            // Store in the replay ring and get the sequence number.
            let seq = self.replay_ring.push(&chunk);

            let frame = OutputFrame { seq, data: chunk };

            // Broadcast to all subscribers.
            self.broadcast(frame).await;

            // Report aggregate queue depth across subscribers.
            let total_depth: usize = self
                .subscribers
                .values()
                .map(|tx| tx.max_capacity() - tx.capacity())
                .sum();
            crate::metrics::record_queue_depth(&self.session_id, total_depth);
        }
    }

    /// Detect OSC title and CWD sequences in raw PTY output.
    ///
    /// OSC 0: `\x1b]0;title\x07` -- set window title + icon name
    /// OSC 2: `\x1b]2;title\x07` -- set window title
    /// OSC 7: `\x1b]7;file://host/path\x07` -- set working directory
    ///
    /// Emits `session_title` ControlEvents and updates internal cwd state.
    fn detect_and_emit_title(&mut self, raw: &[u8]) {
        let text = String::from_utf8_lossy(raw);

        // OSC 7: working directory notification (file://host/path)
        {
            let prefix = "\x1b]7;";
            let mut search_from = 0;
            while let Some(start) = text[search_from..].find(prefix) {
                let uri_start = search_from + start + prefix.len();
                // BEL or ST (ESC \) terminates the sequence
                let end = text[uri_start..]
                    .find('\x07')
                    .or_else(|| text[uri_start..].find("\x1b\\"));
                if let Some(end_offset) = end {
                    let uri = &text[uri_start..uri_start + end_offset];
                    // Parse file:// URI → extract path
                    if let Some(path) = uri.strip_prefix("file://") {
                        // file://hostname/path — skip hostname (up to next /)
                        let path = if let Some(slash_pos) = path.find('/') {
                            &path[slash_pos..]
                        } else {
                            path
                        };
                        // URL-decode percent-encoded characters
                        let decoded = percent_decode(path);
                        self.update_cwd_and_emit(decoded);
                    }
                    search_from = uri_start + end_offset + 1;
                } else {
                    break;
                }
            }
        }

        // OSC 0/2: window title (often contains cwd for shells)
        for prefix in &["\x1b]0;", "\x1b]2;"] {
            let mut search_from = 0;
            while let Some(start) = text[search_from..].find(prefix) {
                let title_start = search_from + start + prefix.len();
                if let Some(end_offset) = text[title_start..].find('\x07') {
                    let title = &text[title_start..title_start + end_offset];
                    if !title.is_empty() {
                        // If we don't have a cwd from OSC 7, try to extract from title.
                        // Common formats: "user@host: /path" or just "/path"
                        if self.cwd.is_empty() {
                            if let Some(extracted) = extract_cwd_from_title(title) {
                                self.cwd = extracted;
                            }
                        }

                        // Detect tool name from title (e.g. "claude" in title)
                        if self.tool.is_none() {
                            self.tool = detect_tool_from_title(title);
                        }

                        let payload = SessionTitlePayload {
                            title: title.to_string(),
                            at: Utc::now(),
                        };
                        let event = ControlEvent {
                            event: "session_title".to_string(),
                            session_id: self.session_id.clone(),
                            payload: serde_json::to_value(&payload).unwrap_or_default(),
                        };
                        let _ = self.event_tx.send(event);
                    }
                    search_from = title_start + end_offset + 1;
                } else {
                    break;
                }
            }
        }
    }

    async fn maybe_refresh_cwd_from_tmux(&mut self, force: bool) {
        if !force && self.state_detector.state() != SessionState::Idle {
            return;
        }
        if !force && self.last_cwd_refresh_at.elapsed() < CWD_REFRESH_MIN_INTERVAL {
            return;
        }
        self.last_cwd_refresh_at = Instant::now();

        let tmux_name = self.tmux_name.clone();
        match query_tmux_cwd(&tmux_name).await {
            Ok(cwd) => self.update_cwd_and_emit(cwd),
            Err(e) => {
                debug!(
                    session_id = %self.session_id,
                    tmux_name = %tmux_name,
                    "tmux cwd refresh failed: {}",
                    e
                );
            }
        }
    }

    fn update_cwd_and_emit(&mut self, cwd: String) {
        let normalized = cwd.trim();
        if normalized.is_empty() || normalized == self.cwd {
            return;
        }

        self.cwd = normalized.to_string();
        let payload = SessionTitlePayload {
            title: self.cwd.clone(),
            at: Utc::now(),
        };
        let event = ControlEvent {
            event: "session_title".to_string(),
            session_id: self.session_id.clone(),
            payload: serde_json::to_value(&payload).unwrap_or_default(),
        };
        let _ = self.event_tx.send(event);
    }

    /// Compare state before and after a detector operation. If the state changed,
    /// emit a `session_state` ControlEvent through the per-session broadcast channel.
    fn maybe_emit_state_change(&self, previous_state: SessionState) -> Option<SessionState> {
        self.maybe_emit_state_change_with_exit_reason(previous_state, None)
    }

    /// Emit a `session_state` ControlEvent if the state changed, optionally
    /// including an `exit_reason` for terminal exit events.
    fn maybe_emit_state_change_with_exit_reason(
        &self,
        previous_state: SessionState,
        exit_reason: Option<String>,
    ) -> Option<SessionState> {
        let (current_state, current_command) = self.state_detector.get_state();
        if current_state != previous_state {
            let payload = SessionStatePayload {
                state: current_state,
                previous_state,
                current_command,
                transport_health: TransportHealth::Healthy,
                exit_reason,
                at: Utc::now(),
            };
            let event = ControlEvent {
                event: "session_state".to_string(),
                session_id: self.session_id.clone(),
                payload: serde_json::to_value(&payload).unwrap_or_default(),
            };
            // If no receivers, send returns Err -- that's fine, nobody is listening.
            let _ = self.event_tx.send(event);
            Some(current_state)
        } else {
            None
        }
    }

    /// Send a frame to all subscribers. Detects overloaded subscribers whose
    /// channels are full, and removes them.
    async fn broadcast(&mut self, frame: OutputFrame) {
        let mut to_remove: Vec<ClientId> = Vec::new();

        for (&client_id, tx) in &self.subscribers {
            match tx.try_send(frame.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!(
                        session_id = %self.session_id,
                        client_id,
                        "subscriber channel full (SESSION_OVERLOADED), dropping client"
                    );
                    crate::metrics::increment_overload(&self.session_id);
                    to_remove.push(client_id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!(session_id = %self.session_id, client_id, "subscriber channel closed");
                    to_remove.push(client_id);
                }
            }
        }

        for id in to_remove {
            self.subscribers.remove(&id);
        }
    }

    /// Handle a new subscriber, including replay of buffered frames.
    async fn handle_subscribe(
        &mut self,
        client_id: ClientId,
        client_tx: mpsc::Sender<OutputFrame>,
        resume_from_seq: Option<u64>,
    ) -> SubscribeOutcome {
        info!(
            session_id = %self.session_id,
            client_id,
            resume_from_seq = ?resume_from_seq,
            "client subscribing"
        );

        let mut outcome = SubscribeOutcome::Ok;

        // If the client wants replay, send buffered frames first.
        if let Some(from_seq) = resume_from_seq {
            let replay_from_seq = from_seq.saturating_add(1);
            match self.replay_ring.replay_from(replay_from_seq) {
                Some(frames) => {
                    for (seq, data) in frames {
                        let frame = OutputFrame { seq, data };
                        if client_tx.send(frame).await.is_err() {
                            warn!(
                                session_id = %self.session_id,
                                client_id,
                                "subscriber dropped during replay"
                            );
                            return SubscribeOutcome::Ok;
                        }
                    }
                }
                None => {
                    // Data has been truncated. We still add the subscriber but
                    // the caller should check replay_ring state and send a
                    // REPLAY_TRUNCATED control event.
                    warn!(
                        session_id = %self.session_id,
                        client_id,
                        from_seq,
                        window_start = self.replay_ring.window_start_seq(),
                        "replay truncated, client needs full refresh"
                    );
                    outcome = SubscribeOutcome::ReplayTruncated {
                        requested_resume_from_seq: from_seq,
                        replay_window_start_seq: self.replay_ring.window_start_seq(),
                        latest_seq: self.replay_ring.latest_seq(),
                    };
                }
            }
        }

        self.subscribers.insert(client_id, client_tx);
        outcome
    }

    /// Build a summary snapshot of this session's current state.
    fn build_summary(&self) -> SessionSummary {
        let (state, current_command) = self.state_detector.get_state();
        let context_limit = crate::types::context_limit_for_tool(self.tool.as_deref());
        SessionSummary {
            session_id: self.session_id.clone(),
            tmux_name: self.tmux_name.clone(),
            state,
            current_command,
            cwd: self.cwd.clone(),
            tool: self.tool.clone(),
            token_count: 0,
            context_limit,
            thought: None,
            is_stale: false,
            attached_clients: self.subscribers.len() as u32,
            transport_health: TransportHealth::Healthy,
            last_activity_at: self.last_activity_at,
        }
    }
}

/// Capture visible pane text directly from tmux.
async fn capture_pane_tail(tmux_name: &str, lines: usize) -> anyhow::Result<String> {
    let lines = lines.clamp(20, 1000);
    let start = format!("-{lines}");

    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-J", "-t", tmux_name, "-S", &start])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tmux capture-pane: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux capture-pane failed: {}",
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Query tmux for the active pane cwd of a session.
async fn query_tmux_cwd(tmux_name: &str) -> anyhow::Result<String> {
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            tmux_name,
            "#{pane_current_path}",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tmux display-message: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux display-message failed: {}",
            stderr.trim()
        ));
    }

    let cwd = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if cwd.is_empty() {
        return Err(anyhow::anyhow!("tmux returned empty pane_current_path"));
    }
    Ok(cwd)
}

// ---------------------------------------------------------------------------
// Title / CWD helpers
// ---------------------------------------------------------------------------

/// Decode percent-encoded characters in a URI path (e.g. `%20` -> ` `).
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(h), Some(l)) = (hi, lo) {
                let hex = [h, l];
                if let Ok(s) = std::str::from_utf8(&hex) {
                    if let Ok(val) = u8::from_str_radix(s, 16) {
                        out.push(val as char);
                        continue;
                    }
                }
            }
            out.push(b as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

/// Try to extract a cwd path from an OSC 0/2 window title.
/// Common formats: "user@host: /path", "user@host:/path", "/path/to/dir"
fn extract_cwd_from_title(title: &str) -> Option<String> {
    // "user@host: /path" or "user@host:/path"
    if let Some(pos) = title.find(": /").or_else(|| title.find(":/")) {
        let path_start = if title[pos..].starts_with(": ") {
            pos + 2
        } else {
            pos + 1
        };
        let path = title[path_start..].trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    // Plain absolute path
    if title.starts_with('/') {
        return Some(title.trim().to_string());
    }
    // "~" or "~/something"
    if title.starts_with('~') {
        if let Some(home) = std::env::var("HOME").ok() {
            let expanded = title.replacen('~', &home, 1);
            return Some(expanded);
        }
        return Some(title.trim().to_string());
    }
    None
}

/// Detect a coding tool name from the window title.
fn detect_tool_from_title(title: &str) -> Option<String> {
    let lower = title.to_lowercase();
    // Check for known tool process names in the title
    for (pattern, name) in &[
        ("claude", "Claude Code"),
        ("codex", "Codex"),
        ("aider", "Aider"),
        ("goose", "Goose"),
        ("cline", "Cline"),
    ] {
        if lower.contains(pattern) {
            return Some(name.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Blocking PTY reader (runs in spawn_blocking)
// ---------------------------------------------------------------------------

fn pty_read_loop(
    session_id: String,
    mut reader: Box<dyn std::io::Read + Send>,
    tx: mpsc::Sender<Vec<u8>>,
) {
    use std::io::Read;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                info!(session_id = %session_id, "PTY EOF");
                break;
            }
            Ok(n) => {
                let data = buf[..n].to_vec();
                if tx.blocking_send(data).is_err() {
                    debug!(session_id = %session_id, "PTY read loop: receiver dropped");
                    break;
                }
            }
            Err(e) => {
                // EIO is expected when the child process exits.
                if e.kind() == std::io::ErrorKind::Other {
                    info!(session_id = %session_id, "PTY read ended (likely child exit)");
                } else {
                    error!(session_id = %session_id, "PTY read error: {}", e);
                }
                break;
            }
        }
    }
}
