use std::collections::HashMap;
use std::ffi::OsStr;
use std::process::Output;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
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
use crate::tmux_target::{exact_pane_target, TmuxTarget};
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
mod subscribers;
mod tmux_input;

use self::activity::output_counts_as_meaningful_activity;
#[cfg(test)]
use self::metadata::{
    cwd_from_osc7_payload, cwd_from_osc7_payload_with_local_hosts, cwd_update,
    extract_cwd_from_title, find_osc_payload_end, osc7_cwd_update_plan, osc_payloads,
    should_refresh_cwd_from_tmux, should_refresh_tool_from_tmux, title_cwd_update,
    title_tool_update, tool_refresh_changes_tool, CWD_REFRESH_MIN_INTERVAL,
    TOOL_REFRESH_MIN_INTERVAL,
};
use self::metadata::{query_tmux_pane_metadata, state_detector_for_initial_tool, TmuxPaneMetadata};
use self::process_tree::query_pane_liveness_for_pid;
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
use self::subscribers::subscriber_cap_rejection;
use self::subscribers::{
    apply_subscriber_cap, attach_open_subscriber, broadcast_removal_for_subscriber,
    remove_broadcast_subscribers, replay_existing_frames, retain_open_subscribers,
    subscribe_outcome_for_rejection,
};

#[cfg(test)]
use self::tmux_input::TmuxInputChunk;
use self::tmux_input::{
    normalize_submit_line_text, send_tmux_input_chunks, send_tmux_submit_line,
    submit_line_fallback_input, tmux_input_chunks, write_and_flush_input,
    write_input_counts_as_activity, TmuxInputSendError,
};

const LIVENESS_CHECK_INTERVAL: Duration = Duration::from_millis(5_000);
const TMUX_PANE_METADATA_CACHE_TTL: Duration = Duration::from_millis(250);
const TMUX_CAPTURE_PANE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OUTPUT_SUBSCRIBERS_PER_SESSION: usize = 16;
const TMUX_PROBE_COOLDOWN_BASE: Duration = Duration::from_secs(30);
const TMUX_PROBE_COOLDOWN_MAX: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TmuxCommandRole {
    Essential,
    NonessentialProbe,
}

#[derive(Debug, Clone)]
struct TmuxTargetHealth {
    cooldown_until: Option<Instant>,
    consecutive_timeouts: u32,
    skipped_probes: u64,
}

#[derive(Debug, Clone)]
struct CachedTmuxPaneMetadata {
    fetched_at: Instant,
    metadata: TmuxPaneMetadata,
}

impl TmuxTargetHealth {
    fn new() -> Self {
        Self {
            cooldown_until: None,
            consecutive_timeouts: 0,
            skipped_probes: 0,
        }
    }

    fn cooldown_remaining_at(&self, now: Instant) -> Option<Duration> {
        self.cooldown_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
    }
}

impl Default for TmuxTargetHealth {
    fn default() -> Self {
        Self::new()
    }
}

static TMUX_HEALTH: OnceLock<Mutex<HashMap<TmuxTarget, TmuxTargetHealth>>> = OnceLock::new();

fn tmux_health() -> &'static Mutex<HashMap<TmuxTarget, TmuxTargetHealth>> {
    TMUX_HEALTH.get_or_init(|| Mutex::new(HashMap::new()))
}

fn tmux_probe_cooldown_duration(consecutive_timeouts: u32) -> Duration {
    let multiplier = consecutive_timeouts.clamp(1, 4);
    TMUX_PROBE_COOLDOWN_BASE
        .saturating_mul(multiplier)
        .min(TMUX_PROBE_COOLDOWN_MAX)
}

fn should_skip_tmux_command(
    target: &TmuxTarget,
    role: TmuxCommandRole,
    operation: &'static str,
) -> anyhow::Result<()> {
    if role == TmuxCommandRole::Essential {
        return Ok(());
    }

    let now = Instant::now();
    let mut health = tmux_health()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(state) = health.get_mut(target) else {
        return Ok(());
    };
    let Some(remaining) = state.cooldown_remaining_at(now) else {
        return Ok(());
    };

    state.skipped_probes = state.skipped_probes.saturating_add(1);
    let skipped_probe_count = state.skipped_probes;
    warn!(
        operation,
        tmux_target = %target.display_label(),
        cooldown_remaining_ms = remaining.as_millis() as u64,
        skipped_probe_count,
        "tmux nonessential probe skipped during circuit-breaker cooldown"
    );
    Err(anyhow::anyhow!(
        "tmux {operation} skipped while tmux target {} is cooling down for {}ms",
        target.display_label(),
        remaining.as_millis()
    ))
}

