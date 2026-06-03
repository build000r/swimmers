use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::process::Output;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::scroll::guard::{ScrollGuard, ScrollOutputChunk};
use crate::session::artifact_responses::{
    build_mermaid_artifact_response, build_plan_file_response_async,
};
use crate::session::replay_ring::ReplayRing;
use crate::session::skill_detection::{detect_skill_from_input_line, drain_completed_input_lines};
use crate::state::detector::StateDetector;
use crate::tmux_target::{exact_pane_target, exact_session_target};
use crate::types::{
    clamp_terminal_resize, ControlEvent, MermaidArtifactResponse, PlanFileResponse,
    SessionBatchMembership, SessionSkillPayload, SessionState, SessionStatePayload, SessionSummary,
    SessionTitlePayload, StateEvidence, TerminalSnapshot, TransportHealth,
};

mod tmux_input;

#[cfg(test)]
use self::tmux_input::TmuxInputChunk;
use self::tmux_input::{
    normalize_submit_line_text, send_tmux_input_chunks, send_tmux_submit_line,
    submit_line_fallback_input, tmux_input_chunks, write_and_flush_input,
    write_input_counts_as_activity,
};

const CWD_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(750);
const TOOL_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(1_000);
const LIVENESS_CHECK_INTERVAL: Duration = Duration::from_millis(2_000);
const TMUX_DISPLAY_MESSAGE_TIMEOUT: Duration = Duration::from_millis(500);
const TMUX_CAPTURE_PANE_TIMEOUT: Duration = Duration::from_secs(1);
const PROCESS_ENTRIES_QUERY_TIMEOUT: Duration = Duration::from_millis(750);
const PROCESS_ENTRIES_CACHE_TTL: Duration = Duration::from_millis(1_500);
const TMUX_NEW_SESSION_EXIT_GRACE: Duration = Duration::from_millis(50);
const MAX_OUTPUT_SUBSCRIBERS_PER_SESSION: usize = 16;
const TMUX_FALLBACK_TERM: &str = "xterm-256color";
const TMUX_FALLBACK_COLORTERM: &str = "truecolor";

fn tmux_command(program: impl AsRef<OsStr>, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .kill_on_drop(true);
    command
}

pub(crate) async fn run_bounded_tmux_command(
    program: impl AsRef<OsStr>,
    args: &[&str],
    timeout_duration: Duration,
    operation: &'static str,
) -> anyhow::Result<Output> {
    let started = Instant::now();
    let mut command = tmux_command(program, args);
    match tokio::time::timeout(timeout_duration, command.output()).await {
        Ok(Ok(output)) => {
            log_bounded_tmux_command_elapsed(
                operation,
                started.elapsed(),
                timeout_duration,
                Some(output.status.success()),
            );
            Ok(output)
        }
        Ok(Err(err)) => {
            let elapsed = started.elapsed();
            debug!(
                operation,
                elapsed_ms = elapsed.as_millis() as u64,
                "tmux command failed to spawn: {}",
                err
            );
            Err(anyhow::anyhow!("failed to run tmux {operation}: {err}"))
        }
        Err(_) => {
            let elapsed = started.elapsed();
            warn!(
                operation,
                elapsed_ms = elapsed.as_millis() as u64,
                timeout_ms = timeout_duration.as_millis() as u64,
                "tmux command timed out"
            );
            Err(anyhow::anyhow!(
                "tmux {operation} timed out after {}ms",
                timeout_duration.as_millis()
            ))
        }
    }
}

fn log_bounded_tmux_command_elapsed(
    operation: &'static str,
    elapsed: Duration,
    timeout_duration: Duration,
    success: Option<bool>,
) {
    let elapsed_ms = elapsed.as_millis() as u64;
    let timeout_ms = timeout_duration.as_millis() as u64;
    if elapsed.as_millis() >= timeout_duration.as_millis().saturating_div(2) {
        warn!(
            operation,
            elapsed_ms, timeout_ms, success, "bounded tmux command completed slowly"
        );
    } else {
        debug!(
            operation,
            elapsed_ms, timeout_ms, success, "bounded tmux command completed"
        );
    }
}

// ---------------------------------------------------------------------------
// Public command enum -- sent to the actor over its mpsc channel
// ---------------------------------------------------------------------------

/// Uniquely identifies a connected client's output subscription.
pub type ClientId = u64;

/// A framed chunk of terminal output with its sequence number.
#[derive(Debug, Clone)]
pub struct OutputFrame {
    /// Monotonic output sequence used by replay-aware WebSocket clients.
    pub seq: u64,
    pub data: Vec<u8>,
}

/// Commands that the rest of the system can send to a session actor.
#[derive(Debug)]
pub enum SessionCommand {
    /// Write raw bytes to the PTY (user input).
    WriteInput(Vec<u8>),

    /// Write raw bytes and acknowledge whether they reached tmux/the PTY.
    WriteInputAck {
        data: Vec<u8>,
        ack: oneshot::Sender<InputDeliveryResult>,
    },

    /// Paste a prompt and submit it to an agent-style terminal prompt.
    SubmitLine(String),

    /// Paste and submit a prompt, acknowledging whether injection succeeded.
    SubmitLineAck {
        text: String,
        ack: oneshot::Sender<InputDeliveryResult>,
    },

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

    /// Read a plan file sibling to the session's schema.mmd.
    GetPlanFile {
        name: String,
        reply: oneshot::Sender<PlanFileResponse>,
    },

    /// Request replay cursor metadata for lifecycle acknowledgments.
    GetReplayCursor(oneshot::Sender<ReplayCursor>),

    /// Graceful shutdown -- detach from tmux, do NOT kill the tmux session.
    Shutdown,
}

/// Subscribe result returned to the websocket layer.
#[derive(Debug)]
pub enum SubscribeOutcome {
    Ok,
    Rejected {
        reason: String,
    },
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

/// Result of a browser/API input delivery attempt after actor-side injection.
#[derive(Debug, Clone)]
pub struct InputDeliveryResult {
    pub delivered: bool,
    pub method: &'static str,
    pub message: Option<String>,
}

impl InputDeliveryResult {
    fn delivered(method: &'static str) -> Self {
        Self {
            delivered: true,
            method,
            message: None,
        }
    }

    fn failed(method: &'static str, message: impl Into<String>) -> Self {
        Self {
            delivered: false,
            method,
            message: Some(message.into()),
        }
    }
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

    #[cfg(any(test, debug_assertions))]
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

    // Last time we ran a process-tree liveness check.
    last_liveness_check_at: Instant,

    // Detected coding tool name
    tool: Option<String>,

    // Most recent detected skill invocation (e.g. "$describe").
    last_skill: Option<String>,

