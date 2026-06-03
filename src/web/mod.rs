use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::envelope::error_body_msg;
use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope, OBSERVER_SCOPES, OPERATOR_SCOPES};
use crate::config::{AuthMode, Config};
use crate::session::actor::{
    ActorHandle, InputDeliveryResult, OutputFrame, ReplayCursor, SessionCommand, SubscribeOutcome,
};
use crate::session::supervisor::LifecycleEvent;
use crate::types::{clamp_terminal_resize, opcodes, ControlEvent, SessionSummary};

const APP_JS_ROUTE: &str = "/app.js";
const RENDERED_SURFACE_JS_ROUTE: &str = "/rendered_surface.js";
const INPUT_SUPPORT_JS_ROUTE: &str = "/input_support.js";
const SEND_SHEET_JS_ROUTE: &str = "/send_sheet.js";
const TERMINAL_SURFACE_SETUP_JS_ROUTE: &str = "/terminal_surface_setup.js";
const TERMINAL_RESIZE_JS_ROUTE: &str = "/terminal_resize.js";
const GLOBAL_SHORTCUT_DISPATCH_JS_ROUTE: &str = "/global_shortcut_dispatch.js";
const SESSION_REFRESH_JS_ROUTE: &str = "/session_refresh.js";
const MERMAID_ARTIFACT_JS_ROUTE: &str = "/mermaid_artifact.js";
const TERMINAL_SAFETY_JS_ROUTE: &str = "/terminal_safety.js";
const TERMINAL_PROTOCOL_JS_ROUTE: &str = "/terminal_protocol.js";
const DIR_BROWSER_JS_ROUTE: &str = "/dir_browser.js";
const COMMAND_PALETTE_JS_ROUTE: &str = "/command_palette.js";
const TROGDOR_LOGIC_JS_ROUTE: &str = "/trogdor_logic.js";
const TROGDOR_RENDER_JS_ROUTE: &str = "/trogdor_render.js";
const WORKBENCH_RENDER_JS_ROUTE: &str = "/workbench_render.js";
const WORKBENCH_REFRESH_JS_ROUTE: &str = "/workbench_refresh.js";
const WORKBENCH_RECORDS_JS_ROUTE: &str = "/workbench_records.js";
const APP_CSS_ROUTE: &str = "/app.css";
const FRANKENTERM_JS_ROUTE: &str = "/assets/frankenterm/FrankenTerm.js";
const FRANKENTERM_WASM_ROUTE: &str = "/assets/frankenterm/FrankenTerm_bg.wasm";
const FRANKENTERM_FONT_ROUTE: &str = "/assets/frankenterm/pragmasevka-nf-subset.woff2";
const TROGDOR_DRAGON_ASSET_ROUTE: &str = "/assets/dragon/{pose}/{frame}";
const PUBLISHED_VIEW_ROUTE: &str = "/selected";
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const WS_AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WS_INPUT_BYTES: usize = 786_432;
const MAX_BROWSER_WS_CONNECTIONS: usize = 64;
const DEFAULT_FRANKENTUI_PKG_CANDIDATES: &[&str] = &[];

static NEXT_WS_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_WS_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

type WsSender = SplitSink<WebSocket, Message>;
type WsReceiver = SplitStream<WebSocket>;

struct ActiveWsGuard;

