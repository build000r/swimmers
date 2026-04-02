use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use regex::Regex;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::scroll::guard::{ScrollGuard, ScrollOutputChunk};
use crate::session::artifacts::{
    default_artifact_registry, ArtifactDiscoveryContext, ArtifactKind,
};
use crate::session::replay_ring::ReplayRing;
use crate::state::detector::StateDetector;
use crate::tmux_target::{exact_pane_target, exact_session_target};
use crate::types::{
    ControlEvent, MermaidArtifactResponse, SessionSkillPayload, SessionState, SessionStatePayload,
    SessionSummary, SessionTitlePayload, TerminalSnapshot, TransportHealth,
};

const CWD_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(750);
const TOOL_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(1_000);
const TMUX_FALLBACK_TERM: &str = "xterm-256color";
const TMUX_FALLBACK_COLORTERM: &str = "truecolor";

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

    /// Request latest Mermaid artifact metadata and source for this session.
    GetMermaidArtifact(oneshot::Sender<MermaidArtifactResponse>),

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

enum ReplayPlan {
    None,
    Frames(Vec<(u64, Vec<u8>)>),
    Truncated {
        requested_resume_from_seq: u64,
        replay_window_start_seq: u64,
        latest_seq: u64,
    },
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

    #[cfg(test)]
    pub fn test_handle(
        session_id: impl Into<String>,
        tmux_name: impl Into<String>,
        cmd_tx: mpsc::Sender<SessionCommand>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(16);
        Self {
            session_id: session_id.into(),
            tmux_name: tmux_name.into(),
            cmd_tx,
            event_tx,
        }
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

    // Last time we refreshed tool detection from tmux/process state.
    last_tool_refresh_at: Instant,

    // Detected coding tool name
    tool: Option<String>,

    // Most recent detected skill invocation (e.g. "$describe").
    last_skill: Option<String>,

    // Buffered input line used for skill invocation detection.
    input_line_buffer: String,

    // Timestamp of most recent terminal output observed by this actor.
    last_activity_at: chrono::DateTime<Utc>,

    // Session creation time used as the baseline for session-scoped artifacts.
    // For attached sessions this is refreshed from tmux metadata.
    session_started_at: chrono::DateTime<Utc>,

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
        initial_tool: Option<String>,
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
        // so that tmux works even when the swimmers server itself runs inside
        // a tmux session.
        let mut cmd = if attach {
            let mut c = CommandBuilder::new("tmux");
            let target = exact_session_target(&tmux_name);
            c.args(["attach-session", "-t", &target]);
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
        let inherited_term = std::env::var("TERM").ok();
        let inherited_colorterm = std::env::var("COLORTERM").ok();
        let (tmux_term, tmux_colorterm, used_term_fallback) =
            resolve_tmux_terminal_env(inherited_term.as_deref(), inherited_colorterm.as_deref());
        cmd.env("TERM", &tmux_term);
        cmd.env("COLORTERM", &tmux_colorterm);
        cmd.env("TERM_PROGRAM", "swimmers");
        if used_term_fallback {
            warn!(
                session_id = %session_id,
                tmux_name = %tmux_name,
                inherited_term = ?inherited_term,
                applied_term = %tmux_term,
                "missing/unsupported TERM for tmux client; applied fallback"
            );
        } else {
            debug!(
                session_id = %session_id,
                tmux_name = %tmux_name,
                inherited_term = ?inherited_term,
                applied_term = %tmux_term,
                colorterm = %tmux_colorterm,
                "configured tmux client terminal environment"
            );
        }

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
            last_tool_refresh_at: Instant::now(),
            tool: initial_tool,
            last_skill: None,
            input_line_buffer: String::new(),
            last_activity_at: Utc::now(),
            session_started_at: Utc::now(),
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
        let Some(mut pty_rx) = self.start_pty_reader() else {
            return;
        };
        self.prime_tmux_metadata().await;

        let mut pty_closed = false;
        while self.run_iteration(&mut pty_rx, &mut pty_closed).await {}

        info!(session_id = %self.session_id, "session actor stopped");
    }

    fn start_pty_reader(&self) -> Option<mpsc::Receiver<Vec<u8>>> {
        let (pty_tx, pty_rx) = mpsc::channel::<Vec<u8>>(256);
        let session_id_for_reader = self.session_id.clone();
        let reader = match self.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(e) => {
                error!(session_id = %self.session_id, "failed to clone PTY reader: {}", e);
                return None;
            }
        };

        tokio::task::spawn_blocking(move || {
            pty_read_loop(session_id_for_reader, reader, pty_tx);
        });

        Some(pty_rx)
    }

    async fn prime_tmux_metadata(&mut self) {
        self.maybe_refresh_session_started_at().await;
        self.maybe_refresh_cwd_from_tmux(true).await;
        self.maybe_refresh_tool_from_tmux(true).await;
    }

    async fn run_iteration(
        &mut self,
        pty_rx: &mut mpsc::Receiver<Vec<u8>>,
        pty_closed: &mut bool,
    ) -> bool {
        let next_timer = self.next_timer_deadline();
        tokio::select! {
            result = pty_rx.recv(), if !*pty_closed => {
                self.handle_pty_read_result(result, pty_closed).await;
                true
            }
            Some(cmd) = self.cmd_rx.recv() => self.handle_command(cmd, *pty_closed).await,
            _ = Self::sleep_until_deadline(next_timer) => {
                self.fire_timers().await;
                true
            }
            else => {
                info!(session_id = %self.session_id, "all channels closed, actor exiting");
                false
            }
        }
    }

    async fn handle_pty_read_result(&mut self, result: Option<Vec<u8>>, pty_closed: &mut bool) {
        match result {
            Some(raw) => self.handle_pty_output(raw).await,
            None => self.mark_pty_closed(pty_closed),
        }
    }

    fn mark_pty_closed(&mut self, pty_closed: &mut bool) {
        info!(session_id = %self.session_id, "PTY channel closed (process exit)");
        *pty_closed = true;
        let prev = self.state_detector.state();
        self.state_detector.mark_exited();
        let _ =
            self.maybe_emit_state_change_with_exit_reason(prev, Some("process_exit".to_string()));
    }

