use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::error::Error as StdError;
use std::f32::consts::TAU;
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, IsTerminal, Stdout, Write};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
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
    CreateSessionRequest, CreateSessionResponse, DirEntry, DirListResponse, ErrorResponse,
    GhosttyOpenMode, MermaidArtifactResponse, NativeDesktopApp, NativeDesktopConfigRequest,
    NativeDesktopModeRequest, NativeDesktopOpenRequest, NativeDesktopOpenResponse,
    NativeDesktopStatusResponse, PlanFileResponse, PublishSelectionRequest, RepoTheme, RestState,
    SessionListResponse, SessionState, SessionSummary, SpawnTool,
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
const API_DIRECTORY_LIST_TIMEOUT: Duration = Duration::from_secs(5);
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
        Some(name) => format!("[mmd:{name}]"),
        None => "[mmd]".to_string(),
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
        // Leave alternate screen before printing the panic so it is visible.
        let mut stdout = io::stdout();
        let _ = leave_terminal_ui(&mut stdout);
        let _ = crossterm_terminal::disable_raw_mode();
        default_hook(info);
    }));
}

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    install_panic_hook();
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
