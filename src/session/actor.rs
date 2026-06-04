use std::collections::HashMap;
use std::ffi::OsStr;
use std::process::Output;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use portable_pty::{native_pty_system, MasterPty, PtySize};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::scroll::guard::{ScrollGuard, ScrollOutputChunk};
use crate::session::artifact_responses::{
    build_mermaid_artifact_response, build_plan_file_response_async,
};
use crate::session::replay_ring::ReplayRing;
use crate::session::skill_detection::{detect_skill_from_input_line, drain_completed_input_lines};
use crate::state::detector::StateDetector;
use crate::tmux_target::exact_pane_target;
use crate::types::{
    clamp_terminal_resize, ControlEvent, MermaidArtifactResponse, PlanFileResponse,
    SessionBatchMembership, SessionSkillPayload, SessionState, SessionStatePayload, SessionSummary,
    StateEvidence, TerminalSnapshot, TransportHealth,
};

mod activity;
mod liveness;
mod metadata;
mod percent_decode;
mod process_tree;
mod spawn;
mod tmux_input;

use self::activity::output_counts_as_meaningful_activity;
#[cfg(test)]
use self::metadata::{
    cwd_from_osc7_payload, cwd_update, extract_cwd_from_title, find_osc_payload_end,
    osc7_cwd_update_plan, osc_payloads, should_refresh_cwd_from_tmux,
    should_refresh_tool_from_tmux, title_cwd_update, title_tool_update, tool_refresh_changes_tool,
    CWD_REFRESH_MIN_INTERVAL, TOOL_REFRESH_MIN_INTERVAL,
};
use self::metadata::{query_tmux_display_message, state_detector_for_initial_tool};
use self::process_tree::query_pane_liveness;
use self::process_tree::{PaneLiveness, ProcessEntry, ProcessTreeIndex};
use self::spawn::{
    build_spawned_session_actor, build_tmux_spawn_command, initial_spawn_pty_size,
    inspect_tmux_child_after_spawn, validate_spawn_start_cwd, SpawnedSessionActorInit,
    TmuxSpawnMode,
};
#[cfg(test)]
use self::spawn::{
    build_tmux_spawn_command_args, resolve_tmux_colorterm, resolve_tmux_term,
    resolve_tmux_terminal_env,
};

#[cfg(test)]
use self::tmux_input::TmuxInputChunk;
use self::tmux_input::{
    normalize_submit_line_text, send_tmux_input_chunks, send_tmux_submit_line,
    submit_line_fallback_input, tmux_input_chunks, write_and_flush_input,
    write_input_counts_as_activity, TmuxInputSendError,
};

const LIVENESS_CHECK_INTERVAL: Duration = Duration::from_millis(2_000);
const TMUX_CAPTURE_PANE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OUTPUT_SUBSCRIBERS_PER_SESSION: usize = 16;

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

struct SubscribeAcceptance {
    client_id: ClientId,
    client_tx: mpsc::Sender<OutputFrame>,
    replay_plan: ReplayPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SubscribeRejection {
    reason: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateChangeDetection {
    previous_state: SessionState,
    current_state: SessionState,
    current_command: Option<String>,
    previous_evidence: StateEvidence,
    current_evidence: StateEvidence,
    state_changed: bool,
    evidence_changed: bool,
}

impl StateChangeDetection {
    fn should_emit_event(&self) -> bool {
        self.state_changed || self.evidence_changed
    }

    fn changed_state(&self) -> Option<SessionState> {
        self.state_changed.then_some(self.current_state)
    }

    fn into_payload(self, exit_reason: Option<String>) -> SessionStatePayload {
        SessionStatePayload {
            state: self.current_state,
            previous_state: self.previous_state,
            current_command: self.current_command,
            state_evidence: self.current_evidence,
            transport_health: TransportHealth::Healthy,
            exit_reason,
            at: Utc::now(),
        }
    }
}

fn compare_session_state_change(
    previous_state: SessionState,
    previous_evidence: StateEvidence,
    current_state: SessionState,
    current_command: Option<String>,
    current_evidence: StateEvidence,
) -> StateChangeDetection {
    let state_changed = current_state != previous_state;
    let evidence_changed = current_evidence != previous_evidence;
    StateChangeDetection {
        previous_state,
        current_state,
        current_command,
        previous_evidence,
        current_evidence,
        state_changed,
        evidence_changed,
    }
}

async fn try_tmux_write_input(
    tmux_name: String,
    data: &[u8],
) -> Option<Result<(), TmuxInputSendError>> {
    let chunks = tmux_input_chunks(data)?;
    Some(send_tmux_input_chunks(&tmux_name, &chunks).await)
}

fn tmux_write_input_result(
    session_id: &str,
    tmux_name: &str,
    result: Result<(), TmuxInputSendError>,
) -> Option<InputDeliveryResult> {
    match result {
        Ok(()) => Some(InputDeliveryResult::delivered("tmux_send_keys")),
        Err(err) => tmux_write_input_error_result(session_id, tmux_name, err),
    }
}

fn tmux_write_input_error_result(
    session_id: &str,
    tmux_name: &str,
    err: TmuxInputSendError,
) -> Option<InputDeliveryResult> {
    warn!(
        session_id = %session_id,
        tmux_name = %tmux_name,
        delivered_chunks = err.delivered_chunks,
        "tmux send-keys input failed: {err}"
    );
    (err.delivered_chunks > 0).then(|| InputDeliveryResult::delivered("tmux_send_keys_partial"))
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
        let spawn_mode = TmuxSpawnMode::from_attach(attach);
        validate_spawn_start_cwd(spawn_mode, start_cwd.as_deref())?;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(initial_spawn_pty_size())
            .map_err(|e| anyhow::anyhow!("failed to open PTY: {}", e))?;

        let cmd = build_tmux_spawn_command(
            spawn_mode,
            &session_id,
            &tmux_name,
            start_cwd.as_deref(),
            initial_command.as_deref(),
        );

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("failed to spawn tmux: {}", e))?;
        inspect_tmux_child_after_spawn(spawn_mode, child.as_mut())?;

        // We intentionally drop the slave side -- the master side is what we use.
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("failed to take PTY writer: {}", e))?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCommand>(256);
        let (event_tx, _) = broadcast::channel::<ControlEvent>(64);

        let actor = build_spawned_session_actor(SpawnedSessionActorInit {
            session_id: session_id.clone(),
            tmux_name: tmux_name.clone(),
            config,
            master: pair.master,
            writer,
            cmd_rx,
            event_tx: event_tx.clone(),
            start_cwd,
            initial_tool,
            attach,
            last_activity_override,
            batch,
        });

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
        let reader = self.clone_pty_reader()?;