    // Optional batch/mission this session was spawned under.
    batch: Option<SessionBatchMembership>,

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
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        session_id: String,
        tmux_name: String,
        attach: bool,
        start_cwd: Option<String>,
        initial_tool: Option<String>,
        initial_command: Option<String>,
        config: Arc<Config>,
        last_activity_override: Option<chrono::DateTime<Utc>>,
        batch: Option<SessionBatchMembership>,
    ) -> anyhow::Result<ActorHandle> {
        let pty_system = native_pty_system();

        let initial_size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        if !attach {
            if let Some(dir) = start_cwd.as_deref() {
                if !std::path::Path::new(dir).is_dir() {
                    return Err(anyhow::anyhow!(
                        "session cwd does not exist or is not a directory: {dir}"
                    ));
                }
            }
        }

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
            if let Some(command) = initial_command.as_deref() {
                c.arg(command);
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

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("failed to spawn tmux: {}", e))?;
        if !attach {
            std::thread::sleep(TMUX_NEW_SESSION_EXIT_GRACE);
            if let Some(status) = child
                .try_wait()
                .map_err(|e| anyhow::anyhow!("failed to inspect tmux after spawn: {}", e))?
            {
                return Err(anyhow::anyhow!(
                    "tmux new-session exited immediately with status {status}"
                ));
            }
        }

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
        let state_detector = state_detector_for_initial_tool(initial_tool.as_deref());

        let actor = SessionActor {
            session_id: session_id.clone(),
            tmux_name: tmux_name.clone(),
            config,
            master: pair.master,
            writer,
            state_detector,
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
            last_liveness_check_at: Instant::now(),
            tool: initial_tool,
            last_skill: None,
            batch,
            input_line_buffer: String::new(),
            last_activity_at: last_activity_override.unwrap_or_else(Utc::now),
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
            biased;
            Some(cmd) = self.cmd_rx.recv() => self.handle_command(cmd, *pty_closed).await,
            result = pty_rx.recv(), if !*pty_closed => {
                self.handle_pty_read_result(result, pty_closed).await;
                true
            }
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
        let prev_evidence = self.state_detector.state_evidence();
        self.state_detector.mark_exited();
        let _ = self.maybe_emit_state_change_with_exit_reason(
            prev,
            prev_evidence,
            Some("process_exit".to_string()),
        );
    }

    async fn handle_command(&mut self, cmd: SessionCommand, pty_closed: bool) -> bool {
        match cmd {
            SessionCommand::WriteInput(data) => {
                let _ = self.handle_write_input(data, pty_closed).await;
            }
            SessionCommand::WriteInputAck { data, ack } => {
                let _ = ack.send(self.handle_write_input(data, pty_closed).await);
            }
            SessionCommand::SubmitLine(text) => {
                let _ = self.handle_submit_line(text, pty_closed).await;
            }
            SessionCommand::SubmitLineAck { text, ack } => {
                let _ = ack.send(self.handle_submit_line(text, pty_closed).await);
            }
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
                let artifact = build_mermaid_artifact_response(
                    self.session_id.clone(),
                    self.tmux_name.clone(),
                    self.cwd.clone(),
                    self.session_started_at,
                )
                .await;
                let _ = reply.send(artifact);
            }
            SessionCommand::GetPlanFile { name, reply } => {
                let response = build_plan_file_response_async(
                    self.session_id.clone(),
                    self.cwd.clone(),
                    self.session_started_at,
                    name,
                )
                .await;
                let _ = reply.send(response);
            }
            SessionCommand::GetReplayCursor(reply) => {
                let _ = reply.send(self.replay_cursor());
            }
            SessionCommand::Shutdown => {
                info!(session_id = %self.session_id, "shutdown requested, detaching");
                // Drain any coalesced scroll-guard frame so the final visible
                // state isn't dropped between the last process() and exit.
                self.flush_scroll_guard().await;
                return false;
            }
        }
        true
    }

    async fn handle_write_input(&mut self, data: Vec<u8>, pty_closed: bool) -> InputDeliveryResult {
        if pty_closed {
            debug!(session_id = %self.session_id, "ignoring write to exited PTY");
            return InputDeliveryResult::failed("none", "session process has exited");
        }

        if write_input_counts_as_activity(&data) {
            self.scroll_guard.notify_input();
            let state_before = self.state_detector.state();
            let evidence_before = self.state_detector.state_evidence();
            self.state_detector.note_input();
            let _ = self.maybe_emit_state_change(state_before, evidence_before);
        }
        self.update_last_skill_from_input(&data);

        if let Some(chunks) = tmux_input_chunks(&data) {
            match send_tmux_input_chunks(&self.tmux_name, &chunks).await {
                Ok(()) => return InputDeliveryResult::delivered("tmux_send_keys"),
                Err(err) => {
                    warn!(
                        session_id = %self.session_id,
                        tmux_name = %self.tmux_name,
                        delivered_chunks = err.delivered_chunks,
                        "tmux send-keys input failed: {err}"
                    );
                    if err.delivered_chunks > 0 {
                        return InputDeliveryResult::delivered("tmux_send_keys_partial");
                    }
                }
            }
        }

        match write_and_flush_input(&mut self.writer, &data) {
            Ok(()) => InputDeliveryResult::delivered("pty_write"),
            Err(e) => {
                error!(session_id = %self.session_id, "PTY write error: {}", e);
                InputDeliveryResult::failed("pty_write", e.to_string())
            }
        }
    }

    async fn handle_submit_line(&mut self, text: String, pty_closed: bool) -> InputDeliveryResult {
        if pty_closed {
            debug!(session_id = %self.session_id, "ignoring submit to exited PTY");
            return InputDeliveryResult::failed("none", "session process has exited");
        }

        let Some(text) = normalize_submit_line_text(&text) else {
            return InputDeliveryResult::failed("none", "text must not be empty");
        };
        let fallback_input = submit_line_fallback_input(&text);

        if write_input_counts_as_activity(&fallback_input) {
            self.scroll_guard.notify_input();
            let state_before = self.state_detector.state();
            let evidence_before = self.state_detector.state_evidence();
            self.state_detector.note_input();
            let _ = self.maybe_emit_state_change(state_before, evidence_before);
        }
        self.update_last_skill_from_input(&fallback_input);

        match send_tmux_submit_line(&self.tmux_name, &text).await {
            Ok(()) => return InputDeliveryResult::delivered("tmux_submit_line"),
            Err(err) => {
                warn!(
                    session_id = %self.session_id,
                    tmux_name = %self.tmux_name,
                    "tmux submit-line fallback failed: {err}"
                );
            }
        }

        match write_and_flush_input(&mut self.writer, &fallback_input) {
            Ok(()) => InputDeliveryResult::delivered("pty_write"),
            Err(e) => {
                error!(session_id = %self.session_id, "PTY submit write error: {}", e);
                InputDeliveryResult::failed("pty_write", e.to_string())
            }
        }
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = clamp_terminal_resize(cols, rows);
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
        let evidence_before = self.state_detector.state_evidence();
        self.state_detector.dismiss_attention();
        if matches!(
            self.maybe_emit_state_change(state_before, evidence_before),
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

fn state_detector_for_initial_tool(initial_tool: Option<&str>) -> StateDetector {
    let mut detector = StateDetector::new();
    if initial_tool
        .and_then(crate::types::detect_tool_name)
        .is_some()
    {
        detector.set_tui_tool_mode(true);
    }
    detector
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
            replay_buffered_frames(&session_id, client_id, client_tx, frames).await
        }
        ReplayPlan::Truncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        } => replay_truncated_outcome(
            &session_id,
            client_id,
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        ),
    }
}

async fn replay_buffered_frames(
    session_id: &str,
    client_id: ClientId,
    client_tx: &mpsc::Sender<OutputFrame>,
    frames: Vec<(u64, Vec<u8>)>,
) -> SubscribeOutcome {
    if send_replay_frames(client_tx, frames).await.is_none() {
        warn!(
            session_id = %session_id,
            client_id,
            "subscriber dropped during replay"
        );
    }
    SubscribeOutcome::Ok
}

async fn send_replay_frames(
    client_tx: &mpsc::Sender<OutputFrame>,
    frames: Vec<(u64, Vec<u8>)>,
) -> Option<()> {
    for (seq, data) in frames {
        client_tx.send(OutputFrame { seq, data }).await.ok()?;
    }
    Some(())
}