impl ActiveWsGuard {
    fn try_acquire() -> Option<Self> {
        ACTIVE_WS_CONNECTIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_BROWSER_WS_CONNECTIONS).then_some(current + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for ActiveWsGuard {
    fn drop(&mut self) {
        ACTIVE_WS_CONNECTIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(index))
        .route(PUBLISHED_VIEW_ROUTE, get(selected_index))
        .route(APP_JS_ROUTE, get(app_js))
        .route(RENDERED_SURFACE_JS_ROUTE, get(rendered_surface_js))
        .route(INPUT_SUPPORT_JS_ROUTE, get(input_support_js))
        .route(SEND_SHEET_JS_ROUTE, get(send_sheet_js))
        .route(
            TERMINAL_SURFACE_SETUP_JS_ROUTE,
            get(terminal_surface_setup_js),
        )
        .route(TERMINAL_RESIZE_JS_ROUTE, get(terminal_resize_js))
        .route(
            GLOBAL_SHORTCUT_DISPATCH_JS_ROUTE,
            get(global_shortcut_dispatch_js),
        )
        .route(SESSION_REFRESH_JS_ROUTE, get(session_refresh_js))
        .route(MERMAID_ARTIFACT_JS_ROUTE, get(mermaid_artifact_js))
        .route(TERMINAL_SAFETY_JS_ROUTE, get(terminal_safety_js))
        .route(TERMINAL_PROTOCOL_JS_ROUTE, get(terminal_protocol_js))
        .route(DIR_BROWSER_JS_ROUTE, get(dir_browser_js))
        .route(COMMAND_PALETTE_JS_ROUTE, get(command_palette_js))
        .route(TROGDOR_LOGIC_JS_ROUTE, get(trogdor_logic_js))
        .route(TROGDOR_RENDER_JS_ROUTE, get(trogdor_render_js))
        .route(WORKBENCH_RENDER_JS_ROUTE, get(workbench_render_js))
        .route(WORKBENCH_REFRESH_JS_ROUTE, get(workbench_refresh_js))
        .route(WORKBENCH_RECORDS_JS_ROUTE, get(workbench_records_js))
        .route(APP_CSS_ROUTE, get(app_css))
        .route(FRANKENTERM_JS_ROUTE, get(franken_term_js))
        .route(FRANKENTERM_WASM_ROUTE, get(franken_term_wasm))
        .route(FRANKENTERM_FONT_ROUTE, get(franken_term_font))
        .route(TROGDOR_DRAGON_ASSET_ROUTE, get(trogdor_dragon_asset))
        .route("/ws/sessions/{session_id}", get(session_ws))
}

#[derive(Debug, Serialize)]
struct BootPayload {
    franken_term_available: bool,
    franken_term_js_url: &'static str,
    franken_term_wasm_url: &'static str,
    franken_term_font_url: &'static str,
    franken_term_asset_info: Option<FrankenTermAssetInfo>,
    follow_published_selection: bool,
    focus_layout: bool,
}

#[derive(Debug, Serialize)]
struct FrankenTermAssetInfo {
    js: FrankenTermAssetFileInfo,
    wasm: FrankenTermAssetFileInfo,
    font: Option<FrankenTermAssetFileInfo>,
}

#[derive(Debug, Serialize)]
struct FrankenTermAssetFileInfo {
    route: &'static str,
    size_bytes: u64,
    checksum: String,
}

async fn index() -> impl IntoResponse {
    render_index(false).await
}

async fn selected_index() -> impl IntoResponse {
    render_index(true).await
}

async fn render_index(focus_layout: bool) -> impl IntoResponse {
    let boot = BootPayload {
        franken_term_available: resolve_frankentui_pkg_dir().is_some(),
        franken_term_js_url: FRANKENTERM_JS_ROUTE,
        franken_term_wasm_url: FRANKENTERM_WASM_ROUTE,
        franken_term_font_url: FRANKENTERM_FONT_ROUTE,
        franken_term_asset_info: franken_term_asset_info().await,
        follow_published_selection: focus_layout,
        focus_layout,
    };
    let boot_json = serde_json::to_string(&boot).unwrap_or_else(|_| "{}".to_string());
    let body_class = if focus_layout {
        "app-body published-focus"
    } else {
        "app-body"
    };

    let html = format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>swimmers</title>
    <link rel="preload" href="{FRANKENTERM_FONT_ROUTE}" as="font" type="font/woff2" crossorigin />
    <link rel="stylesheet" href="{APP_CSS_ROUTE}" />
  </head>
  <body class="{body_class}">
    <div class="shell">
      <main
        class="terminal-stage"
        id="terminal-stage"
        tabindex="0"
        role="application"
        aria-label="swimmers rendered control surface"
      >
        <canvas class="terminal-canvas hidden" id="terminal-canvas"></canvas>
        <canvas class="hud-canvas hidden" id="hud-canvas" aria-hidden="true"></canvas>
        <pre
          class="terminal-fallback hidden"
          id="terminal-fallback"
          tabindex="0"
          aria-label="Live terminal text fallback"></pre>
        <textarea
          class="terminal-a11y-mirror"
          id="terminal-a11y-mirror"
          aria-label="Live terminal text mirror"
          readonly
          tabindex="-1"></textarea>
        <div class="terminal-announcer" id="terminal-announcer" aria-live="polite" aria-atomic="false"></div>
        <div class="terminal-status-strip" id="terminal-status-strip" aria-live="polite"></div>
        <div class="terminal-link-tools hidden" id="terminal-link-tools" role="group" aria-label="Terminal link actions">
          <span id="terminal-link-text"></span>
          <button id="terminal-link-open" type="button">Open</button>
          <button id="terminal-link-copy" type="button">Copy</button>
        </div>
        <div class="loading-overlay visible" id="loading-overlay" aria-hidden="true">
          <div class="loading-label" id="loading-label">Loading FrankenTerm…</div>
          <div class="loading-bar"><div class="loading-bar-fill"></div></div>
        </div>
        <section class="trogdor-surface hidden" id="trogdor-surface" aria-label="Trogdor repository atlas"></section>
      </main>
      <textarea
        class="mobile-kb-proxy"
        id="mobile-kb-proxy"
        aria-hidden="true"
        tabindex="-1"
        inputmode="text"
        autocomplete="off"
        autocorrect="off"
        autocapitalize="off"
        spellcheck="false"></textarea>
      <button
        class="terminal-trogdor-back hidden"
        id="terminal-trogdor-back"
        type="button"
        title="Back to Trogdor atlas"
        aria-label="Back to Trogdor atlas"
        aria-hidden="true">Trogdor</button>
      <div class="terminal-control-strip" id="terminal-control-strip" aria-label="Terminal viewer controls">
        <button id="terminal-palette" type="button" title="Open command palette">K</button>
        <button id="terminal-copy-frame" type="button" title="Copy visible terminal text">TXT</button>
        <button id="terminal-zoom-out" type="button" title="Zoom out">A-</button>
        <button id="terminal-zoom-reset" type="button" title="Reset terminal zoom">100%</button>
        <button id="terminal-zoom-in" type="button" title="Zoom in">A+</button>
        <button id="terminal-mobile-keyboard" type="button" title="Toggle mobile keyboard" aria-pressed="false">KB</button>
        <button id="terminal-workbench-toggle" type="button" title="Toggle session workbench" aria-pressed="false">WB</button>
      </div>
      <aside class="terminal-workbench hidden" id="terminal-workbench" aria-label="Session workbench" aria-hidden="true">
        <div class="workbench-header">
          <div class="workbench-heading">
            <span class="workbench-kicker">Workbench</span>
            <strong id="terminal-workbench-title">No session</strong>
            <span id="terminal-workbench-meta"></span>
          </div>
          <button id="terminal-workbench-refresh" type="button" title="Refresh workbench context">Refresh</button>
        </div>
        <div class="workbench-status" id="terminal-workbench-status" aria-live="polite">idle</div>
        <section class="workbench-section">
          <span class="workbench-label">Task</span>
          <p id="terminal-workbench-task">No task context.</p>
        </section>
        <section class="workbench-section">
          <span class="workbench-label">Now</span>
          <p id="terminal-workbench-current">No current action.</p>
        </section>
        <section class="workbench-section">
          <span class="workbench-label">Pressure</span>
          <p id="terminal-workbench-pressure">No pressure cues.</p>
        </section>
        <section class="workbench-section">
          <span class="workbench-label">Recent</span>
          <ul class="workbench-actions" id="terminal-workbench-actions"></ul>
        </section>
        <section class="workbench-section">
          <span class="workbench-label">Pinned</span>
          <div class="workbench-widgets" id="terminal-workbench-widgets"></div>
        </section>
      </aside>
      <form class="terminal-input-dock hidden" id="terminal-input-dock" aria-label="Terminal input">
        <div class="terminal-key-strip" id="terminal-key-strip" aria-label="Terminal control keys">
          <button type="button" data-terminal-key="ctrl-c" title="Send Ctrl-C">Ctrl-C</button>
          <button type="button" data-terminal-key="escape" title="Send Escape">Esc</button>
          <button type="button" data-terminal-key="tab" title="Send Tab">Tab</button>
          <button type="button" data-terminal-key="arrow-left" title="Send Left">←</button>
          <button type="button" data-terminal-key="arrow-down" title="Send Down">↓</button>
          <button type="button" data-terminal-key="arrow-up" title="Send Up">↑</button>
          <button type="button" data-terminal-key="arrow-right" title="Send Right">→</button>
          <button type="button" data-terminal-key="home" title="Send Home">Home</button>
          <button type="button" data-terminal-key="end" title="Send End">End</button>
          <button type="button" data-terminal-key="page-up" title="Send Page Up">PgUp</button>
          <button type="button" data-terminal-key="page-down" title="Send Page Down">PgDn</button>
        </div>
        <span class="terminal-input-prompt" aria-hidden="true">›</span>
        <textarea
          id="terminal-inline-input"
          rows="1"
          autocomplete="off"
          autocorrect="off"
          autocapitalize="off"
          spellcheck="false"
          aria-label="Terminal input"></textarea>
        <button id="terminal-input-send" type="submit">Send</button>
        <div class="terminal-input-echo" id="terminal-input-echo" aria-live="polite"></div>
      </form>
      <button class="trogdor-launcher hidden" id="trogdor-launcher" type="button">burninate!</button>

      <div class="modal-root" id="modal-root" aria-hidden="true">
        <div class="modal-backdrop" id="modal-backdrop"></div>

        <section class="surface-sheet hidden palette-sheet" id="palette-sheet" aria-labelledby="palette-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Terminal Actions</p>
            <h2 id="palette-sheet-title">Command Palette</h2>
          </div>
          <label class="field">
            <span>Command or session</span>
            <input id="palette-search" type="search" placeholder="Search actions and sessions" autocomplete="off" />
          </label>
          <div class="palette-results" id="palette-results" role="listbox" aria-label="Command palette results"></div>
          <div class="sheet-actions">
            <button class="ghost-button" id="palette-close-button" type="button">Close</button>
          </div>
        </section>

        <section class="surface-sheet hidden" id="search-sheet" aria-labelledby="search-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Rendered Action</p>
            <h2 id="search-sheet-title">Search Terminal</h2>
          </div>
          <form class="sheet-form" id="search-form">
            <label class="field">
              <span>Query</span>
              <input id="terminal-search" type="search" placeholder="Find text in the current terminal view" autocomplete="off" />
            </label>
            <div class="sheet-actions">
              <button class="ghost-button" id="search-prev-button" type="button">Prev</button>
              <button class="ghost-button" id="search-next-button" type="button">Next</button>
              <button class="ghost-button" id="search-clear-button" type="button">Clear</button>
              <button id="search-close-button" type="submit">Done</button>
            </div>
          </form>
        </section>

        <section class="surface-sheet hidden" id="thought-config-sheet" aria-labelledby="thought-config-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Policy</p>
            <h2 id="thought-config-title">Thought Config</h2>
          </div>
          <div class="sheet-copy" id="thought-config-summary">Loading thought config…</div>
          <form class="sheet-form" id="thought-config-form">
            <div class="field">
              <span>Enabled</span>
              <label class="toggle-row">
                <input id="thought-config-enabled" type="checkbox" />
                <span>Run the thought loop</span>
              </label>
            </div>
            <label class="field">
              <span>Backend</span>
              <select id="thought-config-backend"></select>
            </label>
            <label class="field">
              <span>Model</span>
              <input id="thought-config-model" type="text" placeholder="Use backend default or choose a preset" autocomplete="off" list="thought-config-model-presets" />
              <datalist id="thought-config-model-presets"></datalist>
            </label>
            <div class="sheet-copy" id="thought-config-hint"></div>
            <div class="sheet-copy" id="thought-config-daemon"></div>
            <pre class="sheet-result" id="thought-config-result"></pre>
            <div class="sheet-actions">
              <button class="ghost-button" id="thought-config-test-button" type="button">Test</button>
              <button class="ghost-button" id="thought-config-close-button" type="button">Close</button>
              <button id="thought-config-save-button" type="submit">Save</button>
            </div>
          </form>
        </section>

        <section class="surface-sheet hidden" id="native-sheet" aria-labelledby="native-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Desktop</p>
            <h2 id="native-sheet-title">Native Open</h2>
          </div>
          <div class="sheet-copy" id="native-status-copy">Loading native status…</div>
          <form class="sheet-form" id="native-form">
            <label class="field">
              <span>App</span>
              <select id="native-app">
                <option value="iterm">iTerm</option>
                <option value="ghostty">Ghostty</option>
              </select>
            </label>
            <label class="field">
              <span>Ghostty mode</span>
              <select id="native-mode">
                <option value="swap">swap</option>
                <option value="add">add</option>
              </select>
            </label>
            <pre class="sheet-result" id="native-status-result"></pre>
            <div class="sheet-actions">
              <button class="ghost-button" id="native-refresh-button" type="button">Refresh</button>
              <button class="ghost-button" id="native-open-button" type="button">Open Selected</button>
              <button class="ghost-button" id="native-close-button" type="button">Close</button>
              <button id="native-save-button" type="submit">Apply</button>
            </div>
          </form>
        </section>

        <section class="surface-sheet hidden" id="send-sheet" aria-labelledby="send-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Rendered Action</p>
            <h2 id="send-sheet-title">Send Line</h2>
          </div>
          <form class="sheet-form" id="send-form">
            <label class="field">
              <span>Mode</span>
              <select id="send-mode">
                <option value="line">Send + Enter</option>
                <option value="paste">Paste only</option>
              </select>
            </label>
            <label class="field">
              <span>Input</span>
              <textarea id="send-input" rows="5" placeholder="Type a command or paste text. Send appends a newline."></textarea>
            </label>
            <div class="send-history" id="send-history" aria-label="Recent sends"></div>
            <div class="sheet-copy" id="send-hint">Send submits the text to the selected agent prompt. Paste only preserves text exactly for the selected live terminal.</div>
            <div class="sheet-actions">
              <button class="ghost-button" id="send-close-button" type="button">Cancel</button>
              <button id="send-submit-button" type="submit">Send</button>
            </div>
          </form>
        </section>

        <section class="surface-sheet hidden" id="auth-sheet" aria-labelledby="auth-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Connection</p>
            <h2 id="auth-sheet-title">Auth Token</h2>
          </div>
          <div class="sheet-copy">
            Paste `AUTH_TOKEN` or `OBSERVER_TOKEN` when the API is running in token mode.
          </div>
          <div class="sheet-form">
            <label class="field">
              <span>Token</span>
              <input id="token-input" type="password" placeholder="Optional bearer token" autocomplete="off" />
            </label>
            <div class="sheet-actions">
              <button class="ghost-button" id="clear-token-button" type="button">Forget</button>
              <button class="ghost-button" id="auth-close-button" type="button">Close</button>
              <button id="save-token-button" type="button">Connect</button>
            </div>
          </div>
        </section>

        <section class="surface-sheet hidden create-console" id="create-sheet" aria-labelledby="create-sheet-title">
          <header class="console-head">
            <div class="console-heading">
              <p class="console-eyebrow">Repository atlas</p>
              <h2 id="create-sheet-title">Create session</h2>
            </div>
            <button class="console-dismiss" id="create-close-button" type="button" aria-label="Close">esc</button>
          </header>

          <div class="console-toolbar">
            <div class="console-search">
              <svg class="console-search-icon" viewBox="0 0 16 16" width="15" height="15" fill="none" aria-hidden="true">
                <circle cx="7" cy="7" r="4.5" stroke="currentColor" stroke-width="1.5"></circle>
                <path d="M11 11l3.2 3.2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"></path>
              </svg>
              <input id="dirs-search" type="search" placeholder="Search repos, paths, groups…" autocomplete="off" aria-label="Search repositories" />
            </div>
            <label class="console-toggle">
              <input id="dirs-managed-only" type="checkbox" />
              <span>Managed only</span>
            </label>
            <button class="console-ghost" id="create-batch-visible" type="button">Select all</button>
          </div>

          <div class="console-chips" id="dirs-groups" role="group" aria-label="Repository groups"></div>

          <div class="console-pathbar">
            <span class="console-pathbar-kicker">Browsing</span>
            <input id="dirs-path" type="text" placeholder="/absolute/path" autocomplete="off" aria-label="Browse path" />
            <button class="console-ghost" id="dirs-up-button" type="button">Up</button>
            <button class="console-ghost" id="dirs-load-button" type="button">Load</button>
            <button class="console-ghost console-ghost-accent" id="dirs-spawn-here" type="button">Spawn here</button>
          </div>

          <div class="console-table" role="table" aria-label="Repositories">
            <div class="console-row console-row-head" role="row">
              <span class="col-select" aria-hidden="true"></span>
              <span class="col-name" role="columnheader">Repository</span>
              <span class="col-path" role="columnheader">Path</span>
              <span class="col-status" role="columnheader">Status</span>
              <span class="col-groups" role="columnheader">Groups</span>
            </div>
            <div class="console-body browser-list" id="dirs-list" role="rowgroup" aria-label="Directory entries"></div>
          </div>

          <form class="console-dock" id="create-form">
            <div class="console-dock-grid">
              <label class="dock-field dock-field-wide">
                <span>Working directory</span>
                <input id="create-cwd" type="text" placeholder="/absolute/path" autocomplete="off" />
              </label>
              <label class="dock-field">
                <span>Tool</span>
                <span class="dock-select">
                  <select id="create-tool">
                    <option value="grok">Grok</option>
                    <option value="codex">Codex</option>
                    <option value="claude">Claude</option>
                  </select>
                  <svg viewBox="0 0 10 6" width="10" height="6" fill="none" aria-hidden="true"><path d="M1 1l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"></path></svg>
                </span>
              </label>
              <label class="dock-field">
                <span>Launch target</span>
                <span class="dock-select">
                  <select id="create-launch-target"></select>
                  <svg viewBox="0 0 10 6" width="10" height="6" fill="none" aria-hidden="true"><path d="M1 1l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"></path></svg>
                </span>
              </label>
            </div>
            <label class="dock-field dock-field-prompt">
              <span>Boot prompt <em>optional</em></span>
              <textarea id="create-request" rows="2" placeholder="Optional first message for the new session"></textarea>
            </label>
            <div class="console-dock-foot">
              <p class="console-status" id="dirs-summary">Browse directories before creating a session.</p>
              <div class="console-batch hidden" id="create-batch-bar" aria-live="polite">
                <div class="console-batch-copy">
                  <span class="console-batch-count" id="create-batch-count">0 selected</span>
                  <span class="console-batch-tool" id="create-batch-tool">tool: grok</span>
                  <span class="console-batch-preview" id="create-batch-preview">request: (none)</span>
                </div>
                <button class="console-ghost console-batch-clear" id="create-batch-clear" type="button">Clear</button>
                <button class="console-batch-submit" id="create-batch-submit" type="submit" form="create-form">Batch send</button>
              </div>
              <button class="console-create" id="create-button" type="submit">Create session</button>
            </div>
          </form>
        </section>

        <section class="surface-sheet hidden" id="mermaid-sheet" aria-labelledby="mermaid-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Artifact</p>
            <h2 id="mermaid-sheet-title">Mermaid Diagram</h2>
          </div>
          <div class="sheet-copy" id="mermaid-summary">Loading Mermaid artifact…</div>
          <div class="mermaid-preview" id="mermaid-preview" aria-live="polite"></div>
          <pre class="sheet-result" id="mermaid-source"></pre>
          <div class="plan-tabs hidden" id="mermaid-plan-tabs" aria-label="Plan files"></div>
          <pre class="sheet-result hidden" id="mermaid-plan-content"></pre>
          <div class="sheet-actions">
            <button class="ghost-button" id="mermaid-refresh-button" type="button">Refresh</button>
            <button class="ghost-button" id="mermaid-open-button" type="button">Open Host Artifact</button>
            <button class="ghost-button" id="mermaid-close-button" type="button">Close</button>
          </div>
        </section>
      </div>
    </div>

    <script>window.__SWIMMERS_BOOT__ = {boot_json};</script>
    <script type="module" src="{APP_JS_ROUTE}"></script>
  </body>
</html>"#
    );

    ([(header::CACHE_CONTROL, "no-store")], Html(html))
}

/// In debug builds, serve a web asset from its on-disk source so CSS/JS edits
/// show up on a plain browser refresh (no rebuild). Falls back to the baked-in
/// copy if the file can't be read. Release builds always use the embedded copy,
/// at zero cost. `relative` is the path from the crate root (where Cargo.toml
/// lives); `baked` is the matching `include_str!` constant. Note: the page HTML
/// is templated in Rust, so markup changes in this file still need a rebuild.
#[cfg(debug_assertions)]
fn dev_asset(relative: &str, baked: &'static str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| baked.to_string())
}

#[cfg(not(debug_assertions))]
fn dev_asset(_relative: &str, baked: &'static str) -> &'static str {
    baked
}

fn javascript_asset(relative: &str, baked: &'static str) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        dev_asset(relative, baked),
    )
        .into_response()
}

async fn app_js() -> Response {
    javascript_asset("src/web/app.js", include_str!("app.js"))
}

async fn rendered_surface_js() -> Response {
    javascript_asset(
        "src/web/rendered_surface.js",
        include_str!("rendered_surface.js"),
    )
}

async fn input_support_js() -> Response {
    javascript_asset("src/web/input_support.js", include_str!("input_support.js"))
}

async fn send_sheet_js() -> Response {
    javascript_asset("src/web/send_sheet.js", include_str!("send_sheet.js"))
}

async fn terminal_surface_setup_js() -> Response {
    javascript_asset(
        "src/web/terminal_surface_setup.js",
        include_str!("terminal_surface_setup.js"),
    )
}

async fn terminal_resize_js() -> Response {
    javascript_asset(
        "src/web/terminal_resize.js",
        include_str!("terminal_resize.js"),
    )
}

async fn global_shortcut_dispatch_js() -> Response {
    javascript_asset(
        "src/web/global_shortcut_dispatch.js",
        include_str!("global_shortcut_dispatch.js"),
    )
}

async fn session_refresh_js() -> Response {
    javascript_asset(
        "src/web/session_refresh.js",
        include_str!("session_refresh.js"),
    )
}

async fn mermaid_artifact_js() -> Response {
    javascript_asset(
        "src/web/mermaid_artifact.js",
        include_str!("mermaid_artifact.js"),
    )
}

async fn terminal_safety_js() -> Response {
    javascript_asset(
        "src/web/terminal_safety.js",
        include_str!("terminal_safety.js"),
    )
}

async fn terminal_protocol_js() -> Response {
    javascript_asset(
        "src/web/terminal_protocol.js",
        include_str!("terminal_protocol.js"),
    )
}

async fn dir_browser_js() -> Response {
    javascript_asset("src/web/dir_browser.js", include_str!("dir_browser.js"))
}

async fn command_palette_js() -> Response {
    javascript_asset(
        "src/web/command_palette.js",
        include_str!("command_palette.js"),
    )
}

async fn trogdor_logic_js() -> Response {
    javascript_asset("src/web/trogdor_logic.js", include_str!("trogdor_logic.js"))
}

async fn trogdor_render_js() -> Response {
    javascript_asset(
        "src/web/trogdor_render.js",
        include_str!("trogdor_render.js"),
    )
}

async fn workbench_render_js() -> Response {
    javascript_asset(
        "src/web/workbench_render.js",
        include_str!("workbench_render.js"),
    )
}

async fn workbench_refresh_js() -> Response {
    javascript_asset(
        "src/web/workbench_refresh.js",
        include_str!("workbench_refresh.js"),
    )
}

async fn workbench_records_js() -> Response {
    javascript_asset(
        "src/web/workbench_records.js",
        include_str!("workbench_records.js"),
    )
}

async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        dev_asset("src/web/app.css", include_str!("app.css")),
    )
}

