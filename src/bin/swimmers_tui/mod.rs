use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::error::Error as StdError;
use std::f32::consts::TAU;
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, IsTerminal, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self as crossterm_terminal, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use futures::future::BoxFuture;
use mermaid_rs_renderer::{
    compute_layout, parse_mermaid, render_svg, DiagramKind, Layout as MermaidLayout, RenderOptions,
};
use reqwest::Client;
use resvg::tiny_skia::{Pixmap, Transform};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use usvg::Tree;

use swimmers::config::{AuthMode, Config};
use swimmers::repo_theme::{discover_repo_theme, existing_repo_theme};
use swimmers::thought::runtime_config::{DaemonDefaults, ThoughtConfig};
use swimmers::types::{
    CreateSessionRequest, CreateSessionResponse, DirEntry, DirListResponse, DirRepoActionRequest,
    DirRepoActionResponse, ErrorResponse, GhosttyOpenMode, MermaidArtifactResponse,
    NativeDesktopApp, NativeDesktopConfigRequest, NativeDesktopModeRequest,
    NativeDesktopOpenRequest, NativeDesktopOpenResponse, NativeDesktopStatusResponse,
    PlanFileResponse, PublishSelectionRequest, RepoActionKind, RepoActionState, RepoTheme,
    RestState, SessionListResponse, SessionState, SessionSummary, SpawnTool,
};

const MIN_WIDTH: u16 = 70;
const MIN_HEIGHT: u16 = 20;
const FRAME_DURATION: Duration = Duration::from_millis(33);
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const MESSAGE_TTL: Duration = Duration::from_secs(5);
/// How long the backend must be unreachable before the status bar escalates
/// from the short "backend offline" hint to the full diagnostic message.
const BACKEND_OFFLINE_ESCALATION: Duration = Duration::from_secs(10);
const API_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const API_REQUEST_TIMEOUT: Duration = Duration::from_millis(2_000);
const API_SESSION_LIST_TIMEOUT: Duration = Duration::from_secs(5);
const API_MERMAID_ARTIFACT_TIMEOUT: Duration = Duration::from_secs(5);
// /v1/dirs walks the configured base, runs git probes, and inspects services
// per entry. On hosts with many managed repos that real cost runs 4-10 s, so
// the 5 s ceiling we used to ship raced the work and surfaced false
// "backend unavailable" toasts. The real fix is making the endpoint faster
// (parallel probes, caching), but until then keep the budget generous so
// honest slow paths don't look like outages.
const API_DIRECTORY_LIST_TIMEOUT: Duration = Duration::from_secs(20);
const API_DIRECTORY_ACTION_TIMEOUT: Duration = Duration::from_secs(15);
const API_CREATE_SESSION_TIMEOUT: Duration = Duration::from_secs(10);
const API_STARTUP_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const API_STARTUP_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const API_STARTUP_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const API_NATIVE_OPEN_TIMEOUT: Duration = Duration::from_secs(3);
const SPRITE_HEIGHT: u16 = 4;
const LABEL_HEIGHT: u16 = 1;
const ENTITY_WIDTH: u16 = 12;
const ENTITY_HEIGHT: u16 = SPRITE_HEIGHT + LABEL_HEIGHT;
// Sprite theme toggle position (on the header row, right of "swimmers tui").
const SPRITE_THEME_TOGGLE_X: u16 = 16;
const SWIM_BOB_RATE: f32 = 0.08;
const SWIM_VERTICAL_DRIFT: f32 = 0.06;
const SWIM_DRIFT_LIMIT: f32 = 1.0;
const PICKER_WIDTH: u16 = 46;
const PICKER_MAX_HEIGHT: u16 = 16;
const INITIAL_REQUEST_WIDTH: u16 = 58;
const INITIAL_REQUEST_HEIGHT: u16 = 7;
const THOUGHT_RAIL_MIN_WIDTH: u16 = 100;
const THOUGHT_RAIL_MIN_PANEL_WIDTH: u16 = 24;
const THOUGHT_RAIL_GAP: u16 = 1;
const THOUGHT_RAIL_RATIO_DENOMINATOR: u16 = 3;
const THOUGHT_RAIL_HEADER_ROWS: u16 = 1;
const THOUGHT_RAIL_DEFAULT_RATIO: f32 = 1.0 / THOUGHT_RAIL_RATIO_DENOMINATOR as f32;
const THOUGHT_RAIL_DRAG_HITBOX_WIDTH: u16 = 3;
fn mermaid_badge_label(slice_name: Option<&str>) -> String {
    match slice_name {
        Some(name) => format!("[art:{name}]"),
        None => "[art]".to_string(),
    }
}
const MERMAID_BACK_LABEL: &str = "[back to bowl]";
const MERMAID_VIEW_MIN_WIDTH: u16 = 16;
const MERMAID_VIEW_MIN_HEIGHT: u16 = 8;
const MERMAID_KEYBOARD_ZOOM_STEP_PERCENT: i16 = 50;
const MERMAID_SCROLL_ZOOM_STEP_PERCENT: i16 = 25;
const MERMAID_MIN_ZOOM: f32 = 1.0;
const MERMAID_MAX_ZOOM: f32 = 8.0;

