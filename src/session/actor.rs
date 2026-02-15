use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::scroll::guard::ScrollGuard;
use crate::session::replay_ring::ReplayRing;
use crate::state::detector::StateDetector;
use crate::types::{SessionSummary, TerminalSnapshot, TransportHealth};

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
    },

    /// Remove a client subscription.
    Unsubscribe { client_id: ClientId },

    /// Request a terminal text snapshot (reply via oneshot).
    GetSnapshot(oneshot::Sender<TerminalSnapshot>),

    /// Request a session summary (reply via oneshot).
    GetSummary(oneshot::Sender<SessionSummary>),

    /// Graceful shutdown -- detach from tmux, do NOT kill the tmux session.
    Shutdown,
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
}

impl ActorHandle {
    pub async fn send(
        &self,
        cmd: SessionCommand,
    ) -> Result<(), mpsc::error::SendError<SessionCommand>> {
        self.cmd_tx.send(cmd).await
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

    // Cols/rows for summary reporting
    cols: u16,
    rows: u16,
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

        let replay_ring = ReplayRing::new(config.replay_buffer_size);

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
            cols: 80,
            rows: 24,
        };

        // Spawn the actor's run loop on the Tokio runtime.
        tokio::spawn(actor.run());

        let handle = ActorHandle {
            session_id,
            tmux_name,
            cmd_tx,
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

        loop {
            // Compute the next timer deadline from both StateDetector and ScrollGuard.
            let next_timer = self.next_timer_deadline();

            tokio::select! {
                // --- PTY output ---
                Some(raw) = pty_rx.recv() => {
                    self.handle_pty_output(raw).await;
                }

                // --- Inbound commands ---
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        SessionCommand::WriteInput(data) => {
                            self.scroll_guard.notify_input();
                            if let Err(e) = self.writer.write_all(&data) {
                                error!(session_id = %self.session_id, "PTY write error: {}", e);
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
                            self.state_detector.dismiss_attention();
                        }
                        SessionCommand::Subscribe { client_id, client_tx, resume_from_seq } => {
                            self.handle_subscribe(client_id, client_tx, resume_from_seq).await;
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
                        SessionCommand::GetSummary(reply) => {
                            let summary = self.build_summary();
                            let _ = reply.send(summary);
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
        // Check state detector timers (error auto-clear, idle -> attention).
        self.state_detector.check_timers(Instant::now());

        // Flush any coalesced scroll guard data.
        if let Some(flushed) = self.scroll_guard.flush() {
            self.state_detector.process_output(&flushed);
            let seq = self.replay_ring.push(&flushed);
            let frame = OutputFrame {
                seq,
                data: flushed,
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
        let chunks = self.scroll_guard.process(&raw);

        for chunk in chunks {
            // Feed the state detector with each chunk.
            self.state_detector.process_output(&chunk);

            // Store in the replay ring and get the sequence number.
            let seq = self.replay_ring.push(&chunk);

            let frame = OutputFrame { seq, data: chunk };

            // Broadcast to all subscribers.
            self.broadcast(frame).await;
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
    ) {
        info!(
            session_id = %self.session_id,
            client_id,
            resume_from_seq = ?resume_from_seq,
            "client subscribing"
        );

        // If the client wants replay, send buffered frames first.
        if let Some(from_seq) = resume_from_seq {
            match self.replay_ring.replay_from(from_seq) {
                Some(frames) => {
                    for (seq, data) in frames {
                        let frame = OutputFrame { seq, data };
                        if client_tx.send(frame).await.is_err() {
                            warn!(
                                session_id = %self.session_id,
                                client_id,
                                "subscriber dropped during replay"
                            );
                            return;
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
                }
            }
        }

        self.subscribers.insert(client_id, client_tx);
    }

    /// Build a summary snapshot of this session's current state.
    fn build_summary(&self) -> SessionSummary {
        let (state, current_command) = self.state_detector.get_state();
        SessionSummary {
            session_id: self.session_id.clone(),
            tmux_name: self.tmux_name.clone(),
            state,
            current_command,
            cwd: String::new(), // TODO: extract from tmux or OSC 7
            tool: None,         // TODO: detect from process tree
            token_count: 0,
            context_limit: 128_000,
            thought: None,
            is_stale: false,
            attached_clients: self.subscribers.len() as u32,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
        }
    }
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