async fn trogdor_dragon_asset(AxumPath((pose, frame)): AxumPath<(String, String)>) -> Response {
    let Some(bytes) = trogdor_dragon_asset_bytes(&pose, &frame) else {
        return json_error(
            StatusCode::NOT_FOUND,
            "TROGDOR_DRAGON_ASSET_NOT_FOUND",
            "The requested Trogdor dragon sprite frame is not available",
        );
    };

    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response()
}

fn trogdor_dragon_asset_bytes(pose: &str, frame: &str) -> Option<&'static [u8]> {
    // All eight body frames (8-way directional) shipped for every pose. Names
    // match the on-disk filenames: cardinal directions plus 3/4 views.
    macro_rules! frames_for {
        ($pose:literal) => {
            match frame {
                "left.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/left.png"))[..])
                }
                "right.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/right.png"))[..])
                }
                "front.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/front.png"))[..])
                }
                "back.png" => {
                    Some(&include_bytes!(concat!("../../assets/dragon/", $pose, "/back.png"))[..])
                }
                "3q-left.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/3q-left.png"))[..],
                ),
                "3q-right.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/3q-right.png"))[..],
                ),
                "back-left.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/back-left.png"))[..],
                ),
                "back-right.png" => Some(
                    &include_bytes!(concat!("../../assets/dragon/", $pose, "/back-right.png"))[..],
                ),
                _ => None,
            }
        };
    }
    match pose {
        "mouth-closed" => frames_for!("mouth-closed"),
        "mouth-open" => frames_for!("mouth-open"),
        "fire-left-short" => frames_for!("fire-left-short"),
        "fire-left-mid" => frames_for!("fire-left-mid"),
        "fire-left-full" => frames_for!("fire-left-full"),
        "fire-right-short" => frames_for!("fire-right-short"),
        "fire-right-mid" => frames_for!("fire-right-mid"),
        "fire-right-full" => frames_for!("fire-right-full"),
        _ => None,
    }
}

async fn franken_term_js() -> Response {
    serve_frankentui_asset("FrankenTerm.js", "application/javascript; charset=utf-8").await
}

async fn franken_term_wasm() -> Response {
    serve_frankentui_asset("FrankenTerm_bg.wasm", "application/wasm").await
}

async fn franken_term_font() -> Response {
    let Some(pkg_dir) = resolve_frankentui_pkg_dir() else {
        return json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_ASSET_UNAVAILABLE",
            "FrankenTerm package assets are not available on this host",
        );
    };

    let Some(root_dir) = pkg_dir.parent() else {
        return json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_FONT_UNAVAILABLE",
            "FrankenTerm root directory could not be resolved",
        );
    };

    let path = root_dir.join("fonts").join("pragmasevka-nf-subset.woff2");
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_FONT_UNAVAILABLE",
            &format!("font asset was not found in {}", path.display()),
        ),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "FRANKENTERM_FONT_READ_FAILED",
            &format!("failed to read font asset: {err}"),
        ),
    }
}

async fn serve_frankentui_asset(file_name: &str, content_type: &'static str) -> Response {
    let Some(pkg_dir) = resolve_frankentui_pkg_dir() else {
        return json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_ASSET_UNAVAILABLE",
            "FrankenTerm package assets are not available on this host",
        );
    };

    let path = pkg_dir.join(file_name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json_error(
            StatusCode::NOT_FOUND,
            "FRANKENTERM_ASSET_MISSING",
            &format!("{file_name} was not found in {}", pkg_dir.display()),
        ),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "FRANKENTERM_ASSET_READ_FAILED",
            &format!("failed to read {file_name}: {err}"),
        ),
    }
}

async fn franken_term_asset_info() -> Option<FrankenTermAssetInfo> {
    let pkg_dir = resolve_frankentui_pkg_dir()?;
    let js_path = pkg_dir.join("FrankenTerm.js");
    let wasm_path = pkg_dir.join("FrankenTerm_bg.wasm");
    let js = franken_term_asset_file_info(&js_path, FRANKENTERM_JS_ROUTE).await?;
    let wasm = franken_term_asset_file_info(&wasm_path, FRANKENTERM_WASM_ROUTE).await?;
    let font = pkg_dir
        .parent()
        .map(|root| root.join("fonts").join("pragmasevka-nf-subset.woff2"))
        .and_then(|path| {
            std::fs::metadata(&path)
                .ok()
                .filter(|meta| meta.is_file())
                .map(|_| path)
        });
    let font = match font {
        Some(path) => franken_term_asset_file_info(&path, FRANKENTERM_FONT_ROUTE).await,
        None => None,
    };
    Some(FrankenTermAssetInfo { js, wasm, font })
}

async fn franken_term_asset_file_info(
    path: &Path,
    route: &'static str,
) -> Option<FrankenTermAssetFileInfo> {
    let bytes = tokio::fs::read(path).await.ok()?;
    Some(FrankenTermAssetFileInfo {
        route,
        size_bytes: bytes.len() as u64,
        checksum: format!("crc32:{:08x}", crc32fast::hash(&bytes)),
    })
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: Option<String>,
    resume_from_seq: Option<u64>,
    framed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WsOutputMode {
    Raw,
    Framed,
}

async fn session_ws(
    ws: WebSocketUpgrade,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let output_mode = if query_flag_enabled(query.framed.as_deref()) {
        WsOutputMode::Framed
    } else {
        WsOutputMode::Raw
    };
    let resume_from_seq = query.resume_from_seq;

    match state.config.auth_mode {
        AuthMode::LocalTrust | AuthMode::TailnetTrust => {
            let auth = AuthInfo::new(OPERATOR_SCOPES.to_vec());
            if let Err(response) = auth.require_scope(AuthScope::SessionsRead) {
                return response;
            }

            let Some(handle) = state.supervisor.get_session(&session_id).await else {
                return json_error(
                    StatusCode::NOT_FOUND,
                    "SESSION_NOT_FOUND",
                    "session not found",
                );
            };

            ws.on_upgrade(move |socket| {
                handle_session_ws(
                    socket,
                    state,
                    handle,
                    session_id,
                    auth,
                    resume_from_seq,
                    output_mode,
                )
            })
        }
        AuthMode::Token => {
            if query.token.is_some() {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "WS_QUERY_TOKEN_UNSUPPORTED",
                    "WebSocket token query parameters are not supported; send an auth message after opening the socket",
                );
            }

            ws.on_upgrade(move |socket| {
                handle_token_session_ws(socket, state, session_id, resume_from_seq, output_mode)
            })
        }
    }
}

async fn handle_session_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    handle: ActorHandle,
    session_id: String,
    auth: AuthInfo,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) {
    if let Err(err) = session_ws_inner(
        socket,
        state,
        handle,
        session_id.clone(),
        auth,
        resume_from_seq,
        output_mode,
    )
    .await
    {
        tracing::warn!(session_id, "browser attach closed with error: {err}");
    }
}

async fn handle_token_session_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    session_id: String,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) {
    if let Err(err) = token_session_ws_inner(
        socket,
        state,
        session_id.clone(),
        resume_from_seq,
        output_mode,
    )
    .await
    {
        tracing::warn!(
            session_id,
            "token-auth browser attach closed with error: {err}"
        );
    }
}

async fn session_ws_inner(
    socket: WebSocket,
    state: Arc<AppState>,
    handle: ActorHandle,
    session_id: String,
    auth: AuthInfo,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) -> anyhow::Result<()> {
    let Some((_ws_guard, sender, receiver)) = split_limited_socket(socket).await? else {
        return Ok(());
    };
    session_ws_authenticated_inner(
        sender,
        receiver,
        state,
        handle,
        session_id,
        auth,
        resume_from_seq,
        output_mode,
    )
    .await
}