    async fn handle_command(&mut self, cmd: SessionCommand, pty_closed: bool) -> bool {
        match cmd {
            SessionCommand::WriteInput(data) => self.handle_write_input(data, pty_closed),
            SessionCommand::Resize { cols, rows } => self.handle_resize(cols, rows),
            SessionCommand::DismissAttention => self.handle_dismiss_attention().await,
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
            SessionCommand::Unsubscribe { client_id } => self.handle_unsubscribe(client_id),
            SessionCommand::GetSnapshot(reply) => {
                let snap = self.build_snapshot().await;
                let _ = reply.send(snap);
            }
            SessionCommand::GetPaneTail { lines, reply } => {
                let text = capture_pane_tail_or_empty(
                    self.session_id.clone(),
                    self.tmux_name.clone(),
                    lines,
                )
                .await;
                let _ = reply.send(text);
            }
            SessionCommand::GetSummary(reply) => {
                let _ = reply.send(self.build_summary());
            }
            SessionCommand::GetMermaidArtifact(reply) => {
                let artifact = Self::build_mermaid_artifact(
                    self.session_id.clone(),
                    self.tmux_name.clone(),
                    self.cwd.clone(),
                    self.session_started_at,
                )
                .await;
                let _ = reply.send(artifact);
            }
            SessionCommand::GetReplayCursor(reply) => {
                let _ = reply.send(self.replay_cursor());
            }
            SessionCommand::Shutdown => {
                info!(session_id = %self.session_id, "shutdown requested, detaching");
                return false;
            }
        }
        true
    }

    fn handle_write_input(&mut self, data: Vec<u8>, pty_closed: bool) {
        if pty_closed {
            debug!(session_id = %self.session_id, "ignoring write to exited PTY");
            return;
        }

        if write_input_counts_as_activity(&data) {
            self.scroll_guard.notify_input();
            let state_before = self.state_detector.state();
            self.state_detector.note_input();
            let _ = self.maybe_emit_state_change(state_before);
        }
        self.update_last_skill_from_input(&data);
        if let Err(e) = write_and_flush_input(&mut self.writer, &data) {
            error!(session_id = %self.session_id, "PTY write error: {}", e);
        }
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
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

    async fn handle_dismiss_attention(&mut self) {
        let state_before = self.state_detector.state();
        self.state_detector.dismiss_attention();
        if matches!(
            self.maybe_emit_state_change(state_before),
            Some(SessionState::Idle)
        ) {
            self.maybe_refresh_cwd_from_tmux(false).await;
        }
    }

    fn handle_unsubscribe(&mut self, client_id: ClientId) {
        self.subscribers.remove(&client_id);
        debug!(session_id = %self.session_id, client_id, "client unsubscribed");
    }
}

async fn capture_pane_tail_or_empty(session_id: String, tmux_name: String, lines: usize) -> String {
    match capture_pane_tail(&tmux_name, lines).await {
        Ok(text) => text,
        Err(e) => {
            debug!(
                session_id = %session_id,
                tmux_name = %tmux_name,
                "tmux capture-pane failed: {}",
                e
            );
            String::new()
        }
    }
}

async fn replay_existing_frames(
    session_id: String,
    client_id: ClientId,
    client_tx: &mpsc::Sender<OutputFrame>,
    replay_plan: ReplayPlan,
) -> SubscribeOutcome {
    match replay_plan {
        ReplayPlan::None => SubscribeOutcome::Ok,
        ReplayPlan::Frames(frames) => {
            for (seq, data) in frames {
                if client_tx.send(OutputFrame { seq, data }).await.is_err() {
                    warn!(
                        session_id = %session_id,
                        client_id,
                        "subscriber dropped during replay"
                    );
                    return SubscribeOutcome::Ok;
                }
            }
            SubscribeOutcome::Ok
        }
        ReplayPlan::Truncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        } => {
            warn!(
                session_id = %session_id,
                client_id,
                requested_resume_from_seq,
                window_start = replay_window_start_seq,
                "replay truncated, client needs full refresh"
            );
            SubscribeOutcome::ReplayTruncated {
                requested_resume_from_seq,
                replay_window_start_seq,
                latest_seq,
            }
        }
    }
}