mod api;
mod app;
mod commit;
mod entity;
mod events;
mod layout;
mod mermaid;
mod picker;
mod render;
mod terminal;
mod thought_config_editor;
mod thoughts;

pub(crate) use api::*;
pub(crate) use app::*;
pub(crate) use commit::*;
pub(crate) use entity::*;
pub(crate) use events::*;
pub(crate) use layout::*;
pub(crate) use mermaid::*;
pub(crate) use picker::*;
pub(crate) use render::*;
pub(crate) use terminal::*;
pub(crate) use thought_config_editor::*;
pub(crate) use thoughts::*;

pub(crate) fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Capture the panic in the log file so post-mortem doesn't depend on
        // whether the terminal scrollback caught it before the alt screen left.
        tracing::error!(panic = %info, "swimmers-tui panicked");
        // Leave alternate screen before printing the panic so it is visible.
        let mut stdout = io::stdout();
        let _ = leave_terminal_ui(&mut stdout);
        let _ = crossterm_terminal::disable_raw_mode();
        if let Some(path) = client_log_path() {
            eprintln!("swimmers-tui: client log -> {}", path.display());
        }
        default_hook(info);
    }));
}

static CLIENT_LOG_FILE: OnceLock<std::fs::File> = OnceLock::new();
static CLIENT_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn client_log_path() -> Option<&'static Path> {
    CLIENT_LOG_PATH.get().map(PathBuf::as_path)
}

fn make_log_writer() -> Box<dyn io::Write + Send + 'static> {
    match CLIENT_LOG_FILE.get().and_then(|file| file.try_clone().ok()) {
        Some(file) => Box::new(file),
        None => Box::new(io::stderr()),
    }
}

/// Initialize a file-backed tracing subscriber for the TUI binary.
///
/// The diagnostic log path is `${SWIMMERS_TUI_LOG_DIR:-${TMPDIR:-/tmp}}/swimmers-tui-client-${pid}.log`.
/// Filter defaults to `swimmers_tui=info,reqwest=warn`; override with
/// `SWIMMERS_TUI_LOG=...` (env-filter syntax). On any IO failure we fall back
/// to a stderr subscriber so we never silently lose diagnostics.
pub(crate) fn init_tui_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_env("SWIMMERS_TUI_LOG")
        .unwrap_or_else(|_| EnvFilter::new("swimmers_tui=info,reqwest=warn"));

    let dir = env::var_os("SWIMMERS_TUI_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
        });

    let try_open = std::fs::create_dir_all(&dir).and_then(|_| {
        let path = dir.join(format!("swimmers-tui-client-{}.log", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok((path, file))
    });

    match try_open {
        Ok((path, file)) => {
            let _ = CLIENT_LOG_FILE.set(file);
            let _ = CLIENT_LOG_PATH.set(path.clone());
            let _ = fmt()
                .with_env_filter(filter)
                .with_writer(make_log_writer)
                .with_ansi(false)
                .with_target(true)
                .try_init();
            tracing::info!(
                path = %path.display(),
                pid = std::process::id(),
                "swimmers-tui client log opened"
            );
        }
        Err(err) => {
            let _ = fmt()
                .with_env_filter(filter)
                .with_writer(io::stderr)
                .with_ansi(false)
                .try_init();
            tracing::warn!(
                dir = %dir.display(),
                error = %err,
                "could not open swimmers-tui log file; logging to stderr"
            );
        }
    }
}

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tui_tracing();
    install_panic_hook();
    tracing::info!(
        target_url = std::env::var("SWIMMERS_TUI_URL").as_deref().unwrap_or("<unset>"),
        "swimmers-tui run loop starting"
    );
    let (mut app, mut renderer) = initialize_tui_app()?;

    loop {
        let layout = prepare_frame(&mut app, &mut renderer);
        renderer.flush()?;

        if event::poll(FRAME_DURATION)?
            && !handle_tui_event(&mut app, &mut renderer, layout, event::read()?)?
        {
            break;
        }
    }

    app.clear_published_selection();
    renderer.cleanup()?;
    Ok(())
}

#[cfg(test)]
mod tests;
