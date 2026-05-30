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
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope, OBSERVER_SCOPES, OPERATOR_SCOPES};
use crate::config::{AuthMode, Config};
use crate::session::actor::{
    ActorHandle, InputDeliveryResult, OutputFrame, ReplayCursor, SessionCommand, SubscribeOutcome,
};
use crate::session::supervisor::LifecycleEvent;
use crate::types::{clamp_terminal_resize, opcodes, ControlEvent, ErrorResponse};

const APP_JS_ROUTE: &str = "/app.js";
const RENDERED_SURFACE_JS_ROUTE: &str = "/rendered_surface.js";
const INPUT_SUPPORT_JS_ROUTE: &str = "/input_support.js";
const APP_CSS_ROUTE: &str = "/app.css";
const FRANKENTERM_JS_ROUTE: &str = "/assets/frankenterm/FrankenTerm.js";
const FRANKENTERM_WASM_ROUTE: &str = "/assets/frankenterm/FrankenTerm_bg.wasm";
const FRANKENTERM_FONT_ROUTE: &str = "/assets/frankenterm/pragmasevka-nf-subset.woff2";
const TROGDOR_DRAGON_ASSET_ROUTE: &str = "/assets/dragon/{pose}/{frame}";
const PUBLISHED_VIEW_ROUTE: &str = "/selected";
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_WS_INPUT_BYTES: usize = 786_432;
const MAX_BROWSER_WS_CONNECTIONS: usize = 64;
const DEFAULT_FRANKENTUI_PKG_CANDIDATES: &[&str] = &[];

static NEXT_WS_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_WS_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

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

        <section class="surface-sheet hidden create-sheet-burninate" id="create-sheet" aria-labelledby="create-sheet-title">
          <div class="sheet-header create-sheet-header">
            <p class="sheet-eyebrow">Repository Atlas</p>
            <h2 id="create-sheet-title">Create Session</h2>
          </div>
          <div class="sheet-copy" id="dirs-summary">Browse directories before creating a session.</div>
          <div class="sheet-grid create-sheet-grid">
            <section class="sheet-panel create-sheet-browser-panel">
              <div class="sheet-panel-header">
                <h3>Directory Browser</h3>
                <label class="toggle-row">
                  <input id="dirs-managed-only" type="checkbox" />
                  <span>Managed only</span>
                </label>
              </div>
              <div class="create-dir-controls">
                <input id="dirs-search" type="search" placeholder="Search loaded directories" autocomplete="off" />
                <button class="ghost-button" id="create-batch-visible" type="button">Batch visible</button>
                <button class="ghost-button" id="dirs-spawn-here" type="button">Spawn here</button>
              </div>
              <div class="browser-list create-dir-list" id="dirs-list" role="list" aria-label="Directory entries"></div>
              <div class="sheet-toolbar create-sheet-toolbar">
                <input id="dirs-path" type="text" placeholder="/absolute/path" autocomplete="off" />
                <button class="ghost-button" id="dirs-load-button" type="button">Load</button>
                <button class="ghost-button" id="dirs-up-button" type="button">Up</button>
              </div>
              <div class="batch-bar hidden" id="create-batch-bar" aria-live="polite">
                <div class="batch-bar-copy">
                  <span class="batch-count" id="create-batch-count">0 selected</span>
                  <span class="batch-tool" id="create-batch-tool">tool: grok</span>
                  <span class="batch-preview" id="create-batch-preview">request: (none)</span>
                </div>
                <div class="batch-actions">
                  <button class="ghost-button batch-clear" id="create-batch-clear" type="button">Clear</button>
                  <button class="batch-submit" id="create-batch-submit" type="submit" form="create-form">Batch send</button>
                </div>
              </div>
            </section>

            <form class="sheet-form sheet-panel create-sheet-form" id="create-form">
              <label class="field">
                <span>Working Directory</span>
                <input id="create-cwd" type="text" placeholder="/absolute/path" autocomplete="off" />
              </label>
              <label class="field">
                <span>Tool</span>
                <select id="create-tool">
                  <option value="grok">Grok</option>
                  <option value="codex">Codex</option>
                  <option value="claude">Claude</option>
                </select>
              </label>
              <label class="field">
                <span>Launch Target</span>
                <select id="create-launch-target"></select>
              </label>
              <label class="field">
                <span>Initial Request</span>
                <textarea id="create-request" rows="5" placeholder="Optional boot prompt for the new session"></textarea>
              </label>
              <div class="sheet-actions">
                <button class="ghost-button" id="create-close-button" type="button">Cancel</button>
                <button id="create-button" type="submit">Create Session</button>
              </div>
            </form>
          </div>
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

async fn app_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("app.js"),
    )
}

async fn rendered_surface_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("rendered_surface.js"),
    )
}

async fn input_support_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("input_support.js"),
    )
}