        tokio::task::spawn_blocking(move || {
            pty_read_loop(session_id_for_reader, reader, pty_tx);
        });

        Some(pty_rx)
    }

    fn clone_pty_reader(&self) -> Option<Box<dyn std::io::Read + Send>> {
        self.master
            .try_clone_reader()
            .map_err(|e| {
                error!(session_id = %self.session_id, "failed to clone PTY reader: {}", e);
                e
            })
            .ok()
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
            return self.closed_pty_write_input_result();
        }

        self.accept_write_input(&data);
        self.deliver_write_input(&data).await
    }

    fn closed_pty_write_input_result(&self) -> InputDeliveryResult {
        debug!(session_id = %self.session_id, "ignoring write to exited PTY");
        InputDeliveryResult::failed("none", "session process has exited")
    }

    fn accept_write_input(&mut self, data: &[u8]) {
        self.record_write_input_activity(data);
        self.update_last_skill_from_input(data);
    }

    fn record_write_input_activity(&mut self, data: &[u8]) {
        if write_input_counts_as_activity(data) {
            self.scroll_guard.notify_input();
            let state_before = self.state_detector.state();
            let evidence_before = self.state_detector.state_evidence();
            self.state_detector.note_input();
            let _ = self.maybe_emit_state_change(state_before, evidence_before);
        }
    }

    async fn deliver_write_input(&mut self, data: &[u8]) -> InputDeliveryResult {
        let session_id = self.session_id.clone();
        let tmux_name = self.tmux_name.clone();
        if let Some(result) = try_tmux_write_input(tmux_name.clone(), data).await {
            if let Some(delivery) = tmux_write_input_result(&session_id, &tmux_name, result) {
                return delivery;
            }
        }

        self.write_raw_input(data, "PTY write error")
    }

    fn write_raw_input(&mut self, data: &[u8], error_label: &'static str) -> InputDeliveryResult {
        match write_and_flush_input(&mut self.writer, data) {
            Ok(()) => InputDeliveryResult::delivered("pty_write"),
            Err(e) => {
                error!(session_id = %self.session_id, "{}: {}", error_label, e);
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

fn subscriber_cap_rejection(active_subscribers: usize) -> SubscribeRejection {
    SubscribeRejection {
        reason: format!("session already has {active_subscribers} active browser subscribers"),
    }
}

fn subscribe_outcome_for_rejection(rejection: SubscribeRejection) -> SubscribeOutcome {
    SubscribeOutcome::Rejected {
        reason: rejection.reason,
    }
}

fn retain_open_subscribers(subscribers: &mut HashMap<ClientId, mpsc::Sender<OutputFrame>>) {
    subscribers.retain(|_, tx| !tx.is_closed());
}

fn apply_subscriber_cap(
    active_subscribers: usize,
    max_subscribers: usize,
) -> Result<(), SubscribeRejection> {
    (active_subscribers < max_subscribers)
        .then_some(())
        .ok_or_else(|| subscriber_cap_rejection(active_subscribers))
}

fn attach_open_subscriber(
    subscribers: &mut HashMap<ClientId, mpsc::Sender<OutputFrame>>,
    client_id: ClientId,
    client_tx: mpsc::Sender<OutputFrame>,
) -> bool {
    let open = !client_tx.is_closed();
    if open {
        subscribers.insert(client_id, client_tx);
    }
    open
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BroadcastRemovalReason {
    Overloaded,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BroadcastSendOutcome {
    Delivered,
    Remove(BroadcastRemovalReason),
}

impl BroadcastSendOutcome {
    fn removal_reason(self) -> Option<BroadcastRemovalReason> {
        match self {
            Self::Delivered => None,
            Self::Remove(reason) => Some(reason),
        }
    }
}

fn classify_broadcast_send_error(
    err: mpsc::error::TrySendError<OutputFrame>,
) -> BroadcastSendOutcome {
    match err {
        mpsc::error::TrySendError::Full(_) => {
            BroadcastSendOutcome::Remove(BroadcastRemovalReason::Overloaded)
        }
        mpsc::error::TrySendError::Closed(_) => {
            BroadcastSendOutcome::Remove(BroadcastRemovalReason::Closed)
        }
    }
}

fn try_send_broadcast_frame(
    tx: &mpsc::Sender<OutputFrame>,
    frame: &OutputFrame,
) -> BroadcastSendOutcome {
    tx.try_send(frame.clone())
        .map(|()| BroadcastSendOutcome::Delivered)
        .unwrap_or_else(classify_broadcast_send_error)
}

fn broadcast_removal_for_subscriber(
    session_id: &str,
    client_id: ClientId,
    tx: &mpsc::Sender<OutputFrame>,
    frame: &OutputFrame,
) -> Option<ClientId> {
    let reason = try_send_broadcast_frame(tx, frame).removal_reason()?;
    note_broadcast_removal(session_id, client_id, reason);
    Some(client_id)
}

fn note_broadcast_removal(session_id: &str, client_id: ClientId, reason: BroadcastRemovalReason) {
    match reason {
        BroadcastRemovalReason::Overloaded => note_overloaded_subscriber(session_id, client_id),
        BroadcastRemovalReason::Closed => note_closed_subscriber(session_id, client_id),
    }
}

fn note_overloaded_subscriber(session_id: &str, client_id: ClientId) {
    warn!(
        session_id = %session_id,
        client_id,
        "subscriber channel full (SESSION_OVERLOADED), dropping client"
    );
    crate::metrics::increment_overload(session_id);
}

fn note_closed_subscriber(session_id: &str, client_id: ClientId) {
    debug!(session_id = %session_id, client_id, "subscriber channel closed");
}

fn remove_broadcast_subscribers(
    subscribers: &mut HashMap<ClientId, mpsc::Sender<OutputFrame>>,
    client_ids: Vec<ClientId>,
) {
    for id in client_ids {
        subscribers.remove(&id);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeadlineSleep {
    Pending,
    Ready,
    Sleep(Duration),
}

fn deadline_sleep(deadline: Option<Instant>) -> DeadlineSleep {
    deadline
        .map(|deadline| deadline_sleep_after(deadline, Instant::now()))
        .unwrap_or(DeadlineSleep::Pending)
}

fn deadline_sleep_after(deadline: Instant, now: Instant) -> DeadlineSleep {
    deadline
        .checked_duration_since(now)
        .filter(|duration| !duration.is_zero())
        .map_or(DeadlineSleep::Ready, DeadlineSleep::Sleep)
}

async fn sleep_deadline(deadline_sleep: DeadlineSleep) {
    match deadline_sleep {
        DeadlineSleep::Pending => {
            std::future::pending::<()>().await;
        }
        DeadlineSleep::Ready => {}
        DeadlineSleep::Sleep(duration) => {
            tokio::time::sleep(duration).await;
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
        sleep_deadline(deadline_sleep(deadline)).await;
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
        if !should_clear_startup_replay(
            self.clear_replay_on_first_idle,
            self.state_detector.state(),
        ) {
            return;
        }

        self.clear_replay_on_first_idle = false;
        self.replay_ring.clear();
        debug!(
            session_id = %self.session_id,
            "cleared replay ring on first idle (startup garbage removed)"
        );
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
        let outcome = self.reconcile_liveness_query(query_pane_liveness(&tmux_name).await);
        for refresh in outcome.refresh_actions() {
            self.apply_liveness_refresh(refresh).await;
        }
    }

    fn reconcile_liveness_query(
        &mut self,
        query_result: anyhow::Result<PaneLiveness>,
    ) -> LivenessReconciliation {
        match query_result {
            Ok(liveness) => self.reconcile_liveness(liveness),
            Err(e) => {
                self.log_liveness_query_error(e);
                LivenessReconciliation::default()
            }
        }
    }

    fn log_liveness_query_error(&self, error: anyhow::Error) {
        debug!(
            session_id = %self.session_id,
            tmux_name = %self.tmux_name,
            "liveness check failed: {}",
            error
        );
    }

    async fn apply_liveness_refresh(&mut self, refresh: LivenessRefresh) {
        match refresh {
            LivenessRefresh::Cwd => self.maybe_refresh_cwd_from_tmux(false).await,
            LivenessRefresh::Tool => self.maybe_refresh_tool_from_tmux(false).await,
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
        let detection = self.detect_session_state_change(previous_state, previous_evidence);
        if !detection.should_emit_event() {
            return None;
        }

        let changed_state = detection.changed_state();
        self.emit_session_state_payload(detection.into_payload(exit_reason));
        changed_state
    }

    fn detect_session_state_change(
        &self,
        previous_state: SessionState,
        previous_evidence: StateEvidence,
    ) -> StateChangeDetection {
        let (current_state, current_command) = self.state_detector.get_state();
        compare_session_state_change(
            previous_state,
            previous_evidence,
            current_state,
            current_command,
            self.state_detector.state_evidence(),
        )
    }

    fn emit_session_state_payload(&self, payload: SessionStatePayload) {
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
    }

    /// Send a frame to all subscribers. Detects overloaded subscribers whose
    /// channels are full, and removes them.
    async fn broadcast(&mut self, frame: OutputFrame) {
        let to_remove = self
            .subscribers
            .iter()
            .filter_map(|(&client_id, tx)| {
                broadcast_removal_for_subscriber(&self.session_id, client_id, tx, &frame)
            })
            .collect();
        remove_broadcast_subscribers(&mut self.subscribers, to_remove);
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

        match self.accept_subscribe(client_id, client_tx, resume_from_seq) {
            Ok(acceptance) => self.finish_subscribe(acceptance).await,
            Err(rejection) => subscribe_outcome_for_rejection(rejection),
        }
    }

    fn accept_subscribe(
        &mut self,
        client_id: ClientId,
        client_tx: mpsc::Sender<OutputFrame>,
        resume_from_seq: Option<u64>,
    ) -> Result<SubscribeAcceptance, SubscribeRejection> {
        retain_open_subscribers(&mut self.subscribers);
        self.check_subscriber_cap(client_id)?;
        Ok(SubscribeAcceptance {
            client_id,
            client_tx,
            replay_plan: self.replay_plan(resume_from_seq),
        })
    }

    fn check_subscriber_cap(&self, client_id: ClientId) -> Result<(), SubscribeRejection> {
        apply_subscriber_cap(self.subscribers.len(), MAX_OUTPUT_SUBSCRIBERS_PER_SESSION).map_err(
            |rejection| {
                warn!(
                    session_id = %self.session_id,
                    client_id,
                    subscribers = self.subscribers.len(),
                    "subscriber cap reached (SESSION_OVERLOADED), rejecting browser attach"
                );
                crate::metrics::increment_overload(&self.session_id);
                rejection
            },
        )
    }

    async fn finish_subscribe(&mut self, acceptance: SubscribeAcceptance) -> SubscribeOutcome {
        let SubscribeAcceptance {
            client_id,
            client_tx,
            replay_plan,
        } = acceptance;
        let outcome =
            replay_existing_frames(self.session_id.clone(), client_id, &client_tx, replay_plan)
                .await;
        self.attach_subscriber_after_replay(client_id, client_tx);
        outcome
    }

    fn attach_subscriber_after_replay(
        &mut self,
        client_id: ClientId,
        client_tx: mpsc::Sender<OutputFrame>,
    ) {
        if !attach_open_subscriber(&mut self.subscribers, client_id, client_tx) {
            debug!(
                session_id = %self.session_id,
                client_id,
                "subscriber dropped during subscribe ack; not attaching"
            );
        }
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

fn should_clear_startup_replay(clear_on_first_idle: bool, state: SessionState) -> bool {
    clear_on_first_idle && state == SessionState::Idle
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

#[derive(Debug, Clone, Copy, Default)]
struct LivenessReconciliation {
    refresh_cwd: bool,
    refresh_tool: bool,
}

impl LivenessReconciliation {
    fn refresh_actions(self) -> impl Iterator<Item = LivenessRefresh> {
        [
            (self.refresh_cwd, LivenessRefresh::Cwd),
            (self.refresh_tool, LivenessRefresh::Tool),
        ]
        .into_iter()
        .filter_map(|(enabled, refresh)| enabled.then_some(refresh))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivenessRefresh {
    Cwd,
    Tool,
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
    while pty_read_step(&session_id, reader.read(&mut buf), &buf, &tx).should_continue() {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtyReadLoopStep {
    Continue,
    Stop,
}

impl PtyReadLoopStep {
    fn should_continue(self) -> bool {
        self == Self::Continue
    }
}

fn pty_read_step(
    session_id: &str,
    read_result: std::io::Result<usize>,
    buf: &[u8],
    tx: &mpsc::Sender<Vec<u8>>,
) -> PtyReadLoopStep {
    match read_result {
        Ok(n) => pty_read_bytes_step(session_id, n, buf, tx),
        Err(err) => stop_after_pty_read_error(session_id, &err),
    }
}

fn pty_read_bytes_step(
    session_id: &str,
    n: usize,
    buf: &[u8],
    tx: &mpsc::Sender<Vec<u8>>,
) -> PtyReadLoopStep {
    if n == 0 {
        info!(session_id = %session_id, "PTY EOF");
        PtyReadLoopStep::Stop
    } else {
        send_pty_read_bytes(session_id, &buf[..n], tx)
    }
}

fn send_pty_read_bytes(
    session_id: &str,
    data: &[u8],
    tx: &mpsc::Sender<Vec<u8>>,
) -> PtyReadLoopStep {
    if tx.blocking_send(data.to_vec()).is_err() {
        debug!(session_id = %session_id, "PTY read loop: receiver dropped");
        PtyReadLoopStep::Stop
    } else {
        PtyReadLoopStep::Continue
    }
}

fn stop_after_pty_read_error(session_id: &str, err: &std::io::Error) -> PtyReadLoopStep {
    log_pty_read_error(session_id, err);
    PtyReadLoopStep::Stop
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtyReadErrorLog {
    LikelyChildExit,
    Error,
}

fn pty_read_error_log(err: &std::io::Error) -> PtyReadErrorLog {
    match err.kind() {
        std::io::ErrorKind::Other => PtyReadErrorLog::LikelyChildExit,
        _ => PtyReadErrorLog::Error,
    }
}

fn log_pty_read_error(session_id: &str, err: &std::io::Error) {
    match pty_read_error_log(err) {
        // EIO is expected when the child process exits.
        PtyReadErrorLog::LikelyChildExit => {
            info!(session_id = %session_id, "PTY read ended (likely child exit)");
        }
        PtyReadErrorLog::Error => {
            error!(session_id = %session_id, "PTY read error: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::process_tree::{
        process_entries_cache, ProcessEntriesCache, ProcessEntry, PROCESS_ENTRIES_CACHE_TTL,
    };
    use super::{build_tmux_spawn_command, build_tmux_spawn_command_args};
    use super::{
        capture_pane_tail_with_command, compare_session_state_change, cwd_from_osc7_payload,
        cwd_update, deadline_sleep, deadline_sleep_after, extract_cwd_from_title,
        find_osc_payload_end, initial_spawn_pty_size, normalize_submit_line_text,
        osc7_cwd_update_plan, osc_payloads, pty_read_error_log, pty_read_step,
        query_tmux_session_created, resolve_tmux_colorterm, resolve_tmux_term,
        resolve_tmux_terminal_env, run_bounded_tmux_command, should_clear_startup_replay,
        should_refresh_cwd_from_tmux, should_refresh_tool_from_tmux,
        state_detector_for_initial_tool, submit_line_fallback_input, subscriber_cap_rejection,
        title_cwd_update, title_tool_update, tmux_input_chunks, tool_refresh_changes_tool,
        validate_spawn_start_cwd, write_and_flush_input, write_input_counts_as_activity,
        ControlEvent, DeadlineSleep, LivenessReconciliation, LivenessRefresh, OutputFrame,
        PaneLiveness, PtyReadErrorLog, PtyReadLoopStep, SessionActor, SessionCommand,
        SubscribeOutcome, TmuxInputChunk, TmuxSpawnMode, CWD_REFRESH_MIN_INTERVAL,
        MAX_OUTPUT_SUBSCRIBERS_PER_SESSION, TOOL_REFRESH_MIN_INTERVAL,
    };
    use crate::config::Config;
    use crate::scroll::guard::ScrollGuard;
    use crate::session::replay_ring::ReplayRing;
    use crate::types::{SessionState, SessionStatePayload, StateEvidence, TransportHealth};
    use chrono::{TimeZone, Utc};
    use portable_pty::{native_pty_system, PtySize};
    use std::collections::HashMap;
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::sync::{broadcast, mpsc, oneshot};

    fn argv_strings(command: &portable_pty::CommandBuilder) -> Vec<String> {
        command
            .get_argv()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

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

    fn output_frame(seq: u64, data: &[u8]) -> OutputFrame {
        OutputFrame {
            seq,
            data: data.to_vec(),
        }
    }

    #[test]
    fn deadline_sleep_without_deadline_pends() {
        assert_eq!(deadline_sleep(None), DeadlineSleep::Pending);
    }

    #[test]
    fn deadline_sleep_after_ready_for_past_and_current_deadlines() {
        let now = Instant::now();

        assert_eq!(deadline_sleep_after(now, now), DeadlineSleep::Ready);
        assert_eq!(
            deadline_sleep_after(now - Duration::from_millis(1), now),
            DeadlineSleep::Ready
        );
    }

    #[test]
    fn deadline_sleep_after_preserves_positive_duration() {
        let now = Instant::now();
        let duration = Duration::from_millis(123);

        assert_eq!(
            deadline_sleep_after(now + duration, now),
            DeadlineSleep::Sleep(duration)
        );
    }

    #[tokio::test]
    async fn sleep_until_deadline_returns_immediately_for_past_deadline() {
        let past_deadline = Instant::now() - Duration::from_millis(1);

        tokio::time::timeout(
            Duration::from_millis(50),
            SessionActor::sleep_until_deadline(Some(past_deadline)),
        )
        .await
        .expect("past deadlines should return immediately");
    }

    #[tokio::test]
    async fn sleep_until_deadline_without_deadline_can_be_cancelled() {
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            SessionActor::sleep_until_deadline(None)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn sleep_until_deadline_future_wait_can_be_cancelled() {
        let future_deadline = Instant::now() + Duration::from_secs(60);

        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            SessionActor::sleep_until_deadline(Some(future_deadline)),
        )
        .await
        .is_err());
    }

    #[test]
    fn pty_read_step_forwards_exact_read_slice_and_continues() {
        let (tx, mut rx) = mpsc::channel(1);

        let step = pty_read_step("sess-test", Ok(3), b"abcdef", &tx);

        assert_eq!(step, PtyReadLoopStep::Continue);
        assert_eq!(rx.try_recv().expect("pty bytes"), b"abc".to_vec());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn pty_read_step_eof_stops_without_sending() {
        let (tx, mut rx) = mpsc::channel(1);

        let step = pty_read_step("sess-test", Ok(0), b"abcdef", &tx);

        assert_eq!(step, PtyReadLoopStep::Stop);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn pty_read_step_stops_when_receiver_dropped() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        let step = pty_read_step("sess-test", Ok(3), b"abcdef", &tx);

        assert_eq!(step, PtyReadLoopStep::Stop);
    }

    #[test]
    fn pty_read_step_stops_for_likely_child_exit_error() {
        let (tx, mut rx) = mpsc::channel(1);
        let err = io::Error::new(io::ErrorKind::Other, "child exited");

        let step = pty_read_step("sess-test", Err(err), b"abcdef", &tx);

        assert_eq!(step, PtyReadLoopStep::Stop);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn pty_read_step_stops_for_non_other_read_error() {
        let (tx, mut rx) = mpsc::channel(1);
        let err = io::Error::new(io::ErrorKind::Interrupted, "interrupted");

        let step = pty_read_step("sess-test", Err(err), b"abcdef", &tx);

        assert_eq!(step, PtyReadLoopStep::Stop);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn pty_read_error_log_classifies_other_as_likely_child_exit() {
        let err = io::Error::new(io::ErrorKind::Other, "child exited");

        assert_eq!(pty_read_error_log(&err), PtyReadErrorLog::LikelyChildExit);
    }

    #[test]
    fn pty_read_error_log_classifies_non_other_as_error() {
        let err = io::Error::new(io::ErrorKind::Interrupted, "interrupted");

        assert_eq!(pty_read_error_log(&err), PtyReadErrorLog::Error);
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

    fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
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
    fn spawn_initial_pty_size_matches_tmux_bootstrap_contract() {
        let size = initial_spawn_pty_size();

        assert_eq!(size.rows, 24);
        assert_eq!(size.cols, 80);
        assert_eq!(size.pixel_width, 0);
        assert_eq!(size.pixel_height, 0);
    }

    #[test]
    fn spawn_start_cwd_validation_only_applies_to_new_sessions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "contents").expect("file");
        let file = file.to_string_lossy().into_owned();

        let error =
            validate_spawn_start_cwd(TmuxSpawnMode::New, Some(&file)).expect_err("file rejected");
        assert_eq!(
            error.to_string(),
            format!("session cwd does not exist or is not a directory: {file}")
        );
        validate_spawn_start_cwd(TmuxSpawnMode::Attach, Some(&file))
            .expect("attach skips cwd validation");
        validate_spawn_start_cwd(
            TmuxSpawnMode::New,
            Some(dir.path().to_str().expect("utf8 path")),
        )
        .expect("directory accepted");
        validate_spawn_start_cwd(TmuxSpawnMode::New, None).expect("missing cwd accepted");
    }

    #[test]
    fn spawn_attach_command_targets_exact_tmux_session() {
        let command =
            build_tmux_spawn_command_args(TmuxSpawnMode::Attach, "demo.session", None, None);

        assert_eq!(
            argv_strings(&command),
            vec![
                "tmux".to_string(),
                "attach-session".to_string(),
                "-t".to_string(),
                crate::tmux_target::exact_session_target("demo.session"),
            ]
        );
    }

    #[test]
    fn spawn_new_session_command_preserves_optional_cwd_and_initial_command_order() {
        let command = build_tmux_spawn_command_args(
            TmuxSpawnMode::New,
            "demo.session",
            Some("/tmp/project"),
            Some("cargo test"),
        );

        assert_eq!(
            argv_strings(&command),
            vec![
                "tmux".to_string(),
                "new-session".to_string(),
                "-s".to_string(),
                "demo.session".to_string(),
                "-c".to_string(),
                "/tmp/project".to_string(),
                "cargo test".to_string(),
            ]
        );
    }

    #[test]
    fn spawn_new_session_command_omits_absent_optional_args() {
        let command = build_tmux_spawn_command_args(TmuxSpawnMode::New, "demo.session", None, None);

        assert_eq!(
            argv_strings(&command),
            vec![
                "tmux".to_string(),
                "new-session".to_string(),
                "-s".to_string(),
                "demo.session".to_string(),
            ]
        );
    }

    #[test]
    fn spawn_command_env_removes_nested_tmux_and_sets_terminal_defaults() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous_tmux = std::env::var_os("TMUX");
        let previous_tmux_pane = std::env::var_os("TMUX_PANE");
        let previous_term = std::env::var_os("TERM");
        let previous_colorterm = std::env::var_os("COLORTERM");
        std::env::set_var("TMUX", "nested");
        std::env::set_var("TMUX_PANE", "%1");
        std::env::set_var("TERM", "dumb");
        std::env::remove_var("COLORTERM");

        let command = build_tmux_spawn_command(
            TmuxSpawnMode::Attach,
            "sess-test",
            "demo.session",
            None,
            None,
        );

        assert_eq!(command.get_env("TMUX"), None);
        assert_eq!(command.get_env("TMUX_PANE"), None);
        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
        assert_eq!(
            command.get_env("COLORTERM"),
            Some(std::ffi::OsStr::new("truecolor"))
        );
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new("swimmers"))
        );

        restore_env_var("TMUX", previous_tmux);
        restore_env_var("TMUX_PANE", previous_tmux_pane);
        restore_env_var("TERM", previous_term);
        restore_env_var("COLORTERM", previous_colorterm);
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
    fn state_change_detection_distinguishes_noop_evidence_and_state_paths() {
        let previous_evidence = StateEvidence::unobserved("initial");

        let noop = compare_session_state_change(
            SessionState::Idle,
            previous_evidence.clone(),
            SessionState::Idle,
            None,
            previous_evidence.clone(),
        );
        assert!(!noop.should_emit_event());
        assert_eq!(noop.changed_state(), None);

        let evidence_only = compare_session_state_change(
            SessionState::Idle,
            previous_evidence.clone(),
            SessionState::Idle,
            None,
            StateEvidence::unobserved("osc133_prompt"),
        );
        assert!(evidence_only.should_emit_event());
        assert_eq!(evidence_only.changed_state(), None);

        let state_transition = compare_session_state_change(
            SessionState::Idle,
            previous_evidence,
            SessionState::Busy,
            Some("cargo test".to_string()),
            StateEvidence::unobserved("local_input"),
        );
        assert!(state_transition.should_emit_event());
        assert_eq!(state_transition.changed_state(), Some(SessionState::Busy));
    }

    #[test]
    fn state_change_payload_preserves_exit_reason_and_transport_health() {
        let detection = compare_session_state_change(
            SessionState::Busy,
            StateEvidence::unobserved("local_input"),
            SessionState::Exited,
            None,
            StateEvidence::unobserved("process_exit"),
        );

        let payload = detection.into_payload(Some("process_exit".to_string()));

        assert_eq!(payload.state, SessionState::Exited);
        assert_eq!(payload.previous_state, SessionState::Busy);
        assert_eq!(payload.state_evidence.cause, "process_exit");
        assert_eq!(payload.transport_health, TransportHealth::Healthy);
        assert_eq!(payload.exit_reason.as_deref(), Some("process_exit"));
    }

    #[test]
    fn state_change_event_with_exit_reason_preserves_payload_fields() {
        let mut actor = test_actor();
        let mut rx = actor.event_tx.subscribe();
        let previous_state = actor.state_detector.state();
        let previous_evidence = actor.state_detector.state_evidence();

        actor.state_detector.mark_exited();
        let result = actor.maybe_emit_state_change_with_exit_reason(
            previous_state,
            previous_evidence,
            Some("process_exit".to_string()),
        );

        assert_eq!(result, Some(SessionState::Exited));
        let event = rx.try_recv().expect("session_state event");
        assert_eq!(event.event, "session_state");
        assert_eq!(event.session_id, "sess-test");
        let payload: SessionStatePayload =
            serde_json::from_value(event.payload).expect("session_state payload");
        assert_eq!(payload.state, SessionState::Exited);
        assert_eq!(payload.previous_state, SessionState::Idle);
        assert_eq!(payload.transport_health, TransportHealth::Healthy);
        assert_eq!(payload.exit_reason.as_deref(), Some("process_exit"));
    }

    #[test]
    fn state_change_event_returns_transition_with_no_receivers() {
        let mut actor = test_actor();
        let previous_state = actor.state_detector.state();
        let previous_evidence = actor.state_detector.state_evidence();

        actor.state_detector.mark_exited();
        let result = actor.maybe_emit_state_change_with_exit_reason(
            previous_state,
            previous_evidence,
            Some("process_exit".to_string()),
        );

        assert_eq!(result, Some(SessionState::Exited));
    }

    #[test]
    fn liveness_refresh_actions_preserve_cwd_then_tool_order() {
        let actions: Vec<_> = LivenessReconciliation {
            refresh_cwd: true,
            refresh_tool: true,
        }
        .refresh_actions()
        .collect();

        assert_eq!(actions, vec![LivenessRefresh::Cwd, LivenessRefresh::Tool]);
    }

    #[test]
    fn liveness_refresh_actions_skip_disabled_refreshes() {
        let no_actions: Vec<_> = LivenessReconciliation::default()
            .refresh_actions()
            .collect();
        assert!(no_actions.is_empty());

        let tool_only: Vec<_> = LivenessReconciliation {
            refresh_cwd: false,
            refresh_tool: true,
        }
        .refresh_actions()
        .collect();
        assert_eq!(tool_only, vec![LivenessRefresh::Tool]);
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
    async fn broadcast_delivers_frame_to_active_subscribers() {
        let mut actor = test_actor();
        let (client_one_tx, mut client_one_rx) = mpsc::channel(1);
        let (client_two_tx, mut client_two_rx) = mpsc::channel(1);
        actor.subscribers.insert(11, client_one_tx);
        actor.subscribers.insert(22, client_two_tx);

        actor.broadcast(output_frame(7, b"hello")).await;

        let client_one_frame = client_one_rx.try_recv().expect("client one frame");
        assert_eq!(client_one_frame.seq, 7);
        assert_eq!(client_one_frame.data, b"hello".to_vec());
        let client_two_frame = client_two_rx.try_recv().expect("client two frame");
        assert_eq!(client_two_frame.seq, 7);
        assert_eq!(client_two_frame.data, b"hello".to_vec());
        assert_eq!(actor.subscribers.len(), 2);
    }

    #[tokio::test]
    async fn broadcast_removes_full_subscriber_without_replacing_queued_frame() {
        let mut actor = test_actor();
        let (client_tx, mut client_rx) = mpsc::channel(1);
        client_tx
            .try_send(output_frame(1, b"queued"))
            .expect("prefill subscriber channel");
        actor.subscribers.insert(33, client_tx);

        actor.broadcast(output_frame(2, b"new")).await;

        assert!(!actor.subscribers.contains_key(&33));
        let queued_frame = client_rx.try_recv().expect("queued frame");
        assert_eq!(queued_frame.seq, 1);
        assert_eq!(queued_frame.data, b"queued".to_vec());
        assert!(client_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn broadcast_removes_closed_subscriber() {
        let mut actor = test_actor();
        let (client_tx, client_rx) = mpsc::channel(1);
        drop(client_rx);
        actor.subscribers.insert(44, client_tx);

        actor.broadcast(output_frame(3, b"closed")).await;

        assert!(!actor.subscribers.contains_key(&44));
    }

    #[tokio::test]
    async fn handle_subscribe_replays_requested_frames_before_attaching_client() {
        let mut actor = test_actor();
        let first_seq = actor.replay_ring.push(b"first");
        let second_seq = actor.replay_ring.push(b"second");
        let (client_tx, mut client_rx) = mpsc::channel(4);

        let outcome = actor
            .handle_subscribe(55, client_tx, Some(first_seq.saturating_sub(1)))
            .await;

        assert!(matches!(outcome, SubscribeOutcome::Ok));
        assert!(actor.subscribers.contains_key(&55));
        let first = client_rx.try_recv().expect("first replay frame");
        assert_eq!(first.seq, first_seq);
        assert_eq!(first.data, b"first".to_vec());
        let second = client_rx.try_recv().expect("second replay frame");
        assert_eq!(second.seq, second_seq);
        assert_eq!(second.data, b"second".to_vec());
    }

    #[tokio::test]
    async fn handle_subscribe_prunes_closed_subscribers_before_cap_check() {
        let mut actor = test_actor();
        for client_id in 0..MAX_OUTPUT_SUBSCRIBERS_PER_SESSION as u64 {
            let (client_tx, client_rx) = mpsc::channel(1);
            drop(client_rx);
            actor.subscribers.insert(client_id, client_tx);
        }
        let (client_tx, _client_rx) = mpsc::channel(1);

        let outcome = actor.handle_subscribe(99, client_tx, None).await;

        assert!(matches!(outcome, SubscribeOutcome::Ok));
        assert_eq!(actor.subscribers.len(), 1);
        assert!(actor.subscribers.contains_key(&99));
    }

    #[tokio::test]
    async fn handle_subscribe_rejects_when_open_subscriber_cap_is_reached() {
        let mut actor = test_actor();
        let mut receivers = Vec::new();
        for client_id in 0..MAX_OUTPUT_SUBSCRIBERS_PER_SESSION as u64 {
            let (client_tx, client_rx) = mpsc::channel(1);
            receivers.push(client_rx);
            actor.subscribers.insert(client_id, client_tx);
        }
        let (client_tx, _client_rx) = mpsc::channel(1);

        let outcome = actor.handle_subscribe(100, client_tx, None).await;

        match outcome {
            SubscribeOutcome::Rejected { reason } => {
                assert_eq!(
                    reason,
                    subscriber_cap_rejection(MAX_OUTPUT_SUBSCRIBERS_PER_SESSION).reason
                );
            }
            _ => panic!("expected subscriber cap rejection"),
        }
        assert_eq!(actor.subscribers.len(), MAX_OUTPUT_SUBSCRIBERS_PER_SESSION);
    }

    #[tokio::test]
    async fn handle_subscribe_does_not_attach_client_that_drops_during_replay() {
        let mut actor = test_actor();
        actor.replay_ring.push(b"first");
        let (client_tx, client_rx) = mpsc::channel(1);
        drop(client_rx);

        let outcome = actor.handle_subscribe(66, client_tx, Some(0)).await;

        assert!(matches!(outcome, SubscribeOutcome::Ok));
        assert!(!actor.subscribers.contains_key(&66));
    }

    #[test]
    fn cwd_update_trims_rejects_empty_and_skips_unchanged_paths() {
        assert_eq!(
            cwd_update("/tmp/project", " /tmp/other "),
            Some("/tmp/other".to_string())
        );
        assert_eq!(cwd_update("/tmp/project", "   "), None);
        assert_eq!(cwd_update("/tmp/project", "/tmp/project"), None);
    }

    #[test]
    fn osc7_cwd_update_plan_preserves_payload_order_and_update_semantics() {
        let text = concat!(
            "\x1b]7;file://host/tmp/project\x07",
            "\x1b]7;file://host/tmp/one\x07",
            "\x1b]7;http://host/tmp/ignored\x07",
            "\x1b]7;\x07",
            "\x1b]7;file://host/tmp/one\x07",
            "\x1b]7;file://host/tmp/two\x1b\\",
            "\x1b]7;file://host/tmp/one\x07",
        );

        assert_eq!(
            osc7_cwd_update_plan("/tmp/project", text),
            vec![
                "/tmp/one".to_string(),
                "/tmp/two".to_string(),
                "/tmp/one".to_string()
            ]
        );
    }

    #[test]
    fn apply_osc7_payloads_updates_cwd_and_emits_events_in_order() {
        let mut actor = test_actor();
        let mut rx = actor.event_tx.subscribe();

        actor.apply_osc7_payloads(concat!(
            "\x1b]7;file://host/tmp/project\x07",
            "\x1b]7;file://host/tmp/one\x07",
            "\x1b]7;not-file-uri\x07",
            "\x1b]7;file://host/tmp/one\x07",
            "\x1b]7;file://host/tmp/two\x1b\\",
        ));

        assert_eq!(actor.cwd, "/tmp/two");
        for expected_title in ["/tmp/one", "/tmp/two"] {
            let event = rx.try_recv().expect("cwd title event");
            assert_eq!(event.event, "session_title");
            assert_eq!(event.session_id, "sess-test");
            let payload: crate::types::SessionTitlePayload =
                serde_json::from_value(event.payload).expect("session title payload");
            assert_eq!(payload.title, expected_title);
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn update_cwd_and_emit_only_emits_when_cwd_changes() {
        let mut actor = test_actor();
        let mut rx = actor.event_tx.subscribe();

        actor.update_cwd_and_emit(" /tmp/project ".to_string());
        actor.update_cwd_and_emit("   ".to_string());
        assert!(rx.try_recv().is_err());

        actor.update_cwd_and_emit(" /tmp/other ".to_string());

        assert_eq!(actor.cwd, "/tmp/other");
        let event = rx.try_recv().expect("cwd title event");
        assert_eq!(event.event, "session_title");
        assert_eq!(event.session_id, "sess-test");
        let payload: crate::types::SessionTitlePayload =
            serde_json::from_value(event.payload).expect("session title payload");
        assert_eq!(payload.title, "/tmp/other");
    }

    #[test]
    fn title_cwd_update_only_extracts_when_current_cwd_is_empty() {
        assert_eq!(
            title_cwd_update("", "user@host:/tmp/project"),
            Some("/tmp/project".to_string())
        );
        assert_eq!(
            title_cwd_update("/already/set", "user@host:/tmp/project"),
            None
        );
        assert_eq!(title_cwd_update("", "plain-title"), None);
    }

    #[test]
    fn update_cwd_from_title_preserves_existing_cwd_and_fills_empty_cwd() {
        let mut actor = test_actor();

        actor.update_cwd_from_title("user@host:/tmp/ignored");
        assert_eq!(actor.cwd, "/tmp/project");

        actor.cwd.clear();
        actor.update_cwd_from_title("user@host:/tmp/from-title");
        assert_eq!(actor.cwd, "/tmp/from-title");
    }

    #[test]
    fn title_tool_update_only_detects_when_tool_is_missing() {
        assert_eq!(
            title_tool_update(None, "codex - swimmers"),
            Some("Codex".to_string())
        );
        assert_eq!(title_tool_update(Some("Codex"), "claude"), None);
        assert_eq!(title_tool_update(None, "plain shell"), None);
    }

    #[test]
    fn update_tool_from_title_sets_tool_mode_once_for_missing_tool() {
        let mut actor = test_actor();
        actor.tool = None;

        actor.update_tool_from_title("claude code");

        assert_eq!(actor.tool.as_deref(), Some("Claude Code"));
        actor.state_detector.note_input();
        assert_eq!(actor.state_detector.state(), SessionState::Busy);

        actor.update_tool_from_title("codex");
        assert_eq!(actor.tool.as_deref(), Some("Claude Code"));
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
    fn extract_cwd_from_title_ignores_blank_host_prefixed_paths_and_plain_titles() {
        assert_eq!(extract_cwd_from_title("user@host:"), None);
        assert_eq!(extract_cwd_from_title("user@host: "), None);
        assert_eq!(extract_cwd_from_title("plain-title"), None);
        assert_eq!(extract_cwd_from_title("build finished: ok"), None);
    }

    #[test]
    fn extract_cwd_from_title_preserves_home_when_home_is_absent() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous_home = std::env::var_os("HOME");
        std::env::remove_var("HOME");

        assert_eq!(extract_cwd_from_title("~/repo"), Some("~/repo".to_string()));
        assert_eq!(extract_cwd_from_title("~"), Some("~".to_string()));

        if let Some(value) = previous_home {
            std::env::set_var("HOME", value);
        }
    }

    #[test]
    fn extract_cwd_from_title_ignores_invalid_prefix_shapes() {
        assert_eq!(extract_cwd_from_title("user@host:relative/path"), None);
        assert_eq!(extract_cwd_from_title("user@host: ./project"), None);
        assert_eq!(extract_cwd_from_title("user@host /tmp/project"), None);
    }

    #[test]
    fn osc_payload_helpers_extract_bel_and_st_terminated_sequences() {
        let text = "\x1b]7;file://host/tmp/project\x1b\\ middle \x1b]2;codex\x07";
        assert_eq!(find_osc_payload_end("title\x07tail"), Some((5, 1)));
        assert_eq!(find_osc_payload_end("title\x1b\\tail"), Some((5, 2)));
        assert_eq!(
            find_osc_payload_end("title\x07before-st\x1b\\tail"),
            Some((5, 1))
        );
        assert_eq!(
            find_osc_payload_end("title\x1b\\before-bel\x07tail"),
            Some((5, 2))
        );
        assert_eq!(find_osc_payload_end("unterminated title"), None);
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
    fn startup_replay_clears_once_after_first_idle() {
        let mut actor = test_actor();
        actor.clear_replay_on_first_idle = true;
        actor.state_detector.note_input();
        actor.replay_ring.push(b"startup noise");

        assert!(!should_clear_startup_replay(
            true,
            actor.state_detector.state()
        ));
        assert_eq!(actor.state_detector.state(), SessionState::Busy);

        actor.clear_startup_replay_if_idle();
        assert!(actor.clear_replay_on_first_idle);
        assert_eq!(actor.replay_ring.snapshot(), "startup noise");

        actor.state_detector.process_output(b"\x1b]133;A\x07");
        actor.clear_startup_replay_if_idle();

        assert!(!actor.clear_replay_on_first_idle);
        assert_eq!(actor.replay_ring.snapshot(), "");

        actor.replay_ring.push(b"real output");
        actor.clear_startup_replay_if_idle();
        assert_eq!(actor.replay_ring.snapshot(), "real output");
    }

    #[test]
    fn startup_replay_clear_predicate_requires_flag_and_idle_state() {
        assert!(should_clear_startup_replay(true, SessionState::Idle));
        assert!(!should_clear_startup_replay(false, SessionState::Idle));
        assert!(!should_clear_startup_replay(true, SessionState::Busy));
        assert!(!should_clear_startup_replay(true, SessionState::Exited));
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

    #[test]
    fn tmux_tool_refresh_result_applies_only_detected_changes() {
        let mut actor = test_actor();

        actor.apply_tmux_tool_refresh_result("demo", Ok(None));
        assert_eq!(actor.tool.as_deref(), Some("Codex"));

        assert!(!tool_refresh_changes_tool(Some("Codex"), "Codex"));
        actor.apply_tmux_tool_refresh_result("demo", Ok(Some("Codex".to_string())));
        assert_eq!(actor.tool.as_deref(), Some("Codex"));

        assert!(tool_refresh_changes_tool(Some("Codex"), "Claude Code"));
        actor.apply_tmux_tool_refresh_result("demo", Ok(Some("Claude Code".to_string())));
        assert_eq!(actor.tool.as_deref(), Some("Claude Code"));

        actor.apply_tmux_tool_refresh_result("demo", Err(anyhow::anyhow!("tmux failed")));
        assert_eq!(actor.tool.as_deref(), Some("Claude Code"));
    }

    #[test]
    fn actor_tool_refresh_predicate_uses_current_actor_state() {
        let mut actor = test_actor();
        let now = Instant::now();
        actor.last_tool_refresh_at = now - TOOL_REFRESH_MIN_INTERVAL;

        assert!(!actor.should_refresh_tool_from_tmux_at(false, now));

        actor.state_detector.note_input();
        assert!(actor.should_refresh_tool_from_tmux_at(false, now));
        assert!(actor.should_refresh_tool_from_tmux_at(true, now));
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

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
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

        let result = actor.handle_write_input(b"hello\r".to_vec(), true).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(!result.delivered);
        assert_eq!(result.method, "none");
        assert_eq!(
            result.message.as_deref(),
            Some("session process has exited")
        );
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

        let result = actor.handle_write_input(b"hello\r".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(result.delivered);
        assert_eq!(result.method, "tmux_send_keys");
        assert_eq!(result.message, None);
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

        let result = actor.handle_write_input(b"hello\r".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(result.delivered);
        assert_eq!(result.method, "pty_write");
        assert_eq!(result.message, None);
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

        let result = actor.handle_write_input(b"hello\r".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(result.delivered);
        assert_eq!(result.method, "tmux_send_keys_partial");
        assert_eq!(result.message, None);
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

        let result = actor.handle_write_input(b"abc\t".to_vec(), false).await;

        let writer_state = writer_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(result.delivered);
        assert_eq!(result.method, "pty_write");
        assert_eq!(result.message, None);
        assert_eq!(writer_state.writes, b"abc\t");
        assert_eq!(writer_state.flushes, 1);
        assert_eq!(actor.state_detector.state(), SessionState::Busy);
    }

    #[tokio::test]
    async fn handle_write_input_reports_raw_writer_errors_as_pty_write() {
        let mut actor = test_actor();
        actor.writer = Box::new(FailingWriter);

        let result = actor.handle_write_input(b"abc\t".to_vec(), false).await;

        assert!(!result.delivered);
        assert_eq!(result.method, "pty_write");
        assert_eq!(result.message.as_deref(), Some("writer failed"));
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
            resolve_tmux_terminal_env(Some("  screen-256color  "), Some("truecolor"));
        assert_eq!(term, "screen-256color");
        assert_eq!(colorterm, "truecolor");
        assert!(!fallback);
    }

    #[test]
    fn resolve_tmux_term_falls_back_for_unknown_and_blank_values() {
        for inherited_term in [Some("unknown"), Some("  UNKNOWN  "), Some("   ")] {
            let (term, fallback) = resolve_tmux_term(inherited_term);
            assert_eq!(term, "xterm-256color");
            assert!(fallback);
        }
    }

    #[test]
    fn resolve_tmux_colorterm_trims_or_uses_default() {
        assert_eq!(resolve_tmux_colorterm(Some("  truecolor  ")), "truecolor");

        for inherited_colorterm in [None, Some(""), Some("   ")] {
            assert_eq!(resolve_tmux_colorterm(inherited_colorterm), "truecolor");
        }
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
}