fn replay_truncated_outcome(
    session_id: &str,
    client_id: ClientId,
    requested_resume_from_seq: u64,
    replay_window_start_seq: u64,
    latest_seq: u64,
) -> SubscribeOutcome {
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

    /// Compute the earliest timer deadline across StateDetector, ScrollGuard,
    /// and the periodic liveness check.
    fn next_timer_deadline(&self) -> Option<Instant> {
        let state_deadline = self.state_detector.next_deadline();
        let scroll_deadline = self.scroll_guard.check_flush_deadline();
        let liveness_deadline = if self.state_detector.state() != SessionState::Exited {
            Some(self.last_liveness_check_at + LIVENESS_CHECK_INTERVAL)
        } else {
            None
        };
        [state_deadline, scroll_deadline, liveness_deadline]
            .into_iter()
            .flatten()
            .min()
    }

    /// Fire any expired timers and process the results.
    async fn fire_timers(&mut self) {
        // Snapshot state before timers for change detection.
        let state_before = self.state_detector.state();
        let evidence_before = self.state_detector.state_evidence();

        // Check state detector timers (error auto-clear, idle -> attention).
        self.state_detector.check_timers(Instant::now());

        // Emit state change event if timers caused a transition.
        if matches!(
            self.maybe_emit_state_change(state_before, evidence_before),
            Some(SessionState::Idle)
        ) {
            self.maybe_refresh_cwd_from_tmux(false).await;
        }

        // Flush any coalesced scroll guard data.
        self.flush_scroll_guard().await;

        // Process-tree liveness reconciliation.
        self.maybe_check_liveness().await;
    }

    /// Flush any frame buffered in the ScrollGuard through the canonical
    /// output pipeline. Used by the periodic timer fan-out and at session
    /// shutdown so the last coalesced frame isn't dropped.
    async fn flush_scroll_guard(&mut self) {
        if let Some(flushed) = self.scroll_guard.flush() {
            self.process_output_chunk(flushed).await;
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
        let evidence_before = self.state_detector.state_evidence();
        self.state_detector.process_output(&chunk.data);
        self.maybe_update_tool_from_current_command();
        if matches!(
            self.maybe_emit_state_change(state_before, evidence_before),
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
        let current_command = self.state_detector.current_command();
        let Some(tool) =
            current_command_tool_update(current_command.as_deref(), self.tool.as_deref())
        else {
            return;
        };

        self.tool = Some(tool.to_string());
        self.state_detector.set_tui_tool_mode(true);
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

    /// Periodically query the pane's process tree to reconcile state.
    /// Runs every LIVENESS_CHECK_INTERVAL (~2s). Skips if the session has exited.
    async fn maybe_check_liveness(&mut self) {
        let now = Instant::now();
        if !self.should_check_liveness(now) {
            return;
        }
        self.last_liveness_check_at = now;
        self.query_and_reconcile_liveness().await;
    }

    fn should_check_liveness(&self, now: Instant) -> bool {
        self.state_detector.state() != SessionState::Exited
            && now.duration_since(self.last_liveness_check_at) >= LIVENESS_CHECK_INTERVAL
    }

    async fn query_and_reconcile_liveness(&mut self) {
        let tmux_name = self.tmux_name.clone();
        match query_pane_liveness(&tmux_name).await {
            Ok(liveness) => {
                let outcome = self.reconcile_liveness(liveness);
                if outcome.refresh_cwd {
                    self.maybe_refresh_cwd_from_tmux(false).await;
                }
                if outcome.refresh_tool {
                    self.maybe_refresh_tool_from_tmux(false).await;
                }
            }
            Err(e) => {
                debug!(
                    session_id = %self.session_id,
                    tmux_name = %self.tmux_name,
                    "liveness check failed: {}",
                    e
                );
            }
        }
    }

    fn reconcile_liveness(&mut self, liveness: PaneLiveness) -> LivenessReconciliation {
        if !liveness.process_snapshot_fresh {
            debug!(
                session_id = %self.session_id,
                tmux_name = %self.tmux_name,
                "skipping liveness reconciliation from stale process snapshot"
            );
            return LivenessReconciliation::default();
        }

        let state_before = self.state_detector.state();
        let evidence_before = self.state_detector.state_evidence();
        self.state_detector
            .apply_process_liveness(liveness.has_children);
        let refresh_cwd = matches!(
            self.maybe_emit_state_change(state_before, evidence_before),
            Some(SessionState::Idle)
        );
        LivenessReconciliation {
            refresh_cwd,
            refresh_tool: liveness.has_children,
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

    /// Compare state/evidence before and after a detector operation. If either
    /// changed,
    /// emit a `session_state` ControlEvent through the per-session broadcast channel.
    fn maybe_emit_state_change(
        &self,
        previous_state: SessionState,
        previous_evidence: StateEvidence,
    ) -> Option<SessionState> {
        self.maybe_emit_state_change_with_exit_reason(previous_state, previous_evidence, None)
    }

    /// Emit a `session_state` ControlEvent if the state or its evidence changed,
    /// optionally including an `exit_reason` for terminal exit events.
    fn maybe_emit_state_change_with_exit_reason(
        &self,
        previous_state: SessionState,
        previous_evidence: StateEvidence,
        exit_reason: Option<String>,
    ) -> Option<SessionState> {
        let (current_state, current_command) = self.state_detector.get_state();
        let state_evidence = self.state_detector.state_evidence();
        let state_changed = current_state != previous_state;
        let evidence_changed = state_evidence != previous_evidence;
        if state_changed || evidence_changed {
            let payload = SessionStatePayload {
                state: current_state,
                previous_state,
                current_command,
                state_evidence,
                transport_health: TransportHealth::Healthy,
                exit_reason,
                at: Utc::now(),
            };
            debug!(
                session_id = %self.session_id,
                previous_state = ?payload.previous_state,
                state = ?payload.state,
                current_command = ?payload.current_command,
                state_evidence = ?payload.state_evidence,
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
            if state_changed {
                Some(current_state)
            } else {
                None
            }
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

        self.subscribers.retain(|_, tx| !tx.is_closed());
        if self.subscribers.len() >= MAX_OUTPUT_SUBSCRIBERS_PER_SESSION {
            warn!(
                session_id = %self.session_id,
                client_id,
                subscribers = self.subscribers.len(),
                "subscriber cap reached (SESSION_OVERLOADED), rejecting browser attach"
            );
            crate::metrics::increment_overload(&self.session_id);
            return SubscribeOutcome::Rejected {
                reason: format!(
                    "session already has {} active browser subscribers",
                    self.subscribers.len()
                ),
            };
        }

        let outcome = replay_existing_frames(
            self.session_id.clone(),
            client_id,
            &client_tx,
            self.replay_plan(resume_from_seq),
        )
        .await;
        if client_tx.is_closed() {
            debug!(
                session_id = %self.session_id,
                client_id,
                "subscriber dropped during subscribe ack; not attaching"
            );
        } else {
            self.subscribers.insert(client_id, client_tx);
        }
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
        let Some(detected_skill) = detect_skill_from_input_line(line) else {
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
        let state_evidence = self.state_detector.state_evidence();
        let active_subscribers = self
            .subscribers
            .values()
            .filter(|tx| !tx.is_closed())
            .count();
        let stale_subscribers = self.subscribers.len().saturating_sub(active_subscribers);
        let mut summary = SessionSummary::live(
            self.session_id.clone(),
            self.tmux_name.clone(),
            state,
            current_command,
            state_evidence,
            self.cwd.clone(),
            self.tool.clone(),
            active_subscribers as u32,
            stale_subscribers as u32,
            self.last_activity_at,
        );
        summary.last_skill = self.last_skill.clone();
        summary.batch = self.batch.clone();
        summary
    }
}

/// Capture visible pane text directly from tmux.
async fn capture_pane_tail(tmux_name: &str, lines: usize) -> anyhow::Result<String> {
    capture_pane_tail_with_command("tmux", tmux_name, lines).await
}

async fn capture_pane_tail_with_command(
    tmux_command: impl AsRef<std::ffi::OsStr>,
    tmux_name: &str,
    lines: usize,
) -> anyhow::Result<String> {
    let lines = lines.clamp(20, 1000);
    let start = format!("-{lines}");
    let target = exact_pane_target(tmux_name);

    let output = run_bounded_tmux_command(
        tmux_command,
        &["capture-pane", "-p", "-J", "-t", &target, "-S", &start],
        TMUX_CAPTURE_PANE_TIMEOUT,
        "capture-pane",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux capture-pane failed: {}",
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn output_counts_as_meaningful_activity(
    previous_state: SessionState,
    current_state: SessionState,
    chunk: &ScrollOutputChunk,
) -> bool {
    meaningful_output_activity_reason(previous_state, current_state, chunk).is_some()
}

enum MeaningfulOutputActivity {
    BusyBecameIdle,
    VisibleOutput,
}

fn meaningful_output_activity_reason(
    previous_state: SessionState,
    current_state: SessionState,
    chunk: &ScrollOutputChunk,
) -> Option<MeaningfulOutputActivity> {
    if chunk.coalesced_redraw {
        return None;
    }

    if output_transition_finished_busy_work(previous_state, current_state) {
        return Some(MeaningfulOutputActivity::BusyBecameIdle);
    }

    visible_output_is_meaningful(&chunk.data).then_some(MeaningfulOutputActivity::VisibleOutput)
}

fn output_transition_finished_busy_work(
    previous_state: SessionState,
    current_state: SessionState,
) -> bool {
    !matches!(previous_state, SessionState::Idle) && matches!(current_state, SessionState::Idle)
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
        .any(trimmed_line_counts_as_meaningful_output)
}

fn trimmed_line_counts_as_meaningful_output(line: &str) -> bool {
    if line_looks_prompt_like(line) {
        return false;
    }

    line_has_substantive_text(line)
}

fn line_has_substantive_text(line: &str) -> bool {
    line_has_enough_visible_chars(line) && line_has_alphanumeric_char(line)
}

fn line_has_enough_visible_chars(line: &str) -> bool {
    line.chars().filter(|c| !c.is_whitespace()).count() >= 3
}

fn line_has_alphanumeric_char(line: &str) -> bool {
    line.chars().any(|c| c.is_alphanumeric())
}

fn line_looks_prompt_like(line: &str) -> bool {
    prompt_candidate(line)
        .map(prompt_candidate_looks_prompt_like)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy)]
struct PromptCandidate<'a> {
    prefix: &'a str,
    marker: char,
}

fn prompt_candidate(line: &str) -> Option<PromptCandidate<'_>> {
    let line = line.trim_end();
    let mut chars = line.chars();
    let marker = chars.next_back()?;
    is_shell_prompt_marker(marker).then_some(PromptCandidate {
        prefix: chars.as_str().trim_end(),
        marker,
    })
}

fn is_shell_prompt_marker(marker: char) -> bool {
    matches!(marker, '$' | '%' | '#' | '>')
}

fn prompt_candidate_looks_prompt_like(candidate: PromptCandidate<'_>) -> bool {
    if candidate.prefix.is_empty() {
        return true;
    }

    match prompt_prefix_class(candidate.prefix) {
        PromptPrefixClass::PathOrUser => {
            path_prompt_marker_allowed(candidate.marker, candidate.prefix)
        }
        PromptPrefixClass::Plain => plain_prompt_marker_allowed(candidate.marker),
        PromptPrefixClass::Other => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptPrefixClass {
    PathOrUser,
    Plain,
    Other,
}

fn prompt_prefix_class(prefix: &str) -> PromptPrefixClass {
    path_prompt_prefix_class(prefix).unwrap_or_else(|| plain_prompt_prefix_class(prefix))
}

fn path_prompt_prefix_class(prefix: &str) -> Option<PromptPrefixClass> {
    prefix_has_path_or_user_marker(prefix).then_some(PromptPrefixClass::PathOrUser)
}

fn plain_prompt_prefix_class(prefix: &str) -> PromptPrefixClass {
    if plain_prefix_looks_prompt_like(prefix) {
        PromptPrefixClass::Plain
    } else {
        PromptPrefixClass::Other
    }
}

fn path_prompt_marker_allowed(marker: char, prefix: &str) -> bool {
    !path_prompt_is_zsh_jobs_summary(marker, prefix)
}

fn path_prompt_is_zsh_jobs_summary(marker: char, prefix: &str) -> bool {
    matches!(marker, '%') && prefix_is_zsh_jobs_summary(prefix)
}

fn plain_prompt_marker_allowed(marker: char) -> bool {
    matches!(marker, '$' | '#' | '%')
}

type PrefixRejector = fn(&str) -> bool;

const PLAIN_PROMPT_PREFIX_REJECTORS: [PrefixRejector; 4] = [
    plain_prefix_is_too_long,
    plain_prefix_has_whitespace,
    plain_prefix_is_numeric_progress,
    plain_prefix_has_invalid_chars,
];

fn plain_prefix_looks_prompt_like(prefix: &str) -> bool {
    !PLAIN_PROMPT_PREFIX_REJECTORS
        .iter()
        .any(|reject| reject(prefix))
}

fn plain_prefix_is_too_long(prefix: &str) -> bool {
    prefix.len() > 32
}

fn plain_prefix_has_whitespace(prefix: &str) -> bool {
    prefix.chars().any(|c| c.is_whitespace())
}

fn plain_prefix_is_numeric_progress(prefix: &str) -> bool {
    prefix.chars().all(is_numeric_progress_char)
}

fn is_numeric_progress_char(c: char) -> bool {
    matches!(c, '0'..='9' | '.' | ',')
}

fn plain_prefix_has_invalid_chars(prefix: &str) -> bool {
    !prefix.chars().all(is_plain_prompt_char)
}

fn is_plain_prompt_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn prefix_has_path_or_user_marker(prefix: &str) -> bool {
    prefix_contains_path_or_user_char(prefix) || prefix_has_prompt_wrapper_suffix(prefix)
}

fn prefix_contains_path_or_user_char(prefix: &str) -> bool {
    prefix.chars().any(is_path_or_user_char)
}

fn is_path_or_user_char(c: char) -> bool {
    matches!(c, '@' | ':' | '/' | '~' | '\\')
}

fn prefix_has_prompt_wrapper_suffix(prefix: &str) -> bool {
    matches!(prefix.chars().last(), Some(')' | ']'))
}

fn prefix_is_zsh_jobs_summary(prefix: &str) -> bool {
    // zsh's `%` jobs summary line ends in `... 12.34%`; reject those.
    let compact = prefix.replace(',', "");
    compact.chars().all(is_zsh_jobs_summary_char)
}

fn is_zsh_jobs_summary_char(c: char) -> bool {
    c.is_ascii_digit() || c == '.' || c.is_ascii_whitespace()
}

/// Query tmux for the active pane cwd of a session.
async fn query_tmux_display_message(tmux_name: &str, format: &str) -> anyhow::Result<String> {
    let target = exact_pane_target(tmux_name);
    let output = run_bounded_tmux_command(
        "tmux",
        &["display-message", "-p", "-t", &target, format],
        TMUX_DISPLAY_MESSAGE_TIMEOUT,
        "display-message",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux display-message failed: {}",
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn query_tmux_cwd(tmux_name: &str) -> anyhow::Result<String> {
    let cwd = query_tmux_display_message(tmux_name, "#{pane_current_path}").await?;
    if cwd.is_empty() {
        return Err(anyhow::anyhow!("tmux returned empty pane_current_path"));
    }
    Ok(cwd)
}

async fn query_tmux_session_created(tmux_name: &str) -> anyhow::Result<chrono::DateTime<Utc>> {
    let epoch = query_tmux_display_message(tmux_name, "#{session_created}")
        .await?
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
    pcpu: f32,
    comm: String,
    args: String,
}

#[derive(Debug, Default)]
struct ProcessEntriesCache {
    fetched_at: Option<Instant>,
    entries: Vec<ProcessEntry>,
}

struct ProcessEntriesSnapshot {
    entries: Vec<ProcessEntry>,
    fresh: bool,
}

static PROCESS_ENTRIES_CACHE: OnceLock<Mutex<ProcessEntriesCache>> = OnceLock::new();

fn process_entries_cache() -> &'static Mutex<ProcessEntriesCache> {
    PROCESS_ENTRIES_CACHE.get_or_init(|| Mutex::new(ProcessEntriesCache::default()))
}

async fn query_tool_from_tmux_process_tree(tmux_name: &str) -> anyhow::Result<Option<String>> {
    if let Ok(comm) = query_tmux_current_command(tmux_name).await {
        if let Some(tool) = crate::types::detect_tool_name(&comm) {
            return Ok(Some(tool.to_string()));
        }
    }

    let pane_pid = query_tmux_pane_pid(tmux_name).await?;
    let snapshot = query_process_entries().await?;
    if !snapshot.fresh {
        debug!(
            tmux_name,
            "skipping tool detection from stale process snapshot"
        );
        return Ok(None);
    }
    let entries = snapshot.entries;

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

/// Result of a process-tree liveness check for a tmux pane.
#[derive(Debug, Clone, Copy)]
struct PaneLiveness {
    /// True when the pane's shell has at least one child process.
    has_children: bool,
    /// Sum of `%cpu` across all descendant processes (excludes the shell itself).
    #[allow(dead_code)]
    descendant_cpu: f32,
    /// True only when the process tree came from a fresh `ps` snapshot.
    process_snapshot_fresh: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct LivenessReconciliation {
    refresh_cwd: bool,
    refresh_tool: bool,
}

/// Query whether the pane's shell process has running children and their
/// aggregate CPU usage. This is the ground-truth signal for idle vs busy:
/// if the shell is the leaf process, no command is running regardless of what
/// the terminal output looks like.
async fn query_pane_liveness(tmux_name: &str) -> anyhow::Result<PaneLiveness> {
    let pane_pid = query_tmux_pane_pid(tmux_name).await?;
    let snapshot = query_process_entries().await?;
    let mut liveness = compute_pane_liveness(pane_pid, snapshot.entries);
    liveness.process_snapshot_fresh = snapshot.fresh;
    Ok(liveness)
}

/// Pure BFS over the process tree rooted at `pane_pid`. Exported for testing.
fn compute_pane_liveness(pane_pid: u32, entries: Vec<ProcessEntry>) -> PaneLiveness {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut by_pid: HashMap<u32, ProcessEntry> = HashMap::new();

    for entry in entries {
        children.entry(entry.ppid).or_default().push(entry.pid);
        by_pid.insert(entry.pid, entry);
    }

    // Walk descendants of the pane pid (excluding the shell itself).
    let mut has_children = false;
    let mut descendant_cpu: f32 = 0.0;
    let mut queue: VecDeque<u32> = VecDeque::new();
    let mut visited: HashSet<u32> = HashSet::new();
    visited.insert(pane_pid);

    if let Some(direct_children) = children.get(&pane_pid) {
        for &child_pid in direct_children {
            queue.push_back(child_pid);
        }
    }

    while let Some(pid) = queue.pop_front() {
        if !visited.insert(pid) {
            continue;
        }
        has_children = true;
        if let Some(entry) = by_pid.get(&pid) {
            descendant_cpu += entry.pcpu;
        }
        if let Some(grandchildren) = children.get(&pid) {
            for &gc in grandchildren {
                queue.push_back(gc);
            }
        }
    }

    PaneLiveness {
        has_children,
        descendant_cpu,
        process_snapshot_fresh: true,
    }
}

async fn query_tmux_current_command(tmux_name: &str) -> anyhow::Result<String> {
    let comm = query_tmux_display_message(tmux_name, "#{pane_current_command}").await?;
    if comm.is_empty() {
        return Err(anyhow::anyhow!("tmux returned empty pane_current_command"));
    }
    Ok(comm)
}

async fn query_tmux_pane_pid(tmux_name: &str) -> anyhow::Result<u32> {
    let pane_pid = query_tmux_display_message(tmux_name, "#{pane_pid}")
        .await?
        .parse::<u32>()
        .map_err(|e| anyhow::anyhow!("invalid pane_pid from tmux: {}", e))?;

    Ok(pane_pid)
}

async fn query_process_entries() -> anyhow::Result<ProcessEntriesSnapshot> {
    let mut cache = process_entries_cache().lock().await;
    if cache
        .fetched_at
        .map(|fetched_at| fetched_at.elapsed() <= PROCESS_ENTRIES_CACHE_TTL)
        .unwrap_or(false)
    {
        return Ok(ProcessEntriesSnapshot {
            entries: cache.entries.clone(),
            fresh: true,
        });
    }

    match query_process_entries_uncached().await {
        Ok(entries) => {
            cache.fetched_at = Some(Instant::now());
            cache.entries = entries.clone();
            Ok(ProcessEntriesSnapshot {
                entries,
                fresh: true,
            })
        }
        Err(err) if !cache.entries.is_empty() => {
            debug!(
                "using stale process snapshot after ps refresh failed: {}",
                err
            );
            Ok(ProcessEntriesSnapshot {
                entries: cache.entries.clone(),
                fresh: false,
            })
        }
        Err(err) => Err(err),
    }
}

async fn query_process_entries_uncached() -> anyhow::Result<Vec<ProcessEntry>> {
    let mut command = Command::new("ps");
    command
        .args(["-axo", "pid=,ppid=,pcpu=,comm=,args="])
        .kill_on_drop(true);

    let output = tokio::time::timeout(PROCESS_ENTRIES_QUERY_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "ps timed out after {}ms",
                PROCESS_ENTRIES_QUERY_TIMEOUT.as_millis()
            )
        })?
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
    let pcpu = parts.next()?.parse::<f32>().ok()?;
    let comm = parts.next()?.to_string();
    let args = parts.collect::<Vec<&str>>().join(" ");

    Some(ProcessEntry {
        pid,
        ppid,
        pcpu,
        comm,
        args,
    })
}

fn detect_tool_from_process_entry(entry: &ProcessEntry) -> Option<&'static str> {
    crate::types::detect_tool_name(&entry.comm)
        .or_else(|| detect_tool_from_command_line(&entry.args))
}

fn detect_tool_from_command_line(command: &str) -> Option<&'static str> {
    command
        .split_whitespace()
        .find_map(crate::types::detect_tool_name)
}

fn current_command_tool_update(
    current_command: Option<&str>,
    current_tool: Option<&str>,
) -> Option<&'static str> {
    let tool = current_command.and_then(detect_tool_from_command_line)?;
    (current_tool != Some(tool)).then_some(tool)
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
    let mut out = Vec::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(h), Some(l)) = (hi, lo) {
                let hex = [h, l];
                if let Ok(s) = std::str::from_utf8(&hex) {
                    if let Ok(val) = u8::from_str_radix(s, 16) {
                        out.push(val);
                        continue;
                    }
                }
            }
            out.push(b);
        } else {
            out.push(b);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
        if let Ok(home) = std::env::var("HOME") {
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
        ("grok", "Grok"),
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
        capture_pane_tail_with_command, compute_pane_liveness, cwd_from_osc7_payload,
        detect_tool_from_command_line, detect_tool_from_process_entry, extract_cwd_from_title,
        find_osc_payload_end, line_looks_prompt_like, normalize_submit_line_text, osc_payloads,
        output_counts_as_meaningful_activity, parse_process_entry, percent_decode,
        process_entries_cache, query_tmux_session_created, query_tool_from_tmux_process_tree,
        resolve_tmux_terminal_env, run_bounded_tmux_command, should_refresh_cwd_from_tmux,
        should_refresh_tool_from_tmux, state_detector_for_initial_tool, submit_line_fallback_input,
        tmux_input_chunks, visible_output_is_meaningful, write_and_flush_input,
        write_input_counts_as_activity, ControlEvent, PaneLiveness, ProcessEntriesCache,
        ProcessEntry, SessionActor, SessionCommand, TmuxInputChunk, CWD_REFRESH_MIN_INTERVAL,
        PROCESS_ENTRIES_CACHE_TTL, TOOL_REFRESH_MIN_INTERVAL,
    };
    use crate::config::Config;
    use crate::scroll::guard::ScrollGuard;
    use crate::scroll::guard::ScrollOutputChunk;
    use crate::session::replay_ring::ReplayRing;
    use crate::types::{SessionState, SessionStatePayload};
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
            state_detector: state_detector_for_initial_tool(Some("Codex")),
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
            last_liveness_check_at: Instant::now(),
            tool: Some("Codex".to_string()),
            last_skill: None,
            batch: None,
            input_line_buffer: String::new(),
            last_activity_at: Utc::now(),
            session_started_at: Utc::now(),
            clear_replay_on_first_idle: false,
        }
    }

    async fn clear_process_entries_cache() {
        let mut cache = process_entries_cache().lock().await;
        *cache = ProcessEntriesCache::default();
    }

    async fn seed_process_entries_cache(entries: Vec<ProcessEntry>, fetched_at: Instant) {
        let mut cache = process_entries_cache().lock().await;
        cache.fetched_at = Some(fetched_at);
        cache.entries = entries;
    }

    fn restore_path(previous_path: Option<std::ffi::OsString>) {
        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    fn make_executable(path: &std::path::Path) {
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    fn install_fake_tmux(script: &str) -> (tempfile::TempDir, Option<std::ffi::OsString>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux = bin_dir.join("tmux");
        std::fs::write(&tmux, script).expect("tmux script");
        make_executable(&tmux);
        let previous_path = std::env::var_os("PATH");
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = previous_path.as_ref() {
            entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
        }
        for system_dir in ["/bin", "/usr/bin"] {
            let system_dir = std::path::Path::new(system_dir);
            if system_dir.is_dir()
                && !entries
                    .iter()
                    .any(|entry| std::path::Path::new(entry) == system_dir)
            {
                entries.push(system_dir.as_os_str().to_os_string());
            }
        }
        std::env::set_var("PATH", std::env::join_paths(entries).expect("path"));
        (dir, previous_path)
    }

    #[test]
    fn evidence_refresh_emits_session_state_event_without_state_transition() {
        let mut actor = test_actor();
        let mut rx = actor.event_tx.subscribe();
        let previous_state = actor.state_detector.state();
        let previous_evidence = actor.state_detector.state_evidence();

        actor.state_detector.process_output(b"\x1b]133;A\x07");
        let result = actor.maybe_emit_state_change(previous_state, previous_evidence);

        assert_eq!(result, None);
        let event = rx.try_recv().expect("session_state event");
        assert_eq!(event.event, "session_state");
        let payload: SessionStatePayload =
            serde_json::from_value(event.payload).expect("session_state payload");
        assert_eq!(payload.state, SessionState::Idle);
        assert_eq!(payload.previous_state, SessionState::Idle);
        assert_eq!(payload.state_evidence.cause, "osc133_prompt");
        assert_eq!(
            payload.state_evidence.confidence,
            crate::types::StateConfidence::High
        );
    }

    #[test]
    fn initial_tool_enables_tui_mode_before_liveness_reconciliation() {
        let mut actor = test_actor();

        actor.state_detector.note_input();
        actor.reconcile_liveness(PaneLiveness {
            has_children: true,
            descendant_cpu: 0.0,
            process_snapshot_fresh: true,
        });

        assert_eq!(actor.state_detector.state(), SessionState::Busy);
        assert_eq!(actor.state_detector.state_evidence().cause, "local_input");
    }

    #[tokio::test]
    async fn maybe_check_liveness_skips_exited_sessions() {
        let mut actor = test_actor();
        actor.state_detector.mark_exited();
        // Should return immediately without trying tmux (tmux_name "demo" does not exist)
        actor.maybe_check_liveness().await;
        // If we reach here without hanging/panicking, the early-return worked
    }

    #[tokio::test]
    async fn build_summary_reports_drowsy_when_idle_past_threshold() {
        // End-to-end wiring check: prove that build_summary feeds
        // self.last_activity_at into rest_state_from_idle and that the result
        // lands on SessionSummary.rest_state unclobbered. Pure math for the
        // ladder is covered by types::rest_state_tests; this guards the
        // actor-side plumbing.
        let mut actor = test_actor();
        // StateDetector::new() defaults to SessionState::Idle.
        let aged = Utc::now() - chrono::Duration::minutes(10);
        actor.last_activity_at = aged;

        let summary = actor.build_summary();

        assert_eq!(summary.state, crate::types::SessionState::Idle);
        assert_eq!(summary.rest_state, crate::types::RestState::Drowsy);
        assert_eq!(summary.last_activity_at, aged);
    }

    #[tokio::test]
    async fn build_summary_reports_active_for_fresh_idle_session() {
        // Regression guard: a brand-new idle session (last_activity_at = now)
        // must not immediately report Drowsy/Sleeping.
        let actor = test_actor();
        let summary = actor.build_summary();
        assert_eq!(summary.state, crate::types::SessionState::Idle);
        assert_eq!(summary.rest_state, crate::types::RestState::Active);
    }

    #[test]
    fn handle_resize_clamps_zero_and_one_cell_dimensions() {
        let mut actor = test_actor();

        actor.handle_resize(0, 1);

        assert_eq!(actor.cols, crate::types::TERMINAL_RESIZE_MIN_COLS);
        assert_eq!(actor.rows, crate::types::TERMINAL_RESIZE_MIN_ROWS);
    }

    #[test]
    fn handle_resize_clamps_huge_dimensions() {
        let mut actor = test_actor();

        actor.handle_resize(u16::MAX, u16::MAX);

        assert_eq!(actor.cols, crate::types::TERMINAL_RESIZE_MAX_COLS);
        assert_eq!(actor.rows, crate::types::TERMINAL_RESIZE_MAX_ROWS);
    }

    #[tokio::test]
    async fn maybe_check_liveness_throttled_by_interval() {
        let mut actor = test_actor();
        // last_liveness_check_at is set to Instant::now() by test_actor,
        // so the interval guard fires immediately and we never touch tmux.
        actor.maybe_check_liveness().await;
    }

    #[tokio::test]
    async fn maybe_check_liveness_runs_query_when_interval_elapsed() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, previous_path) = install_fake_tmux("#!/bin/sh\nexit 1\n");
        let mut actor = test_actor();
        // Push last_liveness_check_at far enough back to pass the interval guard.
        actor.last_liveness_check_at = Instant::now() - Duration::from_millis(2_100); // past LIVENESS_CHECK_INTERVAL (2s)
                                                                                      // query_pane_liveness will fail for tmux_name "demo" (no real tmux),
                                                                                      // but the Err branch just logs — it must not panic.
        actor.maybe_check_liveness().await;
        // last_liveness_check_at is updated even on query failure
        assert!(actor.last_liveness_check_at.elapsed() < Duration::from_secs(1));
        restore_path(previous_path);
    }

    #[tokio::test]
    async fn maybe_check_liveness_skips_stale_process_cache_that_would_mark_busy() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_process_entries_cache().await;

        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux = bin_dir.join("tmux");
        std::fs::write(
            &tmux,
            r##"#!/bin/sh
if [ "${5-}" = "#{pane_pid}" ]; then
  printf '101\n'
elif [ "${5-}" = "#{pane_current_command}" ]; then
  printf 'bash\n'
else
  printf '\n'
fi
"##,
        )
        .expect("tmux");
        let ps = bin_dir.join("ps");
        std::fs::write(&ps, "#!/bin/sh\nprintf 'ps unavailable\\n' >&2\nexit 1\n").expect("ps");
        make_executable(&tmux);
        make_executable(&ps);

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );
        seed_process_entries_cache(
            vec![proc(101, 1, 0.0), proc(102, 101, 0.0)],
            Instant::now() - PROCESS_ENTRIES_CACHE_TTL - Duration::from_millis(1),
        )
        .await;

        let mut actor = test_actor();
        actor.last_liveness_check_at = Instant::now() - Duration::from_secs(3);
        actor.maybe_check_liveness().await;

        assert_eq!(actor.state_detector.state(), SessionState::Idle);
        assert_eq!(actor.state_detector.state_evidence().cause, "initial_state");

        restore_path(previous_path);
        clear_process_entries_cache().await;
    }

    #[tokio::test]
    async fn maybe_check_liveness_skips_stale_process_cache_that_would_mark_idle() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_process_entries_cache().await;

        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux = bin_dir.join("tmux");
        std::fs::write(
            &tmux,
            r##"#!/bin/sh
if [ "${5-}" = "#{pane_pid}" ]; then
  printf '101\n'
elif [ "${5-}" = "#{pane_current_command}" ]; then
  printf 'bash\n'
else
  printf '\n'
fi
"##,
        )
        .expect("tmux");
        let ps = bin_dir.join("ps");
        std::fs::write(&ps, "#!/bin/sh\nprintf 'ps unavailable\\n' >&2\nexit 1\n").expect("ps");
        make_executable(&tmux);
        make_executable(&ps);

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );
        seed_process_entries_cache(
            vec![proc(101, 1, 0.0)],
            Instant::now() - PROCESS_ENTRIES_CACHE_TTL - Duration::from_millis(1),
        )
        .await;

        let mut actor = test_actor();
        actor.state_detector.note_input();
        actor.last_liveness_check_at = Instant::now() - Duration::from_secs(3);
        actor.maybe_check_liveness().await;

        assert_eq!(actor.state_detector.state(), SessionState::Busy);
        assert_eq!(actor.state_detector.state_evidence().cause, "local_input");

        restore_path(previous_path);
        clear_process_entries_cache().await;
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
            parse_process_entry("10715 37039 2.3 claude /usr/local/bin/claude --print").unwrap();
        assert_eq!(entry.pid, 10_715);
        assert_eq!(entry.ppid, 37_039);
        assert!((entry.pcpu - 2.3).abs() < f32::EPSILON);
        assert_eq!(entry.comm, "claude");
        assert_eq!(entry.args, "/usr/local/bin/claude --print");
    }

    #[test]
    fn detect_tool_from_process_entry_checks_comm_then_args() {
        let from_comm = ProcessEntry {
            pid: 1,
            ppid: 0,
            pcpu: 0.0,
            comm: "codex".to_string(),
            args: "codex".to_string(),
        };
        assert_eq!(detect_tool_from_process_entry(&from_comm), Some("Codex"));

        let from_args = ProcessEntry {
            pid: 2,
            ppid: 1,
            pcpu: 0.0,
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
        assert_eq!(percent_decode("/tmp/caf%C3%A9"), "/tmp/caf\u{e9}");
        assert_eq!(percent_decode("%ZZ/path"), "%/path");
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
        assert_eq!(
            cwd_from_osc7_payload("file://host/tmp/caf%C3%A9"),
            Some("/tmp/caf\u{e9}".to_string())
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
        clear_process_entries_cache().await;
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
            "#!/bin/sh\nprintf '101 1 0.0 bash bash\\n102 101 5.2 node /usr/local/bin/claude --print\\n'\n",
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
        clear_process_entries_cache().await;
    }

    #[tokio::test]
    async fn query_tool_from_tmux_process_tree_skips_stale_process_cache() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_process_entries_cache().await;
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
        std::fs::write(&ps, "#!/bin/sh\nprintf 'ps unavailable\\n' >&2\nexit 1\n").expect("ps");
        make_executable(&tmux);
        make_executable(&ps);

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );
        seed_process_entries_cache(
            vec![
                ProcessEntry {
                    pid: 101,
                    ppid: 1,
                    pcpu: 0.0,
                    comm: "bash".to_string(),
                    args: "bash".to_string(),
                },
                ProcessEntry {
                    pid: 102,
                    ppid: 101,
                    pcpu: 0.0,
                    comm: "node".to_string(),
                    args: "/usr/local/bin/claude --print".to_string(),
                },
            ],
            Instant::now() - PROCESS_ENTRIES_CACHE_TTL - Duration::from_millis(1),
        )
        .await;

        let tool = query_tool_from_tmux_process_tree("demo")
            .await
            .expect("tool query");
        assert_eq!(tool, None);

        restore_path(previous_path);
        clear_process_entries_cache().await;
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

    fn set_tracking_writer(actor: &mut SessionActor) -> Arc<Mutex<TrackingWriterState>> {
        let state = Arc::new(Mutex::new(TrackingWriterState::default()));
        actor.writer = Box::new(TrackingWriter {
            state: Arc::clone(&state),
        });
        state
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

    #[tokio::test]
    async fn handle_write_input_ignores_closed_pty_without_activity() {
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);
        let mut rx = actor.event_tx.subscribe();

        actor.handle_write_input(b"hello\r".to_vec(), true).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(writer_state.writes.is_empty());
        assert_eq!(writer_state.flushes, 0);
        assert_eq!(actor.state_detector.state(), SessionState::Idle);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn handle_write_input_uses_tmux_send_keys_without_raw_writer_when_available() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
        std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
        let (_dir, previous_path) = install_fake_tmux(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
exit 0
"#,
        );
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);
        let mut rx = actor.event_tx.subscribe();

        actor.handle_write_input(b"hello\r".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(writer_state.writes.is_empty());
        assert_eq!(writer_state.flushes, 0);
        assert_eq!(actor.state_detector.state(), SessionState::Busy);
        assert_eq!(actor.state_detector.state_evidence().cause, "local_input");
        let event = rx.try_recv().expect("session_state event");
        let payload: SessionStatePayload =
            serde_json::from_value(event.payload).expect("session_state payload");
        assert_eq!(payload.state, SessionState::Busy);
        assert_eq!(payload.state_evidence.cause, "local_input");

        let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
        assert!(log.contains("send-keys -t =demo: -X cancel"));
        assert!(log.contains("send-keys -t =demo: -l hello"));
        assert!(log.contains("send-keys -t =demo: Enter"));

        std::env::remove_var("TMUX_SEND_LOG");
        restore_path(previous_path);
    }

    #[tokio::test]
    async fn handle_write_input_falls_back_to_raw_writer_when_tmux_send_keys_fails() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, previous_path) = install_fake_tmux(
            r#"#!/bin/sh
printf 'no such target\n' >&2
exit 1
"#,
        );
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);

        actor.handle_write_input(b"hello\r".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(writer_state.writes, b"hello\r");
        assert_eq!(writer_state.flushes, 1);
        assert_eq!(actor.state_detector.state(), SessionState::Busy);
        assert_eq!(actor.state_detector.state_evidence().cause, "local_input");

        restore_path(previous_path);
    }

    #[tokio::test]
    async fn handle_write_input_does_not_replay_raw_buffer_after_partial_tmux_delivery() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
        std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
        let (_dir, previous_path) = install_fake_tmux(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
case "$*" in
  *" Enter") exit 1 ;;
  *) exit 0 ;;