async fn token_session_ws_inner(
    socket: WebSocket,
    state: Arc<AppState>,
    session_id: String,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) -> anyhow::Result<()> {
    // Do not consume a connection slot for the unauthenticated handshake: the
    // global cap is reserved for live attaches so a flood of pre-auth sockets
    // cannot starve legitimate clients while they sit in `WS_AUTH_TIMEOUT`.
    let (mut sender, mut receiver) = socket.split();

    let Some(auth) = authenticate_session_ws(&state.config, &mut sender, &mut receiver).await?
    else {
        return Ok(());
    };

    if !auth.has_scope(AuthScope::SessionsRead) {
        send_ws_error(
            &mut sender,
            "NOT_AUTHORIZED",
            "Insufficient scope for this action",
        )
        .await?;
        return Ok(());
    }

    // Acquire the connection slot only now that the client is authenticated, so
    // the post-auth cap semantics still hold for live attaches.
    let Some(_ws_guard) = acquire_ws_slot(&mut sender).await? else {
        return Ok(());
    };

    let Some(handle) = state.supervisor.get_session(&session_id).await else {
        send_ws_error(&mut sender, "SESSION_NOT_FOUND", "session not found").await?;
        return Ok(());
    };

    session_ws_authenticated_inner(
        sender,
        receiver,
        state,
        handle,
        session_id,
        auth,
        resume_from_seq,
        output_mode,
    )
    .await
}

async fn split_limited_socket(
    socket: WebSocket,
) -> anyhow::Result<Option<(ActiveWsGuard, WsSender, WsReceiver)>> {
    let (mut sender, receiver) = socket.split();
    let Some(ws_guard) = acquire_ws_slot(&mut sender).await? else {
        return Ok(None);
    };
    Ok(Some((ws_guard, sender, receiver)))
}

/// Acquire a post-auth connection slot against the global cap. When the cap is
/// exhausted, notify the client and return `None` without consuming a slot so
/// the caller can close cleanly.
async fn acquire_ws_slot(sender: &mut WsSender) -> anyhow::Result<Option<ActiveWsGuard>> {
    let Some(ws_guard) = ActiveWsGuard::try_acquire() else {
        let notice = serde_json::json!({
            "type": "overloaded",
            "code": "SERVER_OVERLOADED",
            "message": "server has too many active browser terminal attachments",
            "retryAfterMs": 5000,
        });
        sender
            .send(Message::Text(notice.to_string().into()))
            .await?;
        return Ok(None);
    };
    Ok(Some(ws_guard))
}

async fn session_ws_authenticated_inner(
    mut sender: WsSender,
    mut receiver: WsReceiver,
    state: Arc<AppState>,
    handle: ActorHandle,
    session_id: String,
    auth: AuthInfo,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) -> anyhow::Result<()> {
    let client_id = NEXT_WS_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let replay_cursor = request_replay_cursor(&handle).await?;
    let requested_resume_from_seq =
        resume_from_seq.unwrap_or_else(|| replay_cursor.replay_window_start_seq.saturating_sub(1));
    let (mut output_rx, subscribe_outcome) =
        subscribe_to_output(&state, &handle, client_id, Some(requested_resume_from_seq)).await?;
    let mut session_events = handle.subscribe_events();
    let mut thought_events = state.supervisor.subscribe_thought_events();
    let mut lifecycle_events = state.supervisor.subscribe_events();
    let summary = fetch_live_summary(&state, &session_id).await?;
    let can_write = auth.has_scope(AuthScope::StreamWrite);

    let ready_payload = build_ready_payload(
        &session_id,
        can_write,
        replay_cursor,
        requested_resume_from_seq,
        output_mode,
        &summary,
    );
    sender
        .send(Message::Text(ready_payload.to_string().into()))
        .await?;

    if let Some((notice, should_close)) = subscribe_outcome_notice(&subscribe_outcome) {
        sender
            .send(Message::Text(notice.to_string().into()))
            .await?;
        if should_close {
            return Ok(());
        }
    }

    loop {
        let event = tokio::select! {
            maybe_message = receiver.next() => match maybe_message {
                Some(message) => SessionWsEvent::Incoming(message),
                None => break,
            },
            maybe_frame = output_rx.recv() => match maybe_frame {
                Some(frame) => SessionWsEvent::Frame(frame),
                None => break,
            },
            event = session_events.recv() => SessionWsEvent::SessionControl(event),
            event = thought_events.recv() => SessionWsEvent::ThoughtControl(event),
            event = lifecycle_events.recv() => SessionWsEvent::Lifecycle(event),
        };
        if !handle_session_ws_event(&handle, &mut sender, &auth, output_mode, &session_id, event)
            .await?
        {
            break;
        }
    }

    let _ = handle.send(SessionCommand::Unsubscribe { client_id }).await;
    Ok(())
}

/// Build the `ready` handshake payload sent immediately after a client
/// authenticates. Pure; no I/O. `readOnly` mirrors the absence of write scope
/// and `protocol.output` reflects the negotiated output mode.
fn build_ready_payload(
    session_id: &str,
    can_write: bool,
    replay_cursor: ReplayCursor,
    requested_resume_from_seq: u64,
    output_mode: WsOutputMode,
    summary: &Option<SessionSummary>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "ready",
        "sessionId": session_id,
        "readOnly": !can_write,
        "replay": {
            "latestSeq": replay_cursor.latest_seq,
            "windowStartSeq": replay_cursor.replay_window_start_seq,
            "resumeFromSeq": requested_resume_from_seq,
        },
        "protocol": {
            "output": match output_mode {
                WsOutputMode::Raw => "raw",
                WsOutputMode::Framed => "framed_v1",
            },
        },
        "summary": summary,
    })
}

/// Pure mapping from a subscribe outcome to the notice payload to send (if any)
/// and whether the connection should close after sending it. No I/O. `Ok` sends
/// nothing; `Rejected` emits an overloaded notice and closes; `ReplayTruncated`
/// emits a notice and keeps streaming.
fn subscribe_outcome_notice(outcome: &SubscribeOutcome) -> Option<(serde_json::Value, bool)> {
    match outcome {
        SubscribeOutcome::Ok => None,
        SubscribeOutcome::Rejected { reason } => Some((
            serde_json::json!({
                "type": "overloaded",
                "code": "SESSION_OVERLOADED",
                "message": reason,
                "retryAfterMs": 4000,
            }),
            true,
        )),
        SubscribeOutcome::ReplayTruncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        } => Some((
            serde_json::json!({
                "type": "replay_truncated",
                "requestedResumeFromSeq": requested_resume_from_seq,
                "windowStartSeq": replay_window_start_seq,
                "latestSeq": latest_seq,
            }),
            false,
        )),
    }
}

enum SessionWsEvent {
    Incoming(Result<Message, axum::Error>),
    Frame(OutputFrame),
    SessionControl(Result<ControlEvent, broadcast::error::RecvError>),
    ThoughtControl(Result<ControlEvent, broadcast::error::RecvError>),
    Lifecycle(Result<LifecycleEvent, broadcast::error::RecvError>),
}