async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("app.css"),
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
    let auth = match resolve_ws_auth(&state.config, query.token.as_deref()) {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    if let Err(response) = auth.require_scope(AuthScope::SessionsRead) {
        return response;
    }

    let resume_from_seq = query.resume_from_seq;
    let output_mode = if query_flag_enabled(query.framed.as_deref()) {
        WsOutputMode::Framed
    } else {
        WsOutputMode::Raw
    };

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

async fn session_ws_inner(
    socket: WebSocket,
    state: Arc<AppState>,
    handle: ActorHandle,
    session_id: String,
    auth: AuthInfo,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) -> anyhow::Result<()> {
    let Some(_ws_guard) = ActiveWsGuard::try_acquire() else {
        let (mut sender, _) = socket.split();
        let notice = serde_json::json!({
            "type": "overloaded",
            "code": "SERVER_OVERLOADED",
            "message": "server has too many active browser terminal attachments",
            "retryAfterMs": 5000,
        });
        sender
            .send(Message::Text(notice.to_string().into()))
            .await?;
        return Ok(());
    };
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

    let (mut sender, mut receiver) = socket.split();

    let ready_payload = serde_json::json!({
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
    });
    sender
        .send(Message::Text(ready_payload.to_string().into()))
        .await?;

    match subscribe_outcome {
        SubscribeOutcome::Ok => {}
        SubscribeOutcome::Rejected { reason } => {
            let notice = serde_json::json!({
                "type": "overloaded",
                "code": "SESSION_OVERLOADED",
                "message": reason,
                "retryAfterMs": 4000,
            });
            sender
                .send(Message::Text(notice.to_string().into()))
                .await?;
            return Ok(());
        }
        SubscribeOutcome::ReplayTruncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        } => {
            let notice = serde_json::json!({
                "type": "replay_truncated",
                "requestedResumeFromSeq": requested_resume_from_seq,
                "windowStartSeq": replay_window_start_seq,
                "latestSeq": latest_seq,
            });
            sender
                .send(Message::Text(notice.to_string().into()))
                .await?;
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
                WsClientDecision::SendError {
                    code: "INPUT_TOO_LARGE",
                    message: format!(
                        "terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit"
                    ),
                    client_message_id,
                }
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
                WsClientDecision::SendError {
                    code: "INPUT_TOO_LARGE",
                    message: format!(
                        "terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit"
                    ),
                    client_message_id,
                }
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

async fn send_ws_error(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    code: &str,
    message: &str,
) -> anyhow::Result<()> {
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
    serde_json::json!({
        "type": "control_event",
        "event": event.event,
        "sessionId": event.session_id,
        "payload": event.payload,
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
            // empty-token guard and keeps `?token=` from ever matching.
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
    (
        status,
        Json(ErrorResponse {
            code: code.to_string(),
            message: Some(message.to_string()),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn websocket_auth_accepts_observer_and_operator_tokens() {
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("operator".into()),
            observer_token: Some("observer".into()),
            ..Config::default()
        };

        let operator = resolve_ws_auth(&config, Some("operator")).expect("operator auth");
        assert!(operator.has_scope(AuthScope::StreamWrite));

        let observer = resolve_ws_auth(&config, Some("observer")).expect("observer auth");
        assert!(observer.has_scope(AuthScope::SessionsRead));
        assert!(!observer.has_scope(AuthScope::StreamWrite));
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
        assert!(js.contains("TROGDOR_DRAGON_ASSET_BASE = \"/assets/dragon\""));
        assert!(js.contains("function trogdorDragonPose(groups, summary)"));
        assert!(js.contains("trogdor-dragon-sprite"));
        assert!(js.contains("agent-burn-flame"));
        assert!(js.contains("const dragonTarget = dragonPose || TROGDOR_DRAGON_TARGET"));

        let css = include_str!("app.css");
        assert!(css.contains("@keyframes dragon-walk-around"));
        assert!(css.contains("@keyframes dragon-sprite-fire"));
        assert!(css.contains(".agent-burn-flame"));
        assert!(css.contains("@media (prefers-reduced-motion: reduce)"));
    }

    #[test]
    fn app_js_includes_overlay_filter_chip_logic() {
        let js = include_str!("app.js");
        assert!(js.contains("response?.overlay_label"));
        assert!(js.contains("managedButton.dataset.filter = \"managed\""));
        assert!(js.contains("allButton.textContent = \"all folders\""));
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
        assert!(js.contains("function operatorPressureSummary"));
        assert!(js.contains("Tool calls"));
        assert!(js.contains("/agent-context"));
        assert!(js.contains("/pane-tail"));
        assert!(js.contains("/transcript"));
        assert!(js.contains("function renderTurnsPanel"));
        assert!(js.contains("function flushPendingTerminalBytes"));
        assert!(js.contains("Post-turn JSONL"));
        assert!(js.contains("/mermaid-artifact"));
        assert!(js.contains("/git-diff"));
        assert!(js.contains("function renderDiffHtml"));
        assert!(js.contains("function syncTerminalWorkbench()"));
    }

    #[test]
    fn app_js_dedupes_surface_actions_and_stable_resizes() {
        let js = include_str!("app.js");
        assert!(js.contains("function stopSurfaceEvent(event)"));
        assert!(js.contains("event.stopImmediatePropagation"));
        assert!(js.contains(
            "const dimensionsChanged = cols !== state.currentCols || rows !== state.currentRows"
        ));
        assert!(js.contains("if (!force && !dimensionsChanged)"));
        assert!(js.contains("measureAndResizeSurface(true, false)"));
    }

    #[test]
    fn app_js_hides_hud_when_live_terminal_is_focused() {
        let js = include_str!("app.js");
        assert!(js.contains("function syncTerminalPresentation()"));
        assert!(js.contains("terminal-focus-mode"));
        assert!(js.contains("el.hudCanvas.classList.toggle(\"hidden\", terminalFocusMode)"));
        assert!(js.contains("el.hudCanvas.style.display = terminalFocusMode ? \"none\" : \"\""));
        assert!(
            js.contains("el.hudCanvas.style.visibility = terminalFocusMode ? \"hidden\" : \"\"")
        );
        assert!(js.contains("el.terminalCanvas.classList.toggle(\"hidden\", false)"));
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
        // An explicit empty `?token=` must never authenticate, mirroring the
        // HTTP bearer-token empty guard.
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