impl SessionActor {
    fn replay_cursor(&self) -> ReplayCursor {
        ReplayCursor {
            latest_seq: self.replay_ring.latest_seq(),
            replay_window_start_seq: self.replay_ring.window_start_seq(),
        }
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
            self.state_detector.process_output(&flushed.data);
            if matches!(
                self.maybe_emit_state_change(state_before),
                Some(SessionState::Idle)
            ) {
                self.maybe_refresh_cwd_from_tmux(false).await;
            }
            self.record_meaningful_output_activity(state_before, &flushed);

            let seq = self.replay_ring.push(&flushed.data);
            let frame = OutputFrame {
                seq,
                data: flushed.data,
            };
            self.broadcast(frame).await;
        }
    }

    /// Process raw PTY output through the pipeline:
    /// ScrollGuard -> StateDetector -> ReplayRing -> broadcast.
    ///
    /// ScrollGuard returns zero or more chunks (it may buffer for coalescing,
    /// flush a previous buffer alongside new data, or pass through directly).
    async fn handle_pty_output(&mut self, raw: Vec<u8>) {
        self.detect_and_emit_title(&raw);
        for chunk in self.scroll_guard.process(&raw) {
            self.process_output_chunk(chunk).await;
        }
    }

    async fn process_output_chunk(&mut self, chunk: ScrollOutputChunk) {
        let state_before = self.state_detector.state();
        self.state_detector.process_output(&chunk.data);
        self.maybe_update_tool_from_current_command();
        if matches!(
            self.maybe_emit_state_change(state_before),
            Some(SessionState::Idle)
        ) {
            self.maybe_refresh_cwd_from_tmux(false).await;
        }
        self.clear_startup_replay_if_idle();
        self.record_meaningful_output_activity(state_before, &chunk);
        let seq = self.replay_ring.push(&chunk.data);
        self.broadcast(OutputFrame {
            seq,
            data: chunk.data,
        })
        .await;
        crate::metrics::record_queue_depth(&self.session_id, self.total_subscriber_queue_depth());
    }

    fn clear_startup_replay_if_idle(&mut self) {
        if self.clear_replay_on_first_idle && self.state_detector.state() == SessionState::Idle {
            self.clear_replay_on_first_idle = false;
            self.replay_ring.clear();
            debug!(
                session_id = %self.session_id,
                "cleared replay ring on first idle (startup garbage removed)"
            );
        }
    }

    fn total_subscriber_queue_depth(&self) -> usize {
        self.subscribers
            .values()
            .map(|tx| tx.max_capacity() - tx.capacity())
            .sum()
    }

    fn record_meaningful_output_activity(
        &mut self,
        previous_state: SessionState,
        chunk: &ScrollOutputChunk,
    ) {
        let current_state = self.state_detector.state();
        if output_counts_as_meaningful_activity(previous_state, current_state, chunk) {
            self.last_activity_at = Utc::now();
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
        self.apply_osc7_payloads(&text);
        self.apply_title_payloads(&text);
    }

    fn apply_osc7_payloads(&mut self, text: &str) {
        for uri in osc_payloads(text, "\x1b]7;") {
            if let Some(cwd) = cwd_from_osc7_payload(uri) {
                self.update_cwd_and_emit(cwd);
            }
        }
    }

    fn apply_title_payloads(&mut self, text: &str) {
        for title in osc_payloads(text, "\x1b]0;")
            .into_iter()
            .chain(osc_payloads(text, "\x1b]2;"))
        {
            self.apply_title_payload(title);
        }
    }

    fn apply_title_payload(&mut self, title: &str) {
        if title.is_empty() {
            return;
        }
        self.update_cwd_from_title(title);
        self.update_tool_from_title(title);
        self.emit_title_event(title);
    }

    async fn maybe_refresh_cwd_from_tmux(&mut self, force: bool) {
        if !should_refresh_cwd_from_tmux(
            force,
            self.state_detector.state(),
            self.last_cwd_refresh_at,
            Instant::now(),
        ) {
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

    fn maybe_update_tool_from_current_command(&mut self) {
        let current = match self.state_detector.current_command() {
            Some(cmd) => cmd,
            None => return,
        };

        if let Some(tool) = detect_tool_from_command_line(&current) {
            if self.tool.as_deref() != Some(tool) {
                self.tool = Some(tool.to_string());
                self.state_detector.set_tui_tool_mode(true);
            }
        }
    }

    async fn maybe_refresh_tool_from_tmux(&mut self, force: bool) {
        if !should_refresh_tool_from_tmux(
            force,
            self.state_detector.state(),
            self.tool.as_deref(),
            self.last_tool_refresh_at,
            Instant::now(),
        ) {
            return;
        }

        self.last_tool_refresh_at = Instant::now();

        let tmux_name = self.tmux_name.clone();
        match query_tool_from_tmux_process_tree(&tmux_name).await {
            Ok(Some(tool)) => {
                if self.tool.as_deref() != Some(tool.as_str()) {
                    self.tool = Some(tool);
                    self.state_detector.set_tui_tool_mode(true);
                }
            }
            Ok(None) => {}
            Err(e) => {
                debug!(
                    session_id = %self.session_id,
                    tmux_name = %tmux_name,
                    "tmux tool refresh failed: {}",
                    e
                );
            }
        }
    }

    async fn maybe_refresh_session_started_at(&mut self) {
        match query_tmux_session_created(&self.tmux_name).await {
            Ok(session_started_at) => {
                self.session_started_at = session_started_at;
            }
            Err(err) => {
                debug!(
                    session_id = %self.session_id,
                    tmux_name = %self.tmux_name,
                    "tmux session_created refresh failed: {}",
                    err
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

    fn update_cwd_from_title(&mut self, title: &str) {
        if self.cwd.is_empty() {
            if let Some(extracted) = extract_cwd_from_title(title) {
                self.cwd = extracted;
            }
        }
    }

    fn update_tool_from_title(&mut self, title: &str) {
        if self.tool.is_none() {
            self.tool = detect_tool_from_title(title);
            if self.tool.is_some() {
                self.state_detector.set_tui_tool_mode(true);
            }
        }
    }

    fn emit_title_event(&self, title: &str) {
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
            debug!(
                session_id = %self.session_id,
                previous_state = ?payload.previous_state,
                state = ?payload.state,
                current_command = ?payload.current_command,
                transport_health = ?payload.transport_health,
                exit_reason = ?payload.exit_reason,
                at = %payload.at,
                "emitting session_state"
            );
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

        let outcome = replay_existing_frames(
            self.session_id.clone(),
            client_id,
            &client_tx,
            self.replay_plan(resume_from_seq),
        )
        .await;
        self.subscribers.insert(client_id, client_tx);
        outcome
    }

    fn replay_plan(&self, resume_from_seq: Option<u64>) -> ReplayPlan {
        let Some(from_seq) = resume_from_seq else {
            return ReplayPlan::None;
        };

        let Some(frames) = self.replay_ring.replay_from(from_seq.saturating_add(1)) else {
            return ReplayPlan::Truncated {
                requested_resume_from_seq: from_seq,
                replay_window_start_seq: self.replay_ring.window_start_seq(),
                latest_seq: self.replay_ring.latest_seq(),
            };
        };

        ReplayPlan::Frames(frames)
    }

    /// Build a terminal snapshot using tmux capture-pane, falling back to the
    /// replay ring if the tmux command fails.
    async fn build_snapshot(&mut self) -> TerminalSnapshot {
        // Extract values before await to avoid holding &self across the await point
        // (SessionActor contains non-Sync fields like dyn MasterPty).
        let tmux_name = self.tmux_name.clone();
        let session_id = self.session_id.clone();
        let fallback_text = self.replay_ring.snapshot();
        let latest_seq = self.replay_ring.latest_seq();

        let screen_text = match capture_pane_tail(&tmux_name, 300).await {
            Ok(text) => text,
            Err(e) => {
                warn!(
                    session_id = %session_id,
                    tmux_name = %tmux_name,
                    "capture-pane failed for snapshot, falling back to replay ring: {}",
                    e
                );
                fallback_text
            }
        };
        TerminalSnapshot {
            session_id,
            latest_seq,
            truncated: false,
            screen_text,
        }
    }

    fn update_last_skill_from_input(&mut self, data: &[u8]) {
        for line in drain_completed_input_lines(&mut self.input_line_buffer, data) {
            self.process_completed_input_line(&line);
        }
    }

    fn process_completed_input_line(&mut self, line: &str) {
        let Some(detected_skill) = detect_skill_from_input_line(&line) else {
            return;
        };

        if self.last_skill.as_deref() == Some(detected_skill.as_str()) {
            return;
        }

        self.last_skill = Some(detected_skill.clone());

        let event = ControlEvent {
            event: "session_skill".to_string(),
            session_id: self.session_id.clone(),
            payload: serde_json::to_value(SessionSkillPayload {
                last_skill: Some(detected_skill),
                at: Utc::now(),
            })
            .unwrap_or_default(),
        };

        let _ = self.event_tx.send(event);
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
            thought_state: crate::types::ThoughtState::Holding,
            thought_source: crate::types::ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: crate::types::fallback_rest_state(
                state,
                crate::types::ThoughtState::Holding,
            ),
            commit_candidate: false,
            objective_changed_at: None,
            last_skill: self.last_skill.clone(),
            last_activity_at: self.last_activity_at,
            repo_theme_id: None,
        }
    }

    async fn build_mermaid_artifact(
        session_id: String,
        tmux_name: String,
        cwd: String,
        session_started_at: chrono::DateTime<Utc>,
    ) -> MermaidArtifactResponse {
        let fallback_session_id = session_id.clone();
        tokio::task::spawn_blocking(move || {
            let context = ArtifactDiscoveryContext {
                session_id: session_id.clone(),
                tmux_name,
                cwd,
                session_started_at,
                pane_tail: String::new(),
            };
            let response_session_id = session_id.clone();
            default_artifact_registry()
                .discover(ArtifactKind::Mermaid, &context)
                .map(|artifact| MermaidArtifactResponse {
                    session_id: response_session_id.clone(),
                    available: true,
                    path: Some(artifact.path),
                    updated_at: Some(artifact.updated_at),
                    source: artifact.source,
                    error: artifact.error,
                })
                .unwrap_or(MermaidArtifactResponse {
                    session_id: response_session_id,
                    available: false,
                    path: None,
                    updated_at: None,
                    source: None,
                    error: None,
                })
        })
        .await
        .unwrap_or_else(|err| MermaidArtifactResponse {
            session_id: fallback_session_id,
            available: false,
            path: None,
            updated_at: None,
            source: None,
            error: Some(format!("artifact scan task failed: {err}")),
        })
    }
}

/// Capture visible pane text directly from tmux.
async fn capture_pane_tail(tmux_name: &str, lines: usize) -> anyhow::Result<String> {
    let lines = lines.clamp(20, 1000);
    let start = format!("-{lines}");
    let target = exact_pane_target(tmux_name);

    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-J", "-t", &target, "-S", &start])
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

fn write_input_counts_as_activity(data: &[u8]) -> bool {
    let mut index = 0;
    while index < data.len() {
        if data[index] == 0x1b
            && index + 2 < data.len()
            && data[index + 1] == b'['
            && matches!(data[index + 2], b'I' | b'O')
        {
            index += 3;
            continue;
        }

        return true;
    }

    false
}

fn write_and_flush_input(
    writer: &mut Box<dyn std::io::Write + Send>,
    data: &[u8],
) -> io::Result<()> {
    writer.write_all(data)?;
    writer.flush()
}

fn output_counts_as_meaningful_activity(
    previous_state: SessionState,
    current_state: SessionState,
    chunk: &ScrollOutputChunk,
) -> bool {
    if chunk.coalesced_redraw {
        return false;
    }

    if previous_state != SessionState::Idle && current_state == SessionState::Idle {
        return true;
    }

    visible_output_is_meaningful(&chunk.data)
}

fn should_refresh_cwd_from_tmux(
    force: bool,
    state: SessionState,
    last_refresh_at: Instant,
    now: Instant,
) -> bool {
    force
        || (state == SessionState::Idle
            && now.duration_since(last_refresh_at) >= CWD_REFRESH_MIN_INTERVAL)
}

fn should_refresh_tool_from_tmux(
    force: bool,
    state: SessionState,
    tool: Option<&str>,
    last_refresh_at: Instant,
    now: Instant,
) -> bool {
    if force {
        return true;
    }

    if now.duration_since(last_refresh_at) < TOOL_REFRESH_MIN_INTERVAL {
        return false;
    }

    !(tool.is_some() && state == SessionState::Idle)
}

fn visible_output_is_meaningful(data: &[u8]) -> bool {
    let visible = StateDetector::strip_ansi(&String::from_utf8_lossy(data));

    visible
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .any(|line| {
            if line_looks_prompt_like(line) {
                return false;
            }

            let non_whitespace = line.chars().filter(|c| !c.is_whitespace()).count();
            non_whitespace >= 3 && line.chars().any(|c| c.is_alphanumeric())
        })
}

fn line_looks_prompt_like(line: &str) -> bool {
    let line = line.trim_end();
    let mut chars = line.chars();
    let Some(marker @ ('$' | '%' | '#' | '>')) = chars.next_back() else {
        return false;
    };
    let prefix = chars.as_str().trim_end();
    if prefix.is_empty() {
        return true;
    }

    if prefix.contains('@')
        || prefix.contains(':')
        || prefix.contains('/')
        || prefix.contains('~')
        || prefix.contains('\\')
        || prefix.ends_with(')')
        || prefix.ends_with(']')
    {
        if marker == '%' {
            let compact = prefix.replace(',', "");
            if compact
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_whitespace())
            {
                return false;
            }
        }
        return true;
    }

    if prefix.len() > 32 || prefix.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    if prefix
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return false;
    }
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return false;
    }

    matches!(marker, '$' | '#' | '%')
}

/// Query tmux for the active pane cwd of a session.
async fn query_tmux_cwd(tmux_name: &str) -> anyhow::Result<String> {
    let target = exact_pane_target(tmux_name);
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            &target,
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

async fn query_tmux_session_created(tmux_name: &str) -> anyhow::Result<chrono::DateTime<Utc>> {
    let target = exact_pane_target(tmux_name);
    let output = Command::new("tmux")
        .args(["display-message", "-p", "-t", &target, "#{session_created}"])
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

    let epoch = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("invalid tmux session_created value: {}", e))?;
    Utc.timestamp_opt(epoch, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("tmux returned invalid session_created timestamp"))
}

#[derive(Debug, Clone)]
struct ProcessEntry {
    pid: u32,
    ppid: u32,
    comm: String,
    args: String,
}

async fn query_tool_from_tmux_process_tree(tmux_name: &str) -> anyhow::Result<Option<String>> {
    if let Ok(comm) = query_tmux_current_command(tmux_name).await {
        if let Some(tool) = crate::types::detect_tool_name(&comm) {
            return Ok(Some(tool.to_string()));
        }
    }

    let pane_pid = query_tmux_pane_pid(tmux_name).await?;
    let entries = query_process_entries().await?;

    let mut by_pid: HashMap<u32, ProcessEntry> = HashMap::new();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();

    for entry in entries {
        children.entry(entry.ppid).or_default().push(entry.pid);
        by_pid.insert(entry.pid, entry);
    }

    let mut queue = VecDeque::from([pane_pid]);
    let mut visited: HashSet<u32> = HashSet::new();

    while let Some(pid) = queue.pop_front() {
        if !visited.insert(pid) {
            continue;
        }

        if let Some(entry) = by_pid.get(&pid) {
            if let Some(tool) = detect_tool_from_process_entry(entry) {
                return Ok(Some(tool.to_string()));
            }
        }

        if let Some(child_pids) = children.get(&pid) {
            for child_pid in child_pids {
                queue.push_back(*child_pid);
            }
        }
    }

    Ok(None)
}

async fn query_tmux_current_command(tmux_name: &str) -> anyhow::Result<String> {
    let target = exact_pane_target(tmux_name);
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pane_current_command}",
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

    let comm = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if comm.is_empty() {
        return Err(anyhow::anyhow!("tmux returned empty pane_current_command"));
    }
    Ok(comm)
}

async fn query_tmux_pane_pid(tmux_name: &str) -> anyhow::Result<u32> {
    let target = exact_pane_target(tmux_name);
    let output = Command::new("tmux")
        .args(["display-message", "-p", "-t", &target, "#{pane_pid}"])
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

    let pane_pid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|e| anyhow::anyhow!("invalid pane_pid from tmux: {}", e))?;

    Ok(pane_pid)
}

async fn query_process_entries() -> anyhow::Result<Vec<ProcessEntry>> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,comm=,args="])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run ps: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ps failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in stdout.lines() {
        if let Some(entry) = parse_process_entry(line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn parse_process_entry(line: &str) -> Option<ProcessEntry> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let ppid = parts.next()?.parse::<u32>().ok()?;
    let comm = parts.next()?.to_string();
    let args = parts.collect::<Vec<&str>>().join(" ");

    Some(ProcessEntry {
        pid,
        ppid,
        comm,
        args,
    })
}

fn detect_tool_from_process_entry(entry: &ProcessEntry) -> Option<&'static str> {
    crate::types::detect_tool_name(&entry.comm)
        .or_else(|| detect_tool_from_command_line(&entry.args))
}