async fn handle_session_ws_event(
    handle: &ActorHandle,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    auth: &AuthInfo,
    output_mode: WsOutputMode,
    session_id: &str,
    event: SessionWsEvent,
) -> anyhow::Result<bool> {
    match event {
        SessionWsEvent::Incoming(Ok(message)) => {
            handle_client_message(handle, sender, auth, message).await
        }
        SessionWsEvent::Incoming(Err(err)) => Err(err.into()),
        SessionWsEvent::Frame(frame) => {
            send_output_frame(sender, frame, output_mode).await?;
            Ok(true)
        }
        SessionWsEvent::SessionControl(event) => {
            send_control_event_if_relevant(sender, session_id, "session_events", event).await
        }
        SessionWsEvent::ThoughtControl(event) => {
            send_control_event_if_relevant(sender, session_id, "thought_events", event).await
        }
        SessionWsEvent::Lifecycle(event) => {
            send_lifecycle_event_if_relevant(sender, session_id, event).await
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserClientMessage {
    Auth {
        token: String,
    },
    InputText {
        data: String,
        #[serde(default, alias = "clientMessageId")]
        client_message_id: Option<String>,
    },
    SubmitLine {
        data: String,
        #[serde(default, alias = "clientMessageId")]
        client_message_id: Option<String>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Ping,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserWsAuthMessage {
    Auth { token: String },
}

/// Pure routing decision derived from an incoming WebSocket message. No I/O.
#[derive(Debug)]
enum WsClientDecision {
    Close,
    Ignore,
    SendPong(Vec<u8>),
    ReplyPong,
    SendError {
        code: &'static str,
        message: String,
        client_message_id: Option<String>,
    },
    Forward {
        cmd: SessionCommand,
        client_message_id: Option<String>,
    },
}

fn decode_client_message(auth: &AuthInfo, message: &Message) -> WsClientDecision {
    match message {
        Message::Close(_) => WsClientDecision::Close,
        Message::Pong(_) => WsClientDecision::Ignore,
        Message::Ping(bytes) => WsClientDecision::SendPong(bytes.to_vec()),
        Message::Binary(bytes) => {
            if !auth.has_scope(AuthScope::StreamWrite) {
                return WsClientDecision::SendError {
                    code: "READ_ONLY",
                    message: "observer connections cannot send terminal input".to_string(),
                    client_message_id: None,
                };
            }
            if bytes.is_empty() {
                WsClientDecision::Ignore
            } else if bytes.len() > MAX_WS_INPUT_BYTES {
                WsClientDecision::SendError {
                    code: "INPUT_TOO_LARGE",
                    message: format!(
                        "terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit"
                    ),
                    client_message_id: None,
                }
            } else {
                WsClientDecision::Forward {
                    cmd: SessionCommand::WriteInput(bytes.to_vec()),
                    client_message_id: None,
                }
            }
        }
        Message::Text(text) => decode_text_client_message(auth, text.as_str()),
    }
}

fn oversized_input_error(client_message_id: Option<String>) -> WsClientDecision {
    WsClientDecision::SendError {
        code: "INPUT_TOO_LARGE",
        message: format!("terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit"),
        client_message_id,
    }
}

fn decode_text_client_message(auth: &AuthInfo, text: &str) -> WsClientDecision {
    let parsed: BrowserClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(err) => {
            return WsClientDecision::SendError {
                code: "INVALID_MESSAGE",
                message: format!("invalid control message: {err}"),
                client_message_id: None,
            }
        }
    };
    match parsed {
        BrowserClientMessage::Ping => WsClientDecision::ReplyPong,
        BrowserClientMessage::Auth { token: _token } => WsClientDecision::Ignore,
        BrowserClientMessage::InputText {
            data,
            client_message_id,
        } => {
            if !auth.has_scope(AuthScope::StreamWrite) {
                return WsClientDecision::SendError {
                    code: "READ_ONLY",
                    message: "observer connections cannot send terminal input".to_string(),
                    client_message_id,
                };
            }
            if data.is_empty() {
                WsClientDecision::Ignore
            } else if data.len() > MAX_WS_INPUT_BYTES {
                oversized_input_error(client_message_id)
            } else {
                WsClientDecision::Forward {
                    cmd: SessionCommand::WriteInputAck {
                        data: data.into_bytes(),
                        ack: oneshot::channel().0,
                    },
                    client_message_id,
                }
            }
        }
        BrowserClientMessage::SubmitLine {
            data,
            client_message_id,
        } => {
            if !auth.has_scope(AuthScope::StreamWrite) {
                return WsClientDecision::SendError {
                    code: "READ_ONLY",
                    message: "observer connections cannot submit terminal input".to_string(),
                    client_message_id,
                };
            }
            if data.trim().is_empty() {
                WsClientDecision::Ignore
            } else if data.len() > MAX_WS_INPUT_BYTES {
                oversized_input_error(client_message_id)
            } else {
                WsClientDecision::Forward {
                    cmd: SessionCommand::SubmitLineAck {
                        text: data,
                        ack: oneshot::channel().0,
                    },
                    client_message_id,
                }
            }
        }
        BrowserClientMessage::Resize { cols, rows } => {
            if !auth.has_scope(AuthScope::StreamWrite) {
                return WsClientDecision::SendError {
                    code: "READ_ONLY",
                    message: "observer connections cannot resize terminal sessions".to_string(),
                    client_message_id: None,
                };
            }
            let (cols, rows) = clamp_terminal_resize(cols, rows);
            WsClientDecision::Forward {
                cmd: SessionCommand::Resize { cols, rows },
                client_message_id: None,
            }
        }
    }
}

async fn handle_client_message(
    handle: &ActorHandle,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    auth: &AuthInfo,
    message: Message,
) -> anyhow::Result<bool> {
    match decode_client_message(auth, &message) {
        WsClientDecision::Close => return Ok(false),
        WsClientDecision::Ignore => {}
        WsClientDecision::SendPong(bytes) => {
            sender.send(Message::Pong(bytes.into())).await?;
        }
        WsClientDecision::ReplyPong => {
            sender
                .send(Message::Text(r#"{"type":"pong"}"#.into()))
                .await?;
        }
        WsClientDecision::SendError {
            code,
            message: msg,
            client_message_id,
        } => {
            if client_message_id.is_some() {
                send_input_ack(
                    sender,
                    client_message_id,
                    InputDeliveryResult {
                        delivered: false,
                        method: "rejected",
                        message: Some(msg.clone()),
                    },
                )
                .await?;
            }
            send_ws_error(sender, code, &msg).await?;
        }
        WsClientDecision::Forward {
            cmd,
            client_message_id,
        } => {
            forward_ws_command(handle, sender, cmd, client_message_id).await?;
        }
    }

    Ok(true)
}

async fn forward_ws_command(
    handle: &ActorHandle,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    cmd: SessionCommand,
    client_message_id: Option<String>,
) -> anyhow::Result<()> {
    match cmd {
        SessionCommand::WriteInputAck { data, .. } => {
            let (ack_tx, ack_rx) = oneshot::channel();
            handle
                .send(SessionCommand::WriteInputAck { data, ack: ack_tx })
                .await
                .map_err(|err| anyhow::anyhow!("failed to forward command: {err}"))?;
            send_delivery_ack(sender, client_message_id, ack_rx).await?;
        }
        SessionCommand::SubmitLineAck { text, .. } => {
            let (ack_tx, ack_rx) = oneshot::channel();
            handle
                .send(SessionCommand::SubmitLineAck { text, ack: ack_tx })
                .await
                .map_err(|err| anyhow::anyhow!("failed to forward command: {err}"))?;
            send_delivery_ack(sender, client_message_id, ack_rx).await?;
        }
        other => {
            handle
                .send(other)
                .await
                .map_err(|err| anyhow::anyhow!("failed to forward command: {err}"))?;
        }
    }
    Ok(())
}

async fn send_delivery_ack(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    client_message_id: Option<String>,
    ack_rx: oneshot::Receiver<InputDeliveryResult>,
) -> anyhow::Result<()> {
    let delivery = match tokio::time::timeout(REPLY_TIMEOUT, ack_rx).await {
        Ok(Ok(delivery)) => delivery,
        Ok(Err(_)) => InputDeliveryResult {
            delivered: false,
            method: "unknown",
            message: Some("session actor dropped input delivery ack".to_string()),
        },
        Err(_) => InputDeliveryResult {
            delivered: false,
            method: "timeout",
            message: Some("timed out waiting for input delivery confirmation".to_string()),
        },
    };
    send_input_ack(sender, client_message_id, delivery).await
}

async fn send_input_ack(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    client_message_id: Option<String>,
    delivery: InputDeliveryResult,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "type": "input_ack",
        "clientMessageId": client_message_id,
        "delivered": delivery.delivered,
        "method": delivery.method,
        "message": delivery.message,
    });
    sender
        .send(Message::Text(payload.to_string().into()))
        .await?;
    Ok(())
}

async fn send_ws_error(sender: &mut WsSender, code: &str, message: &str) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "type": "error",
        "code": code,
        "message": message,
    });
    sender
        .send(Message::Text(payload.to_string().into()))
        .await?;
    Ok(())
}

#[derive(Debug)]
enum WsAuthDecision {
    Authenticated(AuthInfo),
    Close,
    Reject {
        code: &'static str,
        message: &'static str,
    },
}

async fn authenticate_session_ws(
    config: &Config,
    sender: &mut WsSender,
    receiver: &mut WsReceiver,
) -> anyhow::Result<Option<AuthInfo>> {
    let decision = match tokio::time::timeout(WS_AUTH_TIMEOUT, receiver.next()).await {
        Ok(Some(Ok(message))) => decode_ws_auth_message(config, &message),
        Ok(Some(Err(err))) => return Err(err.into()),
        Ok(None) => WsAuthDecision::Close,
        Err(_) => WsAuthDecision::Reject {
            code: "WS_AUTH_TIMEOUT",
            message: "token-mode WebSocket connections must authenticate before terminal traffic",
        },
    };

    match decision {
        WsAuthDecision::Authenticated(auth) => Ok(Some(auth)),
        WsAuthDecision::Close => Ok(None),
        WsAuthDecision::Reject { code, message } => {
            send_ws_error(sender, code, message).await?;
            Ok(None)
        }
    }
}

fn decode_ws_auth_message(config: &Config, message: &Message) -> WsAuthDecision {
    let Message::Text(text) = message else {
        return WsAuthDecision::Reject {
            code: "WS_AUTH_REQUIRED",
            message:
                "token-mode WebSocket connections must send an auth message before terminal traffic",
        };
    };

    let parsed: BrowserWsAuthMessage = match serde_json::from_str(text.as_str()) {
        Ok(message) => message,
        Err(_) => {
            return WsAuthDecision::Reject {
                code: "WS_AUTH_REQUIRED",
                message:
                    "token-mode WebSocket connections must send an auth message before terminal traffic",
            };
        }
    };

    match parsed {
        BrowserWsAuthMessage::Auth { token } => match resolve_ws_auth(config, Some(token.as_str()))
        {
            Ok(auth) => WsAuthDecision::Authenticated(auth),
            Err(_) => WsAuthDecision::Reject {
                code: "NOT_AUTHENTICATED",
                message: "Missing or invalid authentication token",
            },
        },
    }
}

async fn send_control_event_if_relevant(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    session_id: &str,
    stream: &'static str,
    event: Result<ControlEvent, broadcast::error::RecvError>,
) -> anyhow::Result<bool> {
    match event {
        Ok(event) => {
            if event.session_id == session_id {
                send_ws_json(sender, control_event_ws_payload(&event)).await?;
            }
        }
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            send_ws_json(sender, event_stream_lagged_payload(stream, skipped)).await?;
        }
        Err(broadcast::error::RecvError::Closed) => {}
    }
    Ok(true)
}

async fn send_lifecycle_event_if_relevant(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    session_id: &str,
    event: Result<LifecycleEvent, broadcast::error::RecvError>,
) -> anyhow::Result<bool> {
    match event {
        Ok(event) => {
            if lifecycle_event_session_id(&event) == session_id {
                send_ws_json(sender, lifecycle_event_ws_payload(&event)).await?;
            }
        }
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            send_ws_json(
                sender,
                event_stream_lagged_payload("lifecycle_events", skipped),
            )
            .await?;
        }
        Err(broadcast::error::RecvError::Closed) => {}
    }
    Ok(true)
}

async fn send_ws_json(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    sender
        .send(Message::Text(payload.to_string().into()))
        .await?;
    Ok(())
}

fn control_event_ws_payload(event: &ControlEvent) -> serde_json::Value {
    let contract = event.payload_contract();
    serde_json::json!({
        "type": "control_event",
        "event": contract.event_name(),
        "sessionId": event.session_id,
        "payload": contract.payload_value(),
    })
}

fn lifecycle_event_session_id(event: &LifecycleEvent) -> &str {
    match event {
        LifecycleEvent::Created { session_id, .. } | LifecycleEvent::Deleted { session_id, .. } => {
            session_id
        }
    }
}

fn lifecycle_event_ws_payload(event: &LifecycleEvent) -> serde_json::Value {
    match event {
        LifecycleEvent::Created {
            session_id,
            summary,
            reason,
            repo_theme,
        } => serde_json::json!({
            "type": "lifecycle_event",
            "event": "session_created",
            "sessionId": session_id,
            "reason": reason,
            "summary": summary,
            "repoTheme": repo_theme,
        }),
        LifecycleEvent::Deleted {
            session_id,
            reason,
            delete_mode,
            tmux_session_alive,
        } => serde_json::json!({
            "type": "lifecycle_event",
            "event": "session_deleted",
            "sessionId": session_id,
            "reason": reason,
            "deleteMode": delete_mode,
            "tmuxSessionAlive": tmux_session_alive,
        }),
    }
}

fn event_stream_lagged_payload(stream: &str, skipped: u64) -> serde_json::Value {
    serde_json::json!({
        "type": "event_stream_lagged",
        "stream": stream,
        "skipped": skipped,
    })
}

async fn send_output_frame(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    frame: OutputFrame,
    output_mode: WsOutputMode,
) -> anyhow::Result<()> {
    match output_mode {
        WsOutputMode::Raw => {
            sender.send(Message::Binary(frame.data.into())).await?;
        }
        WsOutputMode::Framed => {
            sender
                .send(Message::Binary(encode_terminal_output_frame(frame).into()))
                .await?;
        }
    }
    Ok(())
}

fn encode_terminal_output_frame(frame: OutputFrame) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + std::mem::size_of::<u64>() + frame.data.len());
    bytes.push(opcodes::TERMINAL_OUTPUT);
    bytes.extend_from_slice(&frame.seq.to_be_bytes());
    bytes.extend_from_slice(&frame.data);
    bytes
}

fn query_flag_enabled(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "framed" | "framed_v1"
    )
}

async fn subscribe_to_output(
    state: &Arc<AppState>,
    handle: &ActorHandle,
    client_id: u64,
    resume_from_seq: Option<u64>,
) -> anyhow::Result<(mpsc::Receiver<OutputFrame>, SubscribeOutcome)> {
    let (client_tx, client_rx) = mpsc::channel(state.config.outbound_queue_bound.max(64));
    let (ack_tx, ack_rx) = oneshot::channel();
    handle
        .send(SessionCommand::Subscribe {
            client_id,
            client_tx,
            resume_from_seq,
            ack: ack_tx,
        })
        .await
        .map_err(|err| anyhow::anyhow!("failed to subscribe to session output: {err}"))?;

    let outcome = tokio::time::timeout(REPLY_TIMEOUT, ack_rx)
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for subscribe ack"))?
        .map_err(|_| anyhow::anyhow!("session actor dropped subscribe ack"))?;

    Ok((client_rx, outcome))
}

async fn request_replay_cursor(handle: &ActorHandle) -> anyhow::Result<ReplayCursor> {
    let (tx, rx) = oneshot::channel();
    handle
        .send(SessionCommand::GetReplayCursor(tx))
        .await
        .map_err(|err| anyhow::anyhow!("failed to request replay cursor: {err}"))?;

    tokio::time::timeout(REPLY_TIMEOUT, rx)
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for replay cursor"))?
        .map_err(|_| anyhow::anyhow!("session actor dropped replay cursor"))
}

