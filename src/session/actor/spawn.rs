use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

use crate::config::Config;
use crate::scroll::guard::ScrollGuard;
use crate::session::replay_ring::ReplayRing;
use crate::tmux_target::{exact_session_target, TmuxTarget};
use crate::types::{ControlEvent, SessionBatchMembership};

use super::{state_detector_for_initial_tool, SessionActor, SessionCommand};

const TMUX_NEW_SESSION_EXIT_GRACE: Duration = Duration::from_millis(50);
const TMUX_FALLBACK_TERM: &str = "xterm-256color";
const TMUX_FALLBACK_COLORTERM: &str = "truecolor";
const TMUX_UNSUPPORTED_TERMS: [&str; 3] = ["", "dumb", "unknown"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TmuxSpawnMode {
    Attach,
    New,
}

impl TmuxSpawnMode {
    pub(super) fn from_attach(attach: bool) -> Self {
        if attach {
            Self::Attach
        } else {
            Self::New
        }
    }

    fn is_new(self) -> bool {
        matches!(self, Self::New)
    }
}

pub(super) fn initial_spawn_pty_size() -> PtySize {
    PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub(super) fn validate_spawn_start_cwd(
    mode: TmuxSpawnMode,
    start_cwd: Option<&str>,
) -> anyhow::Result<()> {
    if mode.is_new() {
        if let Some(dir) = start_cwd {
            if !std::path::Path::new(dir).is_dir() {
                return Err(anyhow::anyhow!(
                    "session cwd does not exist or is not a directory: {dir}"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn build_tmux_spawn_command(
    mode: TmuxSpawnMode,
    tmux_target: &TmuxTarget,
    session_id: &str,
    tmux_name: &str,
    start_cwd: Option<&str>,
    initial_command: Option<&str>,
) -> CommandBuilder {
    let mut command =
        build_tmux_spawn_command_args(mode, tmux_target, tmux_name, start_cwd, initial_command);
    configure_tmux_spawn_command_env(&mut command, session_id, tmux_name);
    command
}

pub(super) fn build_tmux_spawn_command_args(
    mode: TmuxSpawnMode,
    tmux_target: &TmuxTarget,
    tmux_name: &str,
    start_cwd: Option<&str>,
    initial_command: Option<&str>,
) -> CommandBuilder {
    match mode {
        TmuxSpawnMode::Attach => build_tmux_attach_command(tmux_target, tmux_name),
        TmuxSpawnMode::New => {
            build_tmux_new_session_command(tmux_target, tmux_name, start_cwd, initial_command)
        }
    }
}

fn build_tmux_attach_command(tmux_target: &TmuxTarget, tmux_name: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new("tmux");
    tmux_target.apply_to_command_builder(&mut command);
    let target = exact_session_target(tmux_name);
    command.args(["attach-session", "-t", &target]);
    command
}

fn build_tmux_new_session_command(
    tmux_target: &TmuxTarget,
    tmux_name: &str,
    start_cwd: Option<&str>,
    initial_command: Option<&str>,
) -> CommandBuilder {
    let mut command = CommandBuilder::new("tmux");
    tmux_target.apply_to_command_builder(&mut command);
    command.args(["new-session", "-s", tmux_name]);
    if let Some(dir) = start_cwd {
        command.args(["-c", dir]);
    }
    if let Some(command_arg) = initial_command {
        command.arg(command_arg);
    }
    command
}

fn configure_tmux_spawn_command_env(
    command: &mut CommandBuilder,
    session_id: &str,
    tmux_name: &str,
) {
    command.env_remove("TMUX");
    command.env_remove("TMUX_PANE");

    let inherited_term = std::env::var("TERM").ok();
    let inherited_colorterm = std::env::var("COLORTERM").ok();
    let (tmux_term, tmux_colorterm, used_term_fallback) =
        resolve_tmux_terminal_env(inherited_term.as_deref(), inherited_colorterm.as_deref());
    command.env("TERM", &tmux_term);
    command.env("COLORTERM", &tmux_colorterm);
    command.env("TERM_PROGRAM", "swimmers");

    log_tmux_spawn_terminal_env(
        session_id,
        tmux_name,
        inherited_term.as_deref(),
        &tmux_term,
        &tmux_colorterm,
        used_term_fallback,
    );
}

fn log_tmux_spawn_terminal_env(
    session_id: &str,
    tmux_name: &str,
    inherited_term: Option<&str>,
    tmux_term: &str,
    tmux_colorterm: &str,
    used_term_fallback: bool,
) {
    if used_term_fallback {
        warn!(
            session_id,
            tmux_name,
            inherited_term = ?inherited_term,
            applied_term = %tmux_term,
            "missing/unsupported TERM for tmux client; applied fallback"
        );
    } else {
        debug!(
            session_id,
            tmux_name,
            inherited_term = ?inherited_term,
            applied_term = %tmux_term,
            colorterm = %tmux_colorterm,
            "configured tmux client terminal environment"
        );
    }
}

pub(super) fn resolve_tmux_terminal_env(
    inherited_term: Option<&str>,
    inherited_colorterm: Option<&str>,
) -> (String, String, bool) {
    let (resolved_term, needs_term_fallback) = resolve_tmux_term(inherited_term);
    let colorterm = resolve_tmux_colorterm(inherited_colorterm);

    (resolved_term, colorterm, needs_term_fallback)
}

pub(super) fn resolve_tmux_term(inherited_term: Option<&str>) -> (String, bool) {
    let term = inherited_term.map(str::trim).unwrap_or_default();
    let needs_term_fallback = tmux_term_needs_fallback(term);
    let resolved_term = if needs_term_fallback {
        TMUX_FALLBACK_TERM
    } else {
        term
    }
    .to_string();

    (resolved_term, needs_term_fallback)
}

fn tmux_term_needs_fallback(term: &str) -> bool {
    TMUX_UNSUPPORTED_TERMS
        .iter()
        .any(|unsupported| term.eq_ignore_ascii_case(unsupported))
}

pub(super) fn resolve_tmux_colorterm(inherited_colorterm: Option<&str>) -> String {
    inherited_colorterm
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(TMUX_FALLBACK_COLORTERM)
        .to_string()
}

pub(super) fn inspect_tmux_child_after_spawn(
    mode: TmuxSpawnMode,
    child: &mut dyn portable_pty::Child,
) -> anyhow::Result<()> {
    if mode.is_new() {
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
    Ok(())
}

pub(super) struct SpawnedSessionActorInit {
    pub(super) session_id: String,
    pub(super) tmux_name: String,
    pub(super) tmux_target: TmuxTarget,
    pub(super) config: Arc<Config>,
    pub(super) master: Box<dyn MasterPty + Send>,
    pub(super) writer: Box<dyn std::io::Write + Send>,
    pub(super) cmd_rx: mpsc::Receiver<SessionCommand>,
    pub(super) event_tx: broadcast::Sender<ControlEvent>,
    pub(super) start_cwd: Option<String>,
    pub(super) initial_tool: Option<String>,
    pub(super) attach: bool,
    pub(super) last_activity_override: Option<chrono::DateTime<Utc>>,
    pub(super) batch: Option<SessionBatchMembership>,
}

pub(super) fn build_spawned_session_actor(init: SpawnedSessionActorInit) -> SessionActor {
    let state_detector = state_detector_for_initial_tool(init.initial_tool.as_deref());
    SessionActor {
        session_id: init.session_id,
        tmux_name: init.tmux_name,
        tmux_target: init.tmux_target,
        config: init.config.clone(),
        master: init.master,
        writer: init.writer,
        state_detector,
        scroll_guard: ScrollGuard::new(),
        replay_ring: ReplayRing::new(init.config.replay_buffer_size),
        subscribers: HashMap::new(),
        cmd_rx: init.cmd_rx,
        event_tx: init.event_tx,
        cols: 80,
        rows: 24,
        cwd: init.start_cwd.unwrap_or_default(),
        last_cwd_refresh_at: Instant::now(),
        last_tool_refresh_at: Instant::now(),
        last_liveness_check_at: Instant::now(),
        tmux_pane_metadata_cache: None,
        tool: init.initial_tool,
        last_skill: None,
        batch: init.batch,
        input_line_buffer: String::new(),
        last_activity_at: init.last_activity_override.unwrap_or_else(Utc::now),
        session_started_at: Utc::now(),
        clear_replay_on_first_idle: !init.attach,
    }
}