fn record_tmux_command_timeout(target: &TmuxTarget, operation: &'static str) {
    let now = Instant::now();
    let mut health = tmux_health()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let state = health.entry(target.clone()).or_default();
    state.consecutive_timeouts = state.consecutive_timeouts.saturating_add(1);
    let cooldown = tmux_probe_cooldown_duration(state.consecutive_timeouts);
    state.cooldown_until = Some(now + cooldown);
    warn!(
        operation,
        tmux_target = %target.display_label(),
        timeout_count = state.consecutive_timeouts,
        cooldown_ms = cooldown.as_millis() as u64,
        skipped_probe_count = state.skipped_probes,
        "tmux circuit breaker opened after command timeout"
    );
}

fn record_tmux_command_success(target: &TmuxTarget, operation: &'static str) {
    let mut health = tmux_health()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(state) = health.get(target) else {
        return;
    };
    if state.cooldown_until.is_none()
        && state.consecutive_timeouts == 0
        && state.skipped_probes == 0
    {
        return;
    }
    let skipped_probe_count = state.skipped_probes;
    health.remove(target);
    info!(
        operation,
        tmux_target = %target.display_label(),
        skipped_probe_count,
        "tmux circuit breaker recovered after successful command"
    );
}

#[cfg(test)]
pub(crate) fn reset_tmux_health_for_tests() {
    let mut health = tmux_health()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    health.clear();
}

fn tmux_command(program: impl AsRef<OsStr>, args: &[String]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args.iter().map(String::as_str))
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
    run_bounded_tmux_command_for_target(
        program,
        &TmuxTarget::Default,
        args,
        timeout_duration,
        operation,
    )
    .await
}

pub(crate) async fn run_bounded_tmux_command_for_target(
    program: impl AsRef<OsStr>,
    target: &TmuxTarget,
    args: &[&str],
    timeout_duration: Duration,
    operation: &'static str,
) -> anyhow::Result<Output> {
    run_bounded_tmux_command_for_target_with_role(
        program,
        target,
        args,
        timeout_duration,
        operation,
        TmuxCommandRole::Essential,
    )
    .await
}

pub(crate) async fn run_bounded_tmux_probe_for_target(
    program: impl AsRef<OsStr>,
    target: &TmuxTarget,
    args: &[&str],
    timeout_duration: Duration,
    operation: &'static str,
) -> anyhow::Result<Output> {
    run_bounded_tmux_command_for_target_with_role(
        program,
        target,
        args,
        timeout_duration,
        operation,
        TmuxCommandRole::NonessentialProbe,
    )
    .await
}