fn detect_tool_from_command_line(command: &str) -> Option<&'static str> {
    for token in command.split_whitespace() {
        if let Some(tool) = crate::types::detect_tool_name(token) {
            return Some(tool);
        }
    }
    None
}

fn osc_payloads<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
    let mut payloads = Vec::new();
    let mut search_from = 0;

    while let Some(start) = text[search_from..].find(prefix) {
        let payload_start = search_from + start + prefix.len();
        let Some((end_offset, terminator_len)) = find_osc_payload_end(&text[payload_start..])
        else {
            break;
        };
        payloads.push(&text[payload_start..payload_start + end_offset]);
        search_from = payload_start + end_offset + terminator_len;
    }

    payloads
}

fn find_osc_payload_end(text: &str) -> Option<(usize, usize)> {
    let bel = text.find('\x07').map(|offset| (offset, 1));
    let st = text.find("\x1b\\").map(|offset| (offset, 2));
    match (bel, st) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(end), None) | (None, Some(end)) => Some(end),
        (None, None) => None,
    }
}

fn cwd_from_osc7_payload(payload: &str) -> Option<String> {
    let path = payload.strip_prefix("file://")?;
    let path = if let Some(slash_pos) = path.find('/') {
        &path[slash_pos..]
    } else {
        path
    };
    Some(percent_decode(path))
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

fn resolve_tmux_terminal_env(
    inherited_term: Option<&str>,
    inherited_colorterm: Option<&str>,
) -> (String, String, bool) {
    let term = inherited_term.map(str::trim).unwrap_or_default();
    let needs_term_fallback = term.is_empty()
        || term.eq_ignore_ascii_case("dumb")
        || term.eq_ignore_ascii_case("unknown");
    let resolved_term = if needs_term_fallback {
        TMUX_FALLBACK_TERM.to_string()
    } else {
        term.to_string()
    };

    let colorterm = inherited_colorterm
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(TMUX_FALLBACK_COLORTERM)
        .to_string();

    (resolved_term, colorterm, needs_term_fallback)
}

fn detect_skill_from_input_line(line: &str) -> Option<String> {
    extract_skill_from_xml_block(line)
        .or_else(|| extract_skill_from_dollar_token(line))
        .or_else(|| extract_skill_from_slash_token(line))
        .or_else(|| extract_skill_from_using_marker(line))
}

fn drain_completed_input_lines(buffer: &mut String, data: &[u8]) -> Vec<String> {
    let mut completed = Vec::new();
    if data.is_empty() {
        return completed;
    }

    let text = String::from_utf8_lossy(data);
    for ch in text.chars() {
        match ch {
            '\r' | '\n' => {
                let line = buffer.trim().to_string();
                buffer.clear();
                if !line.is_empty() {
                    completed.push(line);
                }
            }
            // Ctrl+C/Ctrl+D should discard any partially typed command line.
            '\u{3}' | '\u{4}' => {
                buffer.clear();
            }
            '\u{8}' | '\u{7f}' => {
                buffer.pop();
            }
            _ if ch.is_control() => {}
            _ => {
                buffer.push(ch);
                if buffer.len() > 8_192 {
                    buffer.clear();
                }
            }
        }
    }

    completed
}

fn extract_skill_from_xml_block(text: &str) -> Option<String> {
    static SKILL_XML_RE: OnceLock<Regex> = OnceLock::new();
    let re = SKILL_XML_RE.get_or_init(|| {
        Regex::new(
            r"(?is)<skill\b[^>]*>.*?<name>\s*([A-Za-z][A-Za-z0-9._/-]{0,63})\s*</name>.*?</skill>",
        )
        .expect("valid skill xml regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter_map(normalize_skill_name)
        .last()
}

fn extract_skill_from_dollar_token(text: &str) -> Option<String> {
    static DOLLAR_SKILL_RE: OnceLock<Regex> = OnceLock::new();
    let re = DOLLAR_SKILL_RE.get_or_init(|| {
        Regex::new(r"\$([A-Za-z][A-Za-z0-9_-]{0,63})").expect("valid dollar skill regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter(|value| is_probable_skill_name(value))
        .filter_map(normalize_skill_name)
        .last()
}

fn extract_skill_from_slash_token(text: &str) -> Option<String> {
    static SLASH_SKILL_RE: OnceLock<Regex> = OnceLock::new();
    let re = SLASH_SKILL_RE.get_or_init(|| {
        Regex::new(r#"^\s*/([A-Za-z][A-Za-z0-9._-]{0,63})(?:\s|$)"#)
            .expect("valid slash skill regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter(|value| is_probable_skill_name(value))
        .filter(|value| !is_common_filesystem_root_name(value))
        .filter_map(normalize_skill_name)
        .last()
}

fn extract_skill_from_using_marker(text: &str) -> Option<String> {
    static USING_SKILL_RE: OnceLock<Regex> = OnceLock::new();
    let re = USING_SKILL_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\busing\s+(?:the\s+)?skill\s+[`"']?([A-Za-z][A-Za-z0-9._/-]{0,63})[`"']?(?:\s+skill)?\b"#,
        )
        .expect("valid using skill regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter(|value| is_probable_skill_name(value))
        .filter_map(normalize_skill_name)
        .last()
}

fn normalize_skill_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
    {
        return None;
    }

    Some(trimmed.to_ascii_lowercase())
}

fn is_probable_skill_name(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }

    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if let Some(installed) = installed_skill_names() {
        // Only enforce strict membership when the discovered registry looks
        // complete enough to trust; tiny registries are often partial.
        if installed.len() >= 5 {
            return installed.contains(&normalized);
        }
        if installed.contains(&normalized) {
            return true;
        }
    }

    // Short tokens are most often partial drafts (e.g. $c, $com, $comm).
    // Allow a known short skill name used in this environment.
    if normalized.len() < 5 {
        return normalized == "gog";
    }

    normalized.chars().any(|ch| ch.is_ascii_lowercase()) || normalized.contains('-')
}

fn installed_skill_names() -> Option<&'static HashSet<String>> {
    static INSTALLED_SKILLS: OnceLock<Option<HashSet<String>>> = OnceLock::new();
    INSTALLED_SKILLS
        .get_or_init(load_installed_skill_names)
        .as_ref()
}

fn load_installed_skill_names() -> Option<HashSet<String>> {
    let home = std::env::var("HOME").ok()?;
    let mut names = HashSet::new();

    for rel_root in [".codex/skills", ".claude/skills"] {
        let root = PathBuf::from(&home).join(rel_root);
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let is_skill_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
            if !is_skill_dir {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(normalized) = normalize_skill_name(&name) {
                names.insert(normalized);
            }
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn is_common_filesystem_root_name(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "bin"
            | "dev"
            | "etc"
            | "home"
            | "lib"
            | "lib64"
            | "mnt"
            | "opt"
            | "private"
            | "proc"
            | "sbin"
            | "sys"
            | "tmp"
            | "usr"
            | "users"
            | "var"
            | "volumes"
    )
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

#[cfg(test)]
mod tests {
    use super::{
        capture_pane_tail, cwd_from_osc7_payload, detect_skill_from_input_line,
        detect_tool_from_command_line, detect_tool_from_process_entry, drain_completed_input_lines,
        extract_cwd_from_title, find_osc_payload_end, line_looks_prompt_like, normalize_skill_name,
        osc_payloads, output_counts_as_meaningful_activity, parse_process_entry, percent_decode,
        query_tmux_session_created, query_tool_from_tmux_process_tree, resolve_tmux_terminal_env,
        should_refresh_cwd_from_tmux, should_refresh_tool_from_tmux, visible_output_is_meaningful,
        write_and_flush_input, write_input_counts_as_activity, ControlEvent, ProcessEntry,
        SessionActor, SessionCommand, CWD_REFRESH_MIN_INTERVAL, TOOL_REFRESH_MIN_INTERVAL,
    };
    use crate::config::Config;
    use crate::scroll::guard::ScrollGuard;
    use crate::scroll::guard::ScrollOutputChunk;
    use crate::session::replay_ring::ReplayRing;
    use crate::state::detector::StateDetector;
    use crate::types::SessionState;
    use chrono::{TimeZone, Utc};
    use portable_pty::{native_pty_system, PtySize};
    use std::collections::HashMap;
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::sync::{broadcast, mpsc, oneshot};

    fn test_actor() -> SessionActor {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let writer = pair.master.take_writer().expect("writer");
        let (_cmd_tx, cmd_rx) = mpsc::channel(8);
        let (event_tx, _) = broadcast::channel::<ControlEvent>(8);

        SessionActor {
            session_id: "sess-test".to_string(),
            tmux_name: "demo".to_string(),
            config: Arc::new(Config::default()),
            master: pair.master,
            writer,
            state_detector: StateDetector::new(),
            scroll_guard: ScrollGuard::new(),
            replay_ring: ReplayRing::new(512 * 1024),
            subscribers: HashMap::new(),
            cmd_rx,
            event_tx,
            cols: 80,
            rows: 24,
            cwd: "/tmp/project".to_string(),
            last_cwd_refresh_at: Instant::now(),
            last_tool_refresh_at: Instant::now(),
            tool: Some("Codex".to_string()),
            last_skill: None,
            input_line_buffer: String::new(),
            last_activity_at: Utc::now(),
            session_started_at: Utc::now(),
            clear_replay_on_first_idle: false,
        }
    }

    #[test]
    fn detect_tool_from_command_line_handles_aliases() {
        assert_eq!(
            detect_tool_from_command_line("FOO=1 /usr/local/bin/claude-code --print"),
            Some("Claude Code")
        );
        assert_eq!(
            detect_tool_from_command_line("codex-cli --help"),
            Some("Codex")
        );
    }

    #[test]
    fn parse_process_entry_parses_ps_row() {
        let entry =
            parse_process_entry("10715 37039 claude /usr/local/bin/claude --print").unwrap();
        assert_eq!(entry.pid, 10_715);
        assert_eq!(entry.ppid, 37_039);
        assert_eq!(entry.comm, "claude");
        assert_eq!(entry.args, "/usr/local/bin/claude --print");
    }

    #[test]
    fn detect_tool_from_process_entry_checks_comm_then_args() {
        let from_comm = ProcessEntry {
            pid: 1,
            ppid: 0,
            comm: "codex".to_string(),
            args: "codex".to_string(),
        };
        assert_eq!(detect_tool_from_process_entry(&from_comm), Some("Codex"));

        let from_args = ProcessEntry {
            pid: 2,
            ppid: 1,
            comm: "node".to_string(),
            args: "/usr/local/bin/claude --json".to_string(),
        };
        assert_eq!(
            detect_tool_from_process_entry(&from_args),
            Some("Claude Code")
        );
    }

    #[test]
    fn line_looks_prompt_like_handles_common_prompt_shapes() {
        assert!(line_looks_prompt_like("$"));
        assert!(line_looks_prompt_like("user@host:/tmp/project$"));
        assert!(line_looks_prompt_like("~/repo %"));
        assert!(!line_looks_prompt_like("42%"));
        assert!(!line_looks_prompt_like("build finished successfully >"));
        assert!(!line_looks_prompt_like("123,456%"));
    }

    #[test]
    fn extract_cwd_from_title_supports_absolute_home_and_host_prefixed_paths() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", "/Users/tester");

        assert_eq!(
            extract_cwd_from_title("user@host:/tmp/project"),
            Some("/tmp/project".to_string())
        );
        assert_eq!(
            extract_cwd_from_title("user@host: /tmp/other"),
            Some("/tmp/other".to_string())
        );
        assert_eq!(
            extract_cwd_from_title("/var/tmp"),
            Some("/var/tmp".to_string())
        );
        assert_eq!(
            extract_cwd_from_title("~/repo"),
            Some("/Users/tester/repo".to_string())
        );
        assert_eq!(extract_cwd_from_title("plain-title"), None);

        if let Some(value) = previous_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn percent_decode_decodes_hex_sequences_and_keeps_invalid_ones() {
        assert_eq!(percent_decode("/tmp/My%20Repo"), "/tmp/My Repo");
        assert_eq!(percent_decode("%ZZ/path"), "%/path");
    }

    #[test]
    fn normalize_skill_name_rejects_blank_and_invalid_values() {
        assert_eq!(normalize_skill_name("  "), None);
        assert_eq!(normalize_skill_name("bad!skill"), None);
        assert_eq!(normalize_skill_name(" Commit "), Some("commit".to_string()));
    }

    #[test]
    fn osc_payload_helpers_extract_bel_and_st_terminated_sequences() {
        let text = "\x1b]7;file://host/tmp/project\x1b\\ middle \x1b]2;codex\x07";
        assert_eq!(find_osc_payload_end("title\x07tail"), Some((5, 1)));
        assert_eq!(find_osc_payload_end("title\x1b\\tail"), Some((5, 2)));
        assert_eq!(
            osc_payloads(text, "\x1b]7;"),
            vec!["file://host/tmp/project"]
        );
        assert_eq!(osc_payloads(text, "\x1b]2;"), vec!["codex"]);
        assert_eq!(
            cwd_from_osc7_payload("file://host/tmp/My%20Repo"),
            Some("/tmp/My Repo".to_string())
        );
    }

    #[test]
    fn refresh_predicates_only_poll_when_needed() {
        let now = Instant::now();
        assert!(should_refresh_cwd_from_tmux(
            true,
            SessionState::Busy,
            now,
            now
        ));
        assert!(!should_refresh_cwd_from_tmux(
            false,
            SessionState::Busy,
            now - CWD_REFRESH_MIN_INTERVAL,
            now
        ));
        assert!(should_refresh_cwd_from_tmux(
            false,
            SessionState::Idle,
            now - CWD_REFRESH_MIN_INTERVAL,
            now
        ));

        assert!(should_refresh_tool_from_tmux(
            true,
            SessionState::Idle,
            Some("Codex"),
            now,
            now
        ));
        assert!(!should_refresh_tool_from_tmux(
            false,
            SessionState::Busy,
            None,
            now,
            now
        ));
        assert!(!should_refresh_tool_from_tmux(
            false,
            SessionState::Idle,
            Some("Codex"),
            now - TOOL_REFRESH_MIN_INTERVAL,
            now
        ));
        assert!(should_refresh_tool_from_tmux(
            false,
            SessionState::Busy,
            Some("Codex"),
            now - TOOL_REFRESH_MIN_INTERVAL,
            now
        ));
    }

    #[tokio::test]
    async fn query_tool_from_tmux_process_tree_uses_current_command_fast_path() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux = bin_dir.join("tmux");
        std::fs::write(
            &tmux,
            "#!/bin/sh\nif [ \"${5-}\" = \"#{pane_current_command}\" ]; then\n  printf 'codex\\n'\nelse\n  printf '101\\n'\nfi\n",
        )
        .expect("tmux");
        let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmux, perms).expect("chmod");

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let tool = query_tool_from_tmux_process_tree("demo")
            .await
            .expect("tool query");
        assert_eq!(tool.as_deref(), Some("Codex"));

        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[tokio::test]
    async fn query_tool_from_tmux_process_tree_walks_process_children_when_needed() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        let tmux = bin_dir.join("tmux");
        std::fs::write(
            &tmux,
            r##"#!/bin/sh
if [ "${5-}" = "#{pane_current_command}" ]; then
  printf 'bash\n'
else
  printf '101\n'
fi
"##,
        )
        .expect("tmux");
        let ps = bin_dir.join("ps");
        std::fs::write(
            &ps,
            "#!/bin/sh\nprintf '101 1 bash bash\\n102 101 node /usr/local/bin/claude --print\\n'\n",
        )
        .expect("ps");
        for path in [&tmux, &ps] {
            let mut perms = std::fs::metadata(path).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).expect("chmod");
        }

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let tool = query_tool_from_tmux_process_tree("demo")
            .await
            .expect("tool query");
        assert_eq!(tool.as_deref(), Some("Claude Code"));

        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[tokio::test]
    async fn get_summary_uses_cached_metadata_without_tmux_refresh() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        let tmux = bin_dir.join("tmux");
        std::fs::write(&tmux, "#!/bin/sh\nsleep 2\nprintf 'codex\\n'\n").expect("tmux");
        let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmux, perms).expect("chmod");

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let mut actor = test_actor();
        actor
            .state_detector
            .process_output(b"running build output\n");
        actor.last_tool_refresh_at = Instant::now() - TOOL_REFRESH_MIN_INTERVAL;

        let (tx, rx) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_millis(200),
            actor.handle_command(SessionCommand::GetSummary(tx), false),
        )
        .await
        .expect("GetSummary should not block on tmux refresh");

        let summary = tokio::time::timeout(Duration::from_millis(200), rx)
            .await
            .expect("summary reply")
            .expect("summary payload");
        assert_eq!(summary.tool.as_deref(), Some("Codex"));
        assert_eq!(summary.cwd, "/tmp/project");

        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn detect_skill_prefers_explicit_skill_block() {
        let line = r#"send <skill><name>describe</name></skill> and $fallback"#;
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("describe".to_string())
        );
    }

    #[test]
    fn detect_skill_falls_back_to_dollar_token() {
        let line = "please run $domain-planner for this slice";
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("domain-planner".to_string())
        );
    }

    #[test]
    fn detect_skill_records_full_commit_name() {
        let line = "$commit";
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("commit".to_string())
        );
    }

    #[test]
    fn detect_skill_ignores_short_partial_dollar_tokens() {
        assert_eq!(detect_skill_from_input_line("$c"), None);
        assert_eq!(detect_skill_from_input_line("$com"), None);
        assert_eq!(detect_skill_from_input_line("$comm"), None);
    }

    #[derive(Default)]
    struct TrackingWriterState {
        writes: Vec<u8>,
        flushes: usize,
    }

    struct TrackingWriter {
        state: Arc<Mutex<TrackingWriterState>>,
    }

    impl Write for TrackingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            state.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            state.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn write_and_flush_input_flushes_pty_writer() {
        let state = Arc::new(Mutex::new(TrackingWriterState::default()));
        let mut writer: Box<dyn Write + Send> = Box::new(TrackingWriter {
            state: Arc::clone(&state),
        });

        write_and_flush_input(&mut writer, b"echo hi\r").expect("write and flush");

        let state = state.lock().unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(state.writes, b"echo hi\r");
        assert_eq!(state.flushes, 1);
    }

    #[test]
    fn detect_skill_falls_back_to_slash_token() {
        let line = "/describe";
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("describe".to_string())
        );
    }

    #[test]
    fn detect_skill_ignores_common_root_path_slash_token() {
        let line = "/tmp";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn detect_skill_ignores_common_shell_env_vars() {
        let line = "echo $HOME && echo $PATH";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn detect_skill_ignores_unknown_dollar_token() {
        let line = "please run $notarealskillzzzzz";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn detect_skill_ignores_generic_using_phrase_without_skill_keyword() {
        let line = "using decision heuristics for this pass";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn completed_lines_drop_partial_skill_on_ctrl_c_carriage_return() {
        let mut buffer = String::new();
        assert!(drain_completed_input_lines(&mut buffer, b"$c").is_empty());
        assert_eq!(buffer, "$c");

        let lines = drain_completed_input_lines(&mut buffer, b"\x03\r");
        assert!(lines.is_empty());
        assert!(buffer.is_empty());
    }

    #[test]
    fn completed_lines_emit_full_skill_after_chunked_input() {
        let mut buffer = String::new();
        assert!(drain_completed_input_lines(&mut buffer, b"$com").is_empty());
        let lines = drain_completed_input_lines(&mut buffer, b"mit\r");
        assert_eq!(lines, vec!["$commit".to_string()]);
    }

    #[test]
    fn resolve_tmux_terminal_env_uses_fallback_for_missing_or_dumb_term() {
        let (term, colorterm, fallback) = resolve_tmux_terminal_env(None, None);
        assert_eq!(term, "xterm-256color");
        assert_eq!(colorterm, "truecolor");
        assert!(fallback);

        let (term, colorterm, fallback) =
            resolve_tmux_terminal_env(Some("  dumb  "), Some(" 24bit "));
        assert_eq!(term, "xterm-256color");
        assert_eq!(colorterm, "24bit");
        assert!(fallback);
    }

    #[test]
    fn resolve_tmux_terminal_env_preserves_valid_term() {
        let (term, colorterm, fallback) =
            resolve_tmux_terminal_env(Some("screen-256color"), Some("truecolor"));
        assert_eq!(term, "screen-256color");
        assert_eq!(colorterm, "truecolor");
        assert!(!fallback);
    }

    #[test]
    fn replay_ring_snapshot_preserves_recent_output() {
        let mut ring = ReplayRing::new(512 * 1024);
        ring.push(b"$ hello world\n");
        ring.push(b"output line 2\n");
        let snapshot_text = ring.snapshot();
        assert_eq!(snapshot_text, "$ hello world\noutput line 2\n");
        assert!(ring.latest_seq() > 0);
    }

    #[test]
    fn visible_output_ignores_prompt_only_lines() {
        assert!(!visible_output_is_meaningful(b"b@host swimmers % "));
        assert!(!visible_output_is_meaningful(b"$ "));
    }

    #[test]
    fn visible_output_detects_substantive_terminal_text() {
        assert!(visible_output_is_meaningful(
            b"checking auth middleware header parsing\n"
        ));
        assert!(visible_output_is_meaningful(
            b"test auth::login ... FAILED\n"
        ));
    }

    #[tokio::test]
    async fn query_tmux_session_created_reads_epoch_from_tmux() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux = bin_dir.join("tmux");
        std::fs::write(&tmux, "#!/bin/sh\nprintf '1774274168\\n'\n").expect("tmux");
        let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmux, perms).expect("chmod");

        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let created_at = query_tmux_session_created("demo")
            .await
            .expect("session_created query");
        assert_eq!(
            created_at,
            Utc.timestamp_opt(1_774_274_168, 0).single().unwrap()
        );

        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[tokio::test]
    async fn capture_pane_tail_uses_exact_session_target_for_numeric_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let target_file = dir.path().join("target.txt");
        let tmux = bin_dir.join("tmux");
        std::fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"${{5-}}\" > \"{}\"\nprintf 'captured\\n'\n",
                target_file.display()
            ),
        )
        .expect("tmux");
        let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmux, perms).expect("chmod");

        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let captured = capture_pane_tail("0", 20).await.expect("capture pane");
        assert_eq!(captured.trim(), "captured");
        assert_eq!(
            std::fs::read_to_string(&target_file).expect("target file"),
            "=0:\n"
        );

        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn coalesced_redraw_does_not_count_as_meaningful_activity() {
        let chunk = ScrollOutputChunk {
            data: b"prompt repaint".to_vec(),
            coalesced_redraw: true,
        };
        assert!(!output_counts_as_meaningful_activity(
            SessionState::Idle,
            SessionState::Idle,
            &chunk,
        ));
    }

    #[test]
    fn prompt_that_finishes_busy_work_counts_as_activity() {
        let chunk = ScrollOutputChunk {
            data: b"b@host swimmers % ".to_vec(),
            coalesced_redraw: false,
        };
        assert!(output_counts_as_meaningful_activity(
            SessionState::Busy,
            SessionState::Idle,
            &chunk,
        ));
    }

    #[test]
    fn standalone_focus_reports_do_not_count_as_activity_input() {
        assert!(!write_input_counts_as_activity(b"\x1b[I"));
        assert!(!write_input_counts_as_activity(b"\x1b[O"));
        assert!(!write_input_counts_as_activity(b"\x1b[I\x1b[O\x1b[I"));
    }

    #[test]
    fn mixed_focus_reports_and_real_input_still_count_as_activity() {
        assert!(write_input_counts_as_activity(b"\x1b[Ia"));
        assert!(write_input_counts_as_activity(b"\x1b[O\r"));
        assert!(write_input_counts_as_activity(b"\t"));
    }
}