#[allow(clippy::result_large_err)]
fn resolve_ws_auth(config: &Config, token: Option<&str>) -> Result<AuthInfo, Response> {
    match config.auth_mode {
        AuthMode::LocalTrust | AuthMode::TailnetTrust => {
            Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec()))
        }
        AuthMode::Token => {
            // Reject a missing or empty token outright. Empty `AUTH_TOKEN`/
            // `OBSERVER_TOKEN` are already filtered at config load, so this is
            // defense-in-depth that mirrors the HTTP `extract_bearer_token`
            // empty-token guard and keeps empty WebSocket auth frames from
            // ever matching.
            let Some(token) = token.filter(|t| !t.is_empty()) else {
                return Err(json_error(
                    StatusCode::UNAUTHORIZED,
                    "NOT_AUTHENTICATED",
                    "Missing or invalid authentication token",
                ));
            };

            if config
                .auth_token
                .as_deref()
                .is_some_and(|expected| bearer_tokens_eq(token, expected))
            {
                return Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec()));
            }

            if config
                .observer_token
                .as_deref()
                .is_some_and(|expected| bearer_tokens_eq(token, expected))
            {
                return Ok(AuthInfo::new(OBSERVER_SCOPES.to_vec()));
            }

            Err(json_error(
                StatusCode::UNAUTHORIZED,
                "NOT_AUTHENTICATED",
                "Missing or invalid authentication token",
            ))
        }
    }
}

fn bearer_tokens_eq(provided: &str, expected: &str) -> bool {
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn resolve_frankentui_pkg_dir() -> Option<PathBuf> {
    for key in ["SWIMMERS_FRANKENTUI_PKG_DIR", "FRANKENTUI_PKG_DIR"] {
        if let Some(path) = std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            let candidate = PathBuf::from(path);
            if valid_frankentui_pkg_dir(&candidate) {
                return Some(candidate);
            }
        }
    }

    DEFAULT_FRANKENTUI_PKG_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|candidate| valid_frankentui_pkg_dir(candidate))
}