esac
"#,
        );
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);

        actor.handle_write_input(b"hello\r".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(writer_state.writes.is_empty());
        assert_eq!(writer_state.flushes, 0);

        let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
        assert!(log.contains("send-keys -t =demo: -l hello"));
        assert!(log.contains("send-keys -t =demo: Enter"));

        std::env::remove_var("TMUX_SEND_LOG");
        restore_path(previous_path);
    }

    #[tokio::test]
    async fn handle_write_input_preserves_control_byte_fallback_payloads() {
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);

        actor.handle_write_input(b"abc\t".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(writer_state.writes, b"abc\t");
        assert_eq!(writer_state.flushes, 1);
        assert_eq!(actor.state_detector.state(), SessionState::Busy);
    }

    #[tokio::test]
    async fn handle_submit_line_uses_tmux_paste_buffer_and_double_enter() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
        std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
        let (_dir, previous_path) = install_fake_tmux(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
exit 0
"#,
        );
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);
        let mut rx = actor.event_tx.subscribe();

        actor
            .handle_submit_line("hello codex\n".to_string(), false)
            .await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(writer_state.writes.is_empty());
        assert_eq!(writer_state.flushes, 0);
        assert_eq!(actor.state_detector.state(), SessionState::Busy);
        let event = rx.try_recv().expect("session_state event");
        let payload: SessionStatePayload =
            serde_json::from_value(event.payload).expect("session_state payload");
        assert_eq!(payload.state, SessionState::Busy);

        let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
        assert!(log.contains("send-keys -t =demo: -X cancel"));
        assert!(log.contains("set-buffer -b swimmers-submit-"));
        assert!(log.contains("-- hello codex"));
        assert!(log.contains("paste-buffer -dpr -b swimmers-submit-"));
        assert_eq!(
            log.lines()
                .filter(|line| *line == "send-keys -t =demo: Enter")
                .count(),
            2
        );

        std::env::remove_var("TMUX_SEND_LOG");
        restore_path(previous_path);
    }

    #[tokio::test]
    async fn handle_submit_line_falls_back_to_raw_writer_when_tmux_submit_fails() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, previous_path) = install_fake_tmux(
            r#"#!/bin/sh
printf 'no such target\n' >&2
exit 1
"#,
        );
        let mut actor = test_actor();
        let writer_state = set_tracking_writer(&mut actor);

        actor
            .handle_submit_line("hello codex".to_string(), false)
            .await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(writer_state.writes, b"hello codex\r\r");
        assert_eq!(writer_state.flushes, 1);
        assert_eq!(actor.state_detector.state(), SessionState::Busy);

        restore_path(previous_path);
    }

    #[test]
    fn tmux_input_chunks_splits_literal_text_and_enter() {
        assert_eq!(
            tmux_input_chunks(b"printf \"hello\\n\"\r"),
            Some(vec![
                TmuxInputChunk::Literal("printf \"hello\\n\"".to_string()),
                TmuxInputChunk::Enter,
            ])
        );
    }

    #[test]
    fn tmux_input_chunks_rejects_control_sequences() {
        assert_eq!(tmux_input_chunks(b"\x1b[A"), None);
        assert_eq!(tmux_input_chunks(b"abc\t"), None);
    }

    #[test]
    fn normalize_submit_line_text_trims_trailing_newlines_only() {
        assert_eq!(
            normalize_submit_line_text("  hello codex  \n\n"),
            Some("  hello codex  ".to_string())
        );
        assert_eq!(normalize_submit_line_text("\r\n"), None);
    }

    #[test]
    fn submit_line_fallback_input_adds_double_enter() {
        assert_eq!(submit_line_fallback_input("hello"), b"hello\r\r");
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

        let captured = capture_pane_tail_with_command(&tmux, "0", 20)
            .await
            .expect("capture pane");
        assert_eq!(captured.trim(), "captured");
        assert_eq!(
            std::fs::read_to_string(&target_file).expect("target file"),
            "=0:\n"
        );
    }

    #[tokio::test]
    async fn bounded_tmux_command_scrubs_nested_tmux_env_vars() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let tmux = dir.path().join("tmux");
        std::fs::write(
            &tmux,
            "#!/bin/sh\nprintf 'TMUX=%s\\nTMUX_PANE=%s\\n' \"${TMUX-unset}\" \"${TMUX_PANE-unset}\"\n",
        )
        .expect("tmux");
        let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmux, perms).expect("chmod");

        let previous_tmux = std::env::var_os("TMUX");
        let previous_tmux_pane = std::env::var_os("TMUX_PANE");
        std::env::set_var("TMUX", "/tmp/tmux,123,0");
        std::env::set_var("TMUX_PANE", "%1");

        let output = run_bounded_tmux_command(
            tmux.as_os_str(),
            &["display-message"],
            Duration::from_secs(2),
            "test-env-scrub",
        )
        .await;

        match previous_tmux {
            Some(value) => std::env::set_var("TMUX", value),
            None => std::env::remove_var("TMUX"),
        }
        match previous_tmux_pane {
            Some(value) => std::env::set_var("TMUX_PANE", value),
            None => std::env::remove_var("TMUX_PANE"),
        }

        let output = output.expect("tmux env probe");

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "TMUX=unset\nTMUX_PANE=unset\n"
        );
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

    fn proc(pid: u32, ppid: u32, pcpu: f32) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid,
            pcpu,
            comm: "test".to_string(),
            args: String::new(),
        }
    }

    #[test]
    fn compute_pane_liveness_idle_shell_has_no_children() {
        // pane_pid 100 has no child processes
        let liveness = compute_pane_liveness(100, vec![proc(99, 1, 0.0), proc(101, 99, 0.0)]);
        assert!(!liveness.has_children);
        assert_eq!(liveness.descendant_cpu, 0.0);
    }

    #[test]
    fn compute_pane_liveness_direct_child_marks_busy() {
        // pane_pid 100 has child 101
        let liveness = compute_pane_liveness(100, vec![proc(100, 1, 0.0), proc(101, 100, 2.5)]);
        assert!(liveness.has_children);
        assert!((liveness.descendant_cpu - 2.5).abs() < 0.01);
    }

    #[test]
    fn compute_pane_liveness_sums_deep_descendant_cpu() {
        // pane 100 → child 101 → grandchild 102
        let entries = vec![proc(100, 1, 0.0), proc(101, 100, 1.0), proc(102, 101, 3.0)];
        let liveness = compute_pane_liveness(100, entries);
        assert!(liveness.has_children);
        assert!((liveness.descendant_cpu - 4.0).abs() < 0.01);
    }

    #[test]
    fn compute_pane_liveness_empty_process_list_is_idle() {
        let liveness = compute_pane_liveness(100, vec![]);
        assert!(!liveness.has_children);
    }
}