async fn run_bounded_tmux_command_for_target_with_role(
    program: impl AsRef<OsStr>,
    target: &TmuxTarget,
    args: &[&str],
    timeout_duration: Duration,
    operation: &'static str,
    role: TmuxCommandRole,
) -> anyhow::Result<Output> {
    should_skip_tmux_command(target, role, operation)?;
    let started = Instant::now();
    let args = target.command_args(args);
    let mut command = tmux_command(program, &args);
    match tokio::time::timeout(timeout_duration, command.output()).await {
        Ok(Ok(output)) => {
            log_bounded_tmux_command_elapsed(
                operation,
                started.elapsed(),
                timeout_duration,
                Some(output.status.success()),
            );
            record_tmux_command_success(target, operation);
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
            record_tmux_command_timeout(target, operation);
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
    pub tmux_target: TmuxTarget,
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
            tmux_target: TmuxTarget::Default,
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
    tmux_target: TmuxTarget,
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

    // Short-lived tmux pane metadata snapshot for refreshes in the same actor tick.
    tmux_pane_metadata_cache: Option<CachedTmuxPaneMetadata>,

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
    tmux_target: TmuxTarget,
    data: &[u8],
) -> Option<Result<(), TmuxInputSendError>> {
    let chunks = tmux_input_chunks(data)?;
    Some(send_tmux_input_chunks(&tmux_name, &tmux_target, &chunks).await)
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
        tmux_target: TmuxTarget,
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
        tmux_target.validate()?;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(initial_spawn_pty_size())
            .map_err(|e| anyhow::anyhow!("failed to open PTY: {}", e))?;

        let cmd = build_tmux_spawn_command(
            spawn_mode,
            &tmux_target,
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
            tmux_target: tmux_target.clone(),
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
            tmux_target,
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
                    self.tmux_target.clone(),
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
        let tmux_target = self.tmux_target.clone();
        if let Some(result) = try_tmux_write_input(tmux_name.clone(), tmux_target, data).await {
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

        match send_tmux_submit_line(&self.tmux_name, &self.tmux_target, &text).await {
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

async fn capture_pane_tail_or_empty(
    session_id: String,
    tmux_name: String,
    tmux_target: TmuxTarget,
    lines: usize,
) -> String {
    match capture_pane_tail(&tmux_name, &tmux_target, lines).await {
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

    async fn query_tmux_pane_metadata_cached(&mut self) -> anyhow::Result<TmuxPaneMetadata> {
        let now = Instant::now();
        if let Some(cache) = self.tmux_pane_metadata_cache.as_ref() {
            if now.duration_since(cache.fetched_at) <= TMUX_PANE_METADATA_CACHE_TTL {
                return Ok(cache.metadata.clone());
            }
        }

        let metadata = query_tmux_pane_metadata(&self.tmux_name, &self.tmux_target).await?;
        self.tmux_pane_metadata_cache = Some(CachedTmuxPaneMetadata {
            fetched_at: Instant::now(),
            metadata: metadata.clone(),
        });
        Ok(metadata)
    }

    async fn maybe_refresh_session_started_at(&mut self) {
        match self
            .query_tmux_pane_metadata_cached()
            .await
            .and_then(|metadata| metadata.session_created())
        {
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
    /// Runs every LIVENESS_CHECK_INTERVAL. Skips if the session has exited.
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
        let outcome = match self.query_tmux_pane_metadata_cached().await {
            Ok(metadata) => {
                let query_result = match metadata.pane_pid() {
                    Ok(pane_pid) => query_pane_liveness_for_pid(pane_pid).await,
                    Err(error) => Err(error),
                };
                self.reconcile_liveness_query(query_result)
            }
            Err(error) => self.reconcile_liveness_query(Err(error)),
        };
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
        apply_subscriber_cap(self.subscribers.len(), MAX_OUTPUT_SUBSCRIBERS_PER_SESSION)
            .inspect_err(|_rejection| {
                warn!(
                    session_id = %self.session_id,
                    client_id,
                    subscribers = self.subscribers.len(),
                    "subscriber cap reached (SESSION_OVERLOADED), rejecting browser attach"
                );
                crate::metrics::increment_overload(&self.session_id);
            })
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

        let tmux_target = self.tmux_target.clone();
        let screen_text = match capture_pane_tail(&tmux_name, &tmux_target, 300).await {
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
        summary.tmux_target = self.tmux_target.clone();
        summary.last_skill = self.last_skill.clone();
        summary.batch = self.batch.clone();
        summary
    }
}

/// Capture visible pane text directly from tmux.
async fn capture_pane_tail(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
    lines: usize,
) -> anyhow::Result<String> {
    capture_pane_tail_with_command("tmux", tmux_name, tmux_target, lines).await
}

async fn capture_pane_tail_with_command(
    tmux_command: impl AsRef<std::ffi::OsStr>,
    tmux_name: &str,
    tmux_target: &TmuxTarget,
    lines: usize,
) -> anyhow::Result<String> {
    let lines = lines.clamp(20, 1000);
    let start = format!("-{lines}");
    let target = exact_pane_target(tmux_name);

    let output = run_bounded_tmux_probe_for_target(
        tmux_command,
        tmux_target,
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

#[cfg(test)]
async fn query_tmux_session_created(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
) -> anyhow::Result<chrono::DateTime<Utc>> {
    query_tmux_pane_metadata(tmux_name, tmux_target)
        .await?
        .session_created()
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
mod tests;