fn valid_frankentui_pkg_dir(path: &Path) -> bool {
    path.join("FrankenTerm.js").is_file() && path.join("FrankenTerm_bg.wasm").is_file()
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(error_body_msg(code, message))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{KnownControlEventPayload, SessionTitlePayload};
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use tempfile::tempdir;

    async fn html_string(response: impl IntoResponse) -> String {
        let response = response.into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("html body");
        String::from_utf8(body.to_vec()).expect("utf8 html")
    }

    #[test]
    fn valid_pkg_dir_requires_js_and_wasm() {
        let dir = tempdir().expect("tempdir");
        assert!(!valid_frankentui_pkg_dir(dir.path()));

        std::fs::write(
            dir.path().join("FrankenTerm.js"),
            "export default async () => {}",
        )
        .expect("write js");
        assert!(!valid_frankentui_pkg_dir(dir.path()));

        std::fs::write(dir.path().join("FrankenTerm_bg.wasm"), b"wasm").expect("write wasm");
        assert!(valid_frankentui_pkg_dir(dir.path()));
    }

    #[test]
    fn websocket_first_message_auth_accepts_observer_and_operator_tokens() {
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("operator".into()),
            observer_token: Some("observer".into()),
            ..Config::default()
        };

        let operator_msg = Message::Text(r#"{"type":"auth","token":"operator"}"#.into());
        match decode_ws_auth_message(&config, &operator_msg) {
            WsAuthDecision::Authenticated(operator) => {
                assert!(operator.has_scope(AuthScope::StreamWrite));
            }
            other => panic!("unexpected operator auth decision: {other:?}"),
        }

        let observer_msg = Message::Text(r#"{"type":"auth","token":"observer"}"#.into());
        match decode_ws_auth_message(&config, &observer_msg) {
            WsAuthDecision::Authenticated(observer) => {
                assert!(observer.has_scope(AuthScope::SessionsRead));
                assert!(!observer.has_scope(AuthScope::StreamWrite));
            }
            other => panic!("unexpected observer auth decision: {other:?}"),
        }
    }

    #[test]
    fn websocket_first_message_auth_rejects_invalid_and_non_auth_messages() {
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("operator".into()),
            observer_token: Some("observer".into()),
            ..Config::default()
        };

        for msg in [
            Message::Text(r#"{"type":"ping"}"#.into()),
            Message::Text(r#"{"type":"auth","token":""}"#.into()),
            Message::Binary(b"operator".to_vec().into()),
        ] {
            assert!(
                matches!(
                    decode_ws_auth_message(&config, &msg),
                    WsAuthDecision::Reject { .. }
                ),
                "{msg:?}"
            );
        }
    }

    #[test]
    fn query_flag_enabled_accepts_framed_values_only() {
        for value in ["1", "true", "yes", "on", "framed", "framed_v1"] {
            assert!(query_flag_enabled(Some(value)), "{value}");
        }
        for value in [None, Some(""), Some("0"), Some("false"), Some("raw")] {
            assert!(!query_flag_enabled(value), "{value:?}");
        }
    }

    #[test]
    fn framed_terminal_output_prefixes_opcode_and_sequence() {
        let frame = OutputFrame {
            seq: 0x0102_0304_0506_0708,
            data: b"hello".to_vec(),
        };

        let encoded = encode_terminal_output_frame(frame);

        assert_eq!(encoded[0], opcodes::TERMINAL_OUTPUT);
        let seq = u64::from_be_bytes(encoded[1..9].try_into().expect("seq bytes"));
        assert_eq!(seq, 0x0102_0304_0506_0708);
        assert_eq!(&encoded[9..], b"hello");
    }

    #[test]
    fn control_event_ws_payload_preserves_session_event_and_payload() {
        let event = ControlEvent {
            event: "session_state".to_string(),
            session_id: "sess_0".to_string(),
            payload: serde_json::json!({
                "state": "attention",
                "previous_state": "idle",
                "current_command": "cargo test",
                "transport_health": "healthy",
                "at": "2026-05-15T10:00:00Z",
            }),
        };

        let payload = control_event_ws_payload(&event);

        assert_eq!(payload["type"], "control_event");
        assert_eq!(payload["event"], "session_state");
        assert_eq!(payload["sessionId"], "sess_0");
        assert_eq!(payload["payload"]["state"], "attention");
        assert_eq!(payload["payload"]["current_command"], "cargo test");
    }

    #[test]
    fn control_event_ws_payload_serializes_title_and_thought_update_contracts() {
        let title = ControlEvent {
            event: "session_title".to_string(),
            session_id: "sess_0".to_string(),
            payload: serde_json::json!({
                "title": "/tmp/swimmers",
                "at": "2026-05-15T10:00:02Z",
            }),
        };
        let title_payload = control_event_ws_payload(&title);
        assert_eq!(title_payload["event"], "session_title");
        assert_eq!(title_payload["payload"]["title"], "/tmp/swimmers");

        let thought = ControlEvent {
            event: "thought_update".to_string(),
            session_id: "sess_0".to_string(),
            payload: serde_json::json!({
                "thought": "operator response needed",
                "token_count": 64000,
                "context_limit": 128000,
                "thought_state": "holding",
                "thought_source": "llm",
                "rest_state": "active",
                "commit_candidate": true,
                "objective_changed": true,
                "at": "2026-05-15T10:00:04Z",
            }),
        };
        let thought_payload = control_event_ws_payload(&thought);
        assert_eq!(thought_payload["event"], "thought_update");
        assert_eq!(thought_payload["payload"]["token_count"], 64000);
        assert_eq!(thought_payload["payload"]["commit_candidate"], true);
    }

    #[test]
    fn control_event_payload_contract_preserves_unknown_events() {
        let event = ControlEvent {
            event: "future_event".to_string(),
            session_id: "sess_future".to_string(),
            payload: serde_json::json!({ "newField": true }),
        };

        let payload = control_event_ws_payload(&event);

        assert_eq!(payload["type"], "control_event");
        assert_eq!(payload["event"], "future_event");
        assert_eq!(payload["sessionId"], "sess_future");
        assert_eq!(payload["payload"]["newField"], true);
    }

    #[test]
    fn known_control_event_payload_uses_tagged_serde_shape() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-05-15T10:00:02Z")
            .expect("timestamp")
            .with_timezone(&chrono::Utc);
        let tagged = serde_json::to_value(KnownControlEventPayload::SessionTitle(
            SessionTitlePayload {
                title: "/tmp/swimmers".to_string(),
                at,
            },
        ))
        .expect("tagged control event");

        assert_eq!(tagged["event"], "session_title");
        assert_eq!(tagged["payload"]["title"], "/tmp/swimmers");
    }

    #[test]
    fn lifecycle_event_ws_payload_reports_deleted_session() {
        let event = LifecycleEvent::Deleted {
            session_id: "sess_0".to_string(),
            reason: "tmux_reconcile_missing".to_string(),
            delete_mode: crate::config::SessionDeleteMode::DetachBridge,
            tmux_session_alive: false,
        };

        let payload = lifecycle_event_ws_payload(&event);

        assert_eq!(lifecycle_event_session_id(&event), "sess_0");
        assert_eq!(payload["type"], "lifecycle_event");
        assert_eq!(payload["event"], "session_deleted");
        assert_eq!(payload["sessionId"], "sess_0");
        assert_eq!(payload["reason"], "tmux_reconcile_missing");
        assert_eq!(payload["deleteMode"], "detach_bridge");
        assert_eq!(payload["tmuxSessionAlive"], false);
    }

    #[test]
    fn event_stream_lagged_payload_keeps_replay_quality_visible() {
        let payload = event_stream_lagged_payload("thought_events", 7);

        assert_eq!(payload["type"], "event_stream_lagged");
        assert_eq!(payload["stream"], "thought_events");
        assert_eq!(payload["skipped"], 7);
    }

    #[tokio::test]
    async fn index_shell_includes_new_web_parity_sheets() {
        let html = html_string(render_index(false).await).await;
        assert!(html.contains("thought-config-sheet"));
        assert!(html.contains("native-sheet"));
        assert!(html.contains("mermaid-sheet"));
        assert!(html.contains("mermaid-plan-tabs"));
        assert!(html.contains("dirs-list"));
        assert!(html.contains("dirs-search"));
        assert!(html.contains("create-batch-visible"));
        assert!(html.contains("dirs-spawn-here"));
        assert!(html.contains("create-launch-target"));
        assert!(html.contains("mobile-kb-proxy"));
        assert!(html.contains("terminal-control-strip"));
        assert!(html.contains("terminal-workbench"));
        assert!(html.contains("terminal-workbench-toggle"));
        assert!(html.contains("terminal-trogdor-back"));
        assert!(html.contains("terminal-workbench-pressure"));
        assert!(html.contains("terminal-workbench-actions"));
        assert!(html.contains("terminal-workbench-widgets"));
        assert!(html.contains("terminal-input-dock"));
        assert!(html.contains("terminal-inline-input"));
        assert!(html.contains("terminal-key-strip"));
        assert!(html.contains("data-terminal-key=\"ctrl-c\""));
        assert!(html.contains("data-terminal-key=\"arrow-up\""));
        assert!(html.contains("palette-sheet"));
        assert!(html.contains("terminal-a11y-mirror"));
        assert!(html.contains("terminal-status-strip"));
        assert!(html.contains("terminal-link-tools"));
        assert!(html.contains("send-mode"));
        assert!(html.contains("send-history"));
        assert!(html.contains(FRANKENTERM_FONT_ROUTE));
        assert!(html.contains("window.__SWIMMERS_BOOT__"));
    }

    #[tokio::test]
    async fn index_boot_payload_includes_frankenterm_asset_manifest_fields() {
        let html = html_string(render_index(false).await).await;
        assert!(html.contains("\"franken_term_font_url\""));
        assert!(html.contains("\"franken_term_asset_info\""));
        assert!(html.contains(FRANKENTERM_FONT_ROUTE));
    }

    #[tokio::test]
    async fn browser_js_asset_handlers_cover_app_module_graph() {
        let assets = [
            (APP_JS_ROUTE, app_js().await, "from \"./dir_browser.js\""),
            (
                RENDERED_SURFACE_JS_ROUTE,
                rendered_surface_js().await,
                "export function buildSurfaceFrame",
            ),
            (
                INPUT_SUPPORT_JS_ROUTE,
                input_support_js().await,
                "export function eventCell",
            ),
            (
                SEND_SHEET_JS_ROUTE,
                send_sheet_js().await,
                "export function sendSheetSubmitPlan",
            ),
            (
                TERMINAL_SURFACE_SETUP_JS_ROUTE,
                terminal_surface_setup_js().await,
                "export async function initializeTerminalSurface",
            ),
            (
                TERMINAL_RESIZE_JS_ROUTE,
                terminal_resize_js().await,
                "export function runTerminalSurfaceResize",
            ),
            (
                GLOBAL_SHORTCUT_DISPATCH_JS_ROUTE,
                global_shortcut_dispatch_js().await,
                "export function runGlobalShortcutAction",
            ),
            (
                SESSION_REFRESH_JS_ROUTE,
                session_refresh_js().await,
                "export async function runSessionRefresh",
            ),
            (
                MERMAID_ARTIFACT_JS_ROUTE,
                mermaid_artifact_js().await,
                "export function boundedArtifactText",
            ),
            (
                TERMINAL_SAFETY_JS_ROUTE,
                terminal_safety_js().await,
                "export function safeAnchorHref",
            ),
            (
                TERMINAL_PROTOCOL_JS_ROUTE,
                terminal_protocol_js().await,
                "export function buildSessionSocketUrl",
            ),
            (
                DIR_BROWSER_JS_ROUTE,
                dir_browser_js().await,
                "export function renderDirEntries",
            ),
            (
                COMMAND_PALETTE_JS_ROUTE,
                command_palette_js().await,
                "export function commandPaletteExecutionPlan",
            ),
            (
                TROGDOR_LOGIC_JS_ROUTE,
                trogdor_logic_js().await,
                "export function trogdorDragonPose",
            ),
            (
                TROGDOR_RENDER_JS_ROUTE,
                trogdor_render_js().await,
                "export function renderTrogdorSurfaceFrame",
            ),
            (
                WORKBENCH_RENDER_JS_ROUTE,
                workbench_render_js().await,
                "export function buildWorkbenchWidgetsHtml",
            ),
            (
                WORKBENCH_REFRESH_JS_ROUTE,
                workbench_refresh_js().await,
                "export async function runWorkbenchWidgetRefresh",
            ),
            (
                WORKBENCH_RECORDS_JS_ROUTE,
                workbench_records_js().await,
                "export function transcriptRecordDisplay",
            ),
        ];

        for (route, response, needle) in assets {
            assert_eq!(response.status(), StatusCode::OK, "{route}");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/javascript; charset=utf-8",
                "{route}"
            );
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("js body");
            let text = String::from_utf8(body.to_vec()).expect("utf8 js asset");
            assert!(text.contains(needle), "{route} did not contain {needle}");
        }
    }

    #[tokio::test]
    async fn franken_term_asset_file_info_reports_size_and_crc() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("FrankenTerm.js");
        let contents = b"export default async () => {}";
        std::fs::write(&path, contents).expect("write asset");

        let info = franken_term_asset_file_info(&path, FRANKENTERM_JS_ROUTE)
            .await
            .expect("asset info");

        assert_eq!(info.route, FRANKENTERM_JS_ROUTE);
        assert_eq!(info.size_bytes, contents.len() as u64);
        assert_eq!(
            info.checksum,
            format!("crc32:{:08x}", crc32fast::hash(contents))
        );
    }

    #[tokio::test]
    async fn trogdor_dragon_asset_route_serves_embedded_png() {
        // Spot-check three combinations spanning the expanded 8 poses × 8 frames
        // grid: the legacy mouth-closed/right pair, the newly-served mouth-open
        // pose, and one of the previously-unreachable 3/4-view body frames.
        let cases = [
            ("mouth-closed", "right.png"),
            ("mouth-open", "back-left.png"),
            ("fire-right-mid", "3q-left.png"),
        ];
        for (pose, frame) in cases {
            let response = trogdor_dragon_asset(AxumPath((pose.to_string(), frame.to_string())))
                .await
                .into_response();

            assert_eq!(response.status(), StatusCode::OK, "{pose}/{frame}");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "image/png",
                "{pose}/{frame}"
            );
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("dragon sprite body");
            assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n", "{pose}/{frame}");
        }

        // Unknown frames still 404.
        let response = trogdor_dragon_asset(AxumPath((
            "mouth-closed".to_string(),
            "diagonal.png".to_string(),
        )))
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn app_js_and_css_wire_trogdor_sprite_burn_loop() {
        let js = include_str!("app.js");
        let trogdor_logic = include_str!("trogdor_logic.js");
        let trogdor_render = include_str!("trogdor_render.js");
        assert!(trogdor_render.contains("TROGDOR_DRAGON_ASSET_BASE = \"/assets/dragon\""));
        assert!(trogdor_logic.contains("export function trogdorDragonPose(groups, summary"));
        assert!(trogdor_render.contains("trogdor-dragon-sprite"));
        assert!(trogdor_render.contains("agent-burn-flame"));
        assert!(trogdor_render.contains("const dragonTarget = dragonPose || TROGDOR_DRAGON_TARGET"));
        assert!(js.contains("renderTrogdorSurfaceFrame"));

        let css = include_str!("app.css");
        assert!(css.contains("@keyframes dragon-walk-around"));
        assert!(css.contains("@keyframes dragon-sprite-fire"));
        assert!(css.contains(".agent-burn-flame"));
        assert!(css.contains("@media (prefers-reduced-motion: reduce)"));
    }

    #[test]
    fn directory_browser_modules_include_overlay_filter_chip_logic() {
        let js = include_str!("app.js");
        let dir_browser = include_str!("dir_browser.js");
        assert!(dir_browser.contains("response?.overlay_label"));
        assert!(dir_browser.contains("managedButton.dataset.filter = \"managed\""));
        assert!(dir_browser.contains("allButton.textContent = \"all folders\""));
        assert!(js.contains("filter === \"managed\" ? true"));
    }

    #[test]
    fn app_js_retries_saved_out_of_base_dir_from_default_base() {
        let js = include_str!("app.js");
        assert!(js.contains("shouldRetryDirListingFromBase"));
        assert!(js.contains("localStorage.removeItem(DIR_BROWSER_PATH_KEY)"));
        assert!(
            js.contains("return loadDirListing(\"\", managed, \"\", { retriedFromBase: true })")
        );
        assert!(js.contains("outside the allowed base directory"));
        assert!(js.contains("rawStoredDirPath.trim() === \"/\" ? \"\" : rawStoredDirPath"));
    }

    #[test]
    fn app_js_exposes_terminal_viewer_ergonomics() {
        let js = include_str!("app.js");
        let workbench_render = include_str!("workbench_render.js");
        assert!(js.contains("TERMINAL_ZOOM_STORAGE_KEY"));
        assert!(js.contains("setZoom"));
        assert!(js.contains("focusMobileKeyboard"));
        assert!(js.contains("mobileKeyboardProxy"));
        assert!(js.contains("function openCommandPalette()"));
        assert!(js.contains("function syncTerminalAccessibilityMirror"));
        assert!(js.contains("function drainTerminalLinkClicks()"));
        assert!(js.contains("function rememberSendHistory(text)"));
        assert!(js.contains("function syncTerminalStatusStrip()"));
        assert!(js.contains("function refreshAgentContextForSelectedSession"));
        assert!(js.contains("function refreshWorkbenchWidgetsForSelectedSession"));
        assert!(workbench_render.contains("export function operatorPressureSummary"));
        assert!(workbench_render.contains("Tool calls"));
        assert!(js.contains("/agent-context"));
        assert!(js.contains("/pane-tail"));
        assert!(js.contains("/transcript"));
        assert!(workbench_render.contains("function renderTurnsPanel"));
        assert!(js.contains("function flushPendingTerminalBytes"));
        assert!(workbench_render.contains("Post-turn JSONL"));
        assert!(js.contains("/mermaid-artifact"));
        assert!(js.contains("/git-diff"));
        assert!(workbench_render.contains("function renderDiffHtml"));
        assert!(js.contains("function syncTerminalWorkbench()"));
    }

    #[test]
    fn app_js_dedupes_surface_actions_and_stable_resizes() {
        let js = include_str!("app.js");
        let resize = include_str!("terminal_resize.js");
        assert!(js.contains("function stopSurfaceEvent(event)"));
        assert!(js.contains("event.stopImmediatePropagation"));
        assert!(
            js.contains("runTerminalSurfaceResize({ pushResize, force }, terminalResizeRuntime)")
        );
        assert!(resize.contains("terminalResizeGeometryPlan({"));
        assert!(resize.contains("if (!resizePlan.shouldResize)"));
        assert!(js.contains("queueMeasureAndResizeSurface(true, false)"));
    }

    #[test]
    fn app_js_hides_hud_when_live_terminal_is_focused() {
        let js = include_str!("app.js");
        assert!(js.contains("function syncTerminalPresentation()"));
        assert!(js.contains("terminal-focus-mode"));
        assert!(
            js.contains("terminalPresentationPlan({ hasCurrentSession: Boolean(currentSession())")
        );
        assert!(js.contains("el.hudCanvas.classList.toggle(\"hidden\", plan.hudHidden)"));
        assert!(
            js.contains("[el.hudCanvas.style.display, el.hudCanvas.style.visibility] = [plan.hudDisplay, plan.hudVisibility]")
        );
        assert!(js
            .contains("el.terminalCanvas.classList.toggle(\"hidden\", plan.terminalCanvasHidden)"));
    }

    #[test]
    fn app_js_falls_back_when_live_terminal_canvas_does_not_paint() {
        let js = include_str!("app.js");
        assert!(js.contains("function feedTerminalBytes(bytes)"));
        assert!(js.contains("flushEncodedInputBytes();"));
        assert!(js.contains("function terminalCanvasHasVisiblePixels()"));
        assert!(js.contains("function verifyTerminalPaintOrFallback()"));
        assert!(js.contains("setTerminalTextFallbackActive(true, { clearText: false })"));
        assert!(js.contains("function sendFallbackTerminalEvent(event)"));
        assert!(js.contains("function updateTerminalFallbackText(text)"));
        assert!(js.contains("function terminalFallbackOwnsPointer(event)"));
        let css = include_str!("app.css");
        assert!(css.contains("white-space: pre-wrap"));
        assert!(css.contains("overflow-wrap: anywhere"));
        assert!(css.contains("pointer-events: auto"));
    }

    #[test]
    fn app_js_trogdor_agent_click_opens_terminal() {
        let js = include_str!("app.js");
        assert!(js.contains("function closeTrogdorAtlasForTerminal()"));
        assert!(js.contains("function openTrogdorAtlas()"));
        assert!(js.contains("terminalTrogdorBack"));
        assert!(js.contains("function sendTerminalControlKey(actionId)"));
        assert!(js.contains("terminalKeyActionForDomEvent(event)"));
        assert!(js.contains("async function openTrogdorAgentTerminal(sessionId)"));
        assert!(js.contains("state.trogdorAtlasOpen = false"));
        assert!(js.contains("state.trogdorAtlasOpen = true"));
        assert!(js.contains("state.hoveredTrogdorSessionId = null"));
        assert!(js.contains("function applyTrogdorAtlasVisibility()"));
        assert!(js.contains("el.trogdorSurface.style.display = visible ? \"\" : \"none\""));
        assert!(js.contains("document.body.classList.toggle(\"trogdor-mode\", visible)"));
        assert!(js.contains("closeTrogdorAtlasForTerminal();"));
        assert!(js.contains("closeTrogdorAtlasForTerminal()"));
        assert!(js.contains("await selectSession(normalized)"));
        assert!(js.contains("focusTerminalInputSurface({ preventScroll: true })"));
        assert!(js.contains("refreshAgentContextForSelectedSession({ force: true })"));
        assert!(js.contains("await openTrogdorAgentTerminal(zone.sessionId)"));
    }

    #[tokio::test]
    async fn published_route_shell_sets_follow_focus_mode() {
        let html = html_string(render_index(true).await).await;
        assert!(html.contains("published-focus"));
        assert!(html.contains("follow_published_selection"));
    }

    // --- decode_client_message unit tests ---

    fn operator_auth() -> AuthInfo {
        AuthInfo::new(OPERATOR_SCOPES.to_vec())
    }

    fn observer_auth() -> AuthInfo {
        AuthInfo::new(OBSERVER_SCOPES.to_vec())
    }

    // --- build_ready_payload / subscribe_outcome_notice (handshake branches) ---

    #[test]
    fn build_ready_payload_observer_is_read_only_with_framed_protocol() {
        let cursor = ReplayCursor {
            latest_seq: 42,
            replay_window_start_seq: 10,
        };
        let payload = build_ready_payload("sess-1", false, cursor, 7, WsOutputMode::Framed, &None);
        assert_eq!(payload["type"], "ready");
        assert_eq!(payload["sessionId"], "sess-1");
        assert_eq!(payload["readOnly"], true);
        assert_eq!(payload["replay"]["latestSeq"], 42);
        assert_eq!(payload["replay"]["windowStartSeq"], 10);
        assert_eq!(payload["replay"]["resumeFromSeq"], 7);
        assert_eq!(payload["protocol"]["output"], "framed_v1");
        assert_eq!(payload["summary"], serde_json::Value::Null);
    }

    #[test]
    fn build_ready_payload_writer_is_not_read_only_with_raw_protocol() {
        let cursor = ReplayCursor {
            latest_seq: 1,
            replay_window_start_seq: 0,
        };
        let payload = build_ready_payload("s", true, cursor, 0, WsOutputMode::Raw, &None);
        assert_eq!(payload["readOnly"], false);
        assert_eq!(payload["protocol"]["output"], "raw");
    }

    #[test]
    fn subscribe_outcome_notice_ok_sends_nothing() {
        assert!(subscribe_outcome_notice(&SubscribeOutcome::Ok).is_none());
    }

    #[test]
    fn subscribe_outcome_notice_rejected_overloads_and_closes() {
        let (notice, should_close) = subscribe_outcome_notice(&SubscribeOutcome::Rejected {
            reason: "session is busy".to_string(),
        })
        .expect("rejected outcome should produce a notice");
        assert!(
            should_close,
            "a rejected subscription must close the socket"
        );
        assert_eq!(notice["type"], "overloaded");
        assert_eq!(notice["code"], "SESSION_OVERLOADED");
        assert_eq!(notice["message"], "session is busy");
        assert_eq!(notice["retryAfterMs"], 4000);
    }

    #[test]
    fn subscribe_outcome_notice_replay_truncated_notifies_and_continues() {
        let (notice, should_close) = subscribe_outcome_notice(&SubscribeOutcome::ReplayTruncated {
            requested_resume_from_seq: 5,
            replay_window_start_seq: 9,
            latest_seq: 20,
        })
        .expect("replay-truncated outcome should produce a notice");
        assert!(
            !should_close,
            "replay-truncated must keep streaming, not close"
        );
        assert_eq!(notice["type"], "replay_truncated");
        assert_eq!(notice["requestedResumeFromSeq"], 5);
        assert_eq!(notice["windowStartSeq"], 9);
        assert_eq!(notice["latestSeq"], 20);
    }

    // --- session_ws_authenticated_inner live-socket integration ---

    fn test_state() -> Arc<AppState> {
        use tokio::sync::RwLock;
        let config = Arc::new(Config::default());
        let supervisor = crate::session::supervisor::SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(
                crate::thought::runtime_config::ThoughtConfig::default(),
            )),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(crate::thought::protocol::SyncRequestSequence::new()),
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
                std::time::Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(
                crate::api::PublishedSelectionState::default(),
            )),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    // Drives the real authenticated WS loop end-to-end over a live socket. Under
    // LocalTrust there is no first-frame handshake, so connecting to a live
    // session exercises session_ws_authenticated_inner through the replay-cursor
    // request, output subscribe (Ok), summary fetch, ready payload, and teardown.
    // A minimal fake actor answers the three handshake commands the path needs.
    #[tokio::test]
    async fn session_ws_authenticated_inner_streams_ready_payload_over_live_socket() {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(16);
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetReplayCursor(reply) => {
                        let _ = reply.send(ReplayCursor {
                            latest_seq: 5,
                            replay_window_start_seq: 1,
                        });
                    }
                    SessionCommand::Subscribe { ack, .. } => {
                        let _ = ack.send(SubscribeOutcome::Ok);
                    }
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(SessionSummary::placeholder(
                            "ws-sess",
                            "ws-sess",
                            chrono::Utc::now(),
                        ));
                    }
                    _ => {}
                }
            }
        });

        let state = test_state();
        let handle = ActorHandle::test_handle("ws-sess", "ws-sess", cmd_tx);
        state.supervisor.insert_test_handle(handle).await;

        let app = routes().with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ws test server");
        let addr = listener.local_addr().expect("server addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let url = format!("ws://{addr}/ws/sessions/ws-sess");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .expect("ws connect");

        let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ready payload within timeout")
            .expect("ws stream item")
            .expect("ws message");
        let text = match first {
            ClientMessage::Text(text) => text,
            other => panic!("expected text ready frame, got {other:?}"),
        };
        let payload: serde_json::Value =
            serde_json::from_str(&text).expect("ready payload is json");
        assert_eq!(payload["type"], "ready");
        assert_eq!(payload["sessionId"], "ws-sess");
        // LocalTrust grants operator scopes, so the stream is writable.
        assert_eq!(payload["readOnly"], false);
        assert_eq!(payload["replay"]["latestSeq"], 5);
        assert_eq!(payload["replay"]["windowStartSeq"], 1);

        let _ = ws.close(None).await;
        server.abort();
    }

    #[test]
    fn decode_client_message_close_returns_close() {
        let msg = Message::Close(None);
        assert!(matches!(
            decode_client_message(&operator_auth(), &msg),
            WsClientDecision::Close
        ));
    }

    #[test]
    fn decode_client_message_pong_is_ignored() {
        let msg = Message::Pong(b"ping".to_vec().into());
        assert!(matches!(
            decode_client_message(&operator_auth(), &msg),
            WsClientDecision::Ignore
        ));
    }

    #[test]
    fn decode_client_message_ping_frame_sends_pong() {
        let msg = Message::Ping(b"abc".to_vec().into());
        match decode_client_message(&operator_auth(), &msg) {
            WsClientDecision::SendPong(bytes) => assert_eq!(bytes, b"abc"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_client_message_binary_without_write_scope_is_read_only_error() {
        let msg = Message::Binary(b"hello".to_vec().into());
        match decode_client_message(&observer_auth(), &msg) {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_client_message_binary_with_write_scope_forwards_write_input() {
        let msg = Message::Binary(b"hello".to_vec().into());
        match decode_client_message(&operator_auth(), &msg) {
            WsClientDecision::Forward {
                cmd: SessionCommand::WriteInput(data),
                ..
            } => {
                assert_eq!(data, b"hello")
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_client_message_empty_binary_is_ignored() {
        let msg = Message::Binary(b"".to_vec().into());
        assert!(matches!(
            decode_client_message(&operator_auth(), &msg),
            WsClientDecision::Ignore
        ));
    }

    #[test]
    fn decode_text_client_message_invalid_json_sends_error() {
        match decode_text_client_message(&operator_auth(), "not-json") {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "INVALID_MESSAGE"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_ping_replies_pong() {
        assert!(matches!(
            decode_text_client_message(&operator_auth(), r#"{"type":"ping"}"#),
            WsClientDecision::ReplyPong
        ));
    }

    #[test]
    fn decode_text_client_message_auth_after_ready_is_ignored() {
        assert!(matches!(
            decode_text_client_message(&operator_auth(), r#"{"type":"auth","token":"secret"}"#),
            WsClientDecision::Ignore
        ));
    }

    #[test]
    fn decode_text_client_message_input_text_without_scope_is_read_only() {
        let json = r#"{"type":"input_text","data":"hello"}"#;
        match decode_text_client_message(&observer_auth(), json) {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_input_text_forwards_write_input() {
        let json = r#"{"type":"input_text","data":"hello"}"#;
        match decode_text_client_message(&operator_auth(), json) {
            WsClientDecision::Forward {
                cmd: SessionCommand::WriteInputAck { data, .. },
                ..
            } => {
                assert_eq!(data, b"hello")
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_submit_line_without_scope_is_read_only() {
        let json = r#"{"type":"submit_line","data":"hello"}"#;
        match decode_text_client_message(&observer_auth(), json) {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_submit_line_forwards_submit_line() {
        let json = r#"{"type":"submit_line","data":"hello"}"#;
        match decode_text_client_message(&operator_auth(), json) {
            WsClientDecision::Forward {
                cmd: SessionCommand::SubmitLineAck { text, .. },
                ..
            } => {
                assert_eq!(text, "hello")
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_blank_submit_line_is_ignored() {
        let json = r#"{"type":"submit_line","data":"   "}"#;
        assert!(matches!(
            decode_text_client_message(&operator_auth(), json),
            WsClientDecision::Ignore
        ));
    }

    #[test]
    fn decode_text_client_message_oversized_input_text_is_rejected() {
        let big = "x".repeat(MAX_WS_INPUT_BYTES + 1);
        let json = format!(r#"{{"type":"input_text","data":"{big}"}}"#);
        match decode_text_client_message(&operator_auth(), &json) {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "INPUT_TOO_LARGE"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_oversized_submit_line_is_rejected() {
        let big = "x".repeat(MAX_WS_INPUT_BYTES + 1);
        let json = format!(r#"{{"type":"submit_line","data":"{big}"}}"#);
        match decode_text_client_message(&operator_auth(), &json) {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "INPUT_TOO_LARGE"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn resolve_ws_auth_rejects_empty_token_in_token_mode() {
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("secret".to_string()),
            ..Config::default()
        };
        // An explicit empty auth-frame token must never authenticate, mirroring
        // the HTTP bearer-token empty guard.
        assert!(resolve_ws_auth(&config, Some("")).is_err());
        assert!(resolve_ws_auth(&config, None).is_err());
        assert!(resolve_ws_auth(&config, Some("secret")).is_ok());
    }

    #[test]
    fn decode_text_client_message_resize_without_scope_is_read_only() {
        let json = r#"{"type":"resize","cols":80,"rows":24}"#;
        match decode_text_client_message(&observer_auth(), json) {
            WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_resize_forwards_resize_command() {
        let json = r#"{"type":"resize","cols":80,"rows":24}"#;
        match decode_text_client_message(&operator_auth(), json) {
            WsClientDecision::Forward {
                cmd: SessionCommand::Resize { cols, rows },
                ..
            } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_resize_clamps_tiny_payloads() {
        let json = r#"{"type":"resize","cols":0,"rows":1}"#;
        match decode_text_client_message(&operator_auth(), json) {
            WsClientDecision::Forward {
                cmd: SessionCommand::Resize { cols, rows },
                ..
            } => {
                assert_eq!(cols, crate::types::TERMINAL_RESIZE_MIN_COLS);
                assert_eq!(rows, crate::types::TERMINAL_RESIZE_MIN_ROWS);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decode_text_client_message_resize_clamps_huge_payloads() {
        let json = r#"{"type":"resize","cols":65535,"rows":65535}"#;
        match decode_text_client_message(&operator_auth(), json) {
            WsClientDecision::Forward {
                cmd: SessionCommand::Resize { cols, rows },
                ..
            } => {
                assert_eq!(cols, crate::types::TERMINAL_RESIZE_MAX_COLS);
                assert_eq!(rows, crate::types::TERMINAL_RESIZE_MAX_ROWS);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
