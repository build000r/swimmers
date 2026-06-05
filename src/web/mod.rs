use std::sync::atomic::{AtomicUsize, Ordering};
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
use serde::Serialize;
use tokio::sync::oneshot;

use crate::api::envelope::error_body_msg;
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
#[cfg(test)]
use crate::auth::{OBSERVER_SCOPES, OPERATOR_SCOPES};
#[cfg(test)]
use crate::config::{AuthMode, Config};
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};

mod assets;
mod ws_auth;
mod ws_events;
mod ws_messages;

use self::ws_auth::{authenticate_session_ws, session_ws_route_plan, WsOutputMode, WsQuery};
#[cfg(test)]
use self::ws_auth::{
    decode_ws_auth_first_message, decode_ws_auth_message, query_flag_enabled, resolve_ws_auth,
    WsAuthDecision, WsAuthFirstMessage,
};
use self::ws_events::{prepare_session_ws_start, run_session_ws_event_loop, send_session_ws_ready};
use self::ws_messages::{decode_client_message, WsClientDecision};

const PUBLISHED_VIEW_ROUTE: &str = "/selected";
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_BROWSER_WS_CONNECTIONS: usize = 64;

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
        .merge(assets::routes())
        .route("/ws/sessions/{session_id}", get(session_ws))
}

#[derive(Debug, Serialize)]
struct BootPayload {
    franken_term_available: bool,
    franken_term_js_url: &'static str,
    franken_term_wasm_url: &'static str,
    franken_term_font_url: &'static str,
    franken_term_asset_info: Option<assets::FrankenTermAssetInfo>,
    follow_published_selection: bool,
    focus_layout: bool,
}

async fn index() -> impl IntoResponse {
    render_index(false).await
}

async fn selected_index() -> impl IntoResponse {
    render_index(true).await
}

async fn render_index(focus_layout: bool) -> impl IntoResponse {
    let boot = BootPayload {
        franken_term_available: assets::resolve_frankentui_pkg_dir().is_some(),
        franken_term_js_url: assets::FRANKENTERM_JS_ROUTE,
        franken_term_wasm_url: assets::FRANKENTERM_WASM_ROUTE,
        franken_term_font_url: assets::FRANKENTERM_FONT_ROUTE,
        franken_term_asset_info: assets::franken_term_asset_info().await,
        follow_published_selection: focus_layout,
        focus_layout,
    };
    let boot_json = serde_json::to_string(&boot).unwrap_or_else(|_| "{}".to_string());
    let body_class = if focus_layout {
        "app-body published-focus"
    } else {
        "app-body"
    };
    let frontend_assets = assets::frontend_asset_tags().await;
    let stylesheet_tags = frontend_stylesheet_tags(&frontend_assets);
    let module_preload_tags = frontend_module_preload_tags(&frontend_assets);
    let module_script_tags = frontend_module_script_tags(&frontend_assets);
    let franken_term_font_route = assets::FRANKENTERM_FONT_ROUTE;
    let franken_term_available = boot.franken_term_available;

    let html = format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>swimmers</title>
    <link rel="preload" href="{franken_term_font_route}" as="font" type="font/woff2" crossorigin />
    {stylesheet_tags}
    {module_preload_tags}
  </head>
  <body class="{body_class}">
    <div class="shell">
      <div id="swimmers-react-root" data-swimmers-react-root="shell">
        <main
          class="terminal-stage"
          id="terminal-stage"
          tabindex="0"
          role="application"
          aria-label="swimmers rendered control surface"
          data-franken-term-available="{franken_term_available}"
          data-focus-layout="{focus_layout}"
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
      </div>
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
    {module_script_tags}
  </body>
</html>"#
    );

    ([(header::CACHE_CONTROL, "no-store")], Html(html))
}

fn frontend_stylesheet_tags(assets: &assets::FrontendAssetTags) -> String {
    assets
        .stylesheets
        .iter()
        .map(|href| {
            format!(
                r#"<link rel="stylesheet" href="{}" />"#,
                escape_html_attr(href)
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ")
}

fn frontend_module_preload_tags(assets: &assets::FrontendAssetTags) -> String {
    assets
        .module_preloads
        .iter()
        .map(|href| {
            format!(
                r#"<link rel="modulepreload" crossorigin href="{}" />"#,
                escape_html_attr(href)
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ")
}

fn frontend_module_script_tags(assets: &assets::FrontendAssetTags) -> String {
    assets
        .module_scripts
        .iter()
        .map(|src| {
            format!(
                r#"<script type="module" src="{}"></script>"#,
                escape_html_attr(src)
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ")
}

fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn session_ws(
    ws: WebSocketUpgrade,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let output_mode = query.output_mode();
    let resume_from_seq = query.resume_from_seq;

    let plan = match session_ws_route_plan(&state, &session_id, &query).await {
        Ok(plan) => plan,
        Err(response) => return response,
    };

    plan.into_response(ws, state, session_id, resume_from_seq, output_mode)
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
    let mut session = prepare_session_ws_start(
        &state,
        &handle,
        &session_id,
        &auth,
        resume_from_seq,
        output_mode,
    )
    .await?;

    if !send_session_ws_ready(&mut sender, &session).await? {
        return Ok(());
    }

    run_session_ws_event_loop(
        &handle,
        &mut sender,
        &mut receiver,
        &auth,
        output_mode,
        &session_id,
        &mut session,
    )
    .await?;

    let _ = handle
        .send(SessionCommand::Unsubscribe {
            client_id: session.client_id,
        })
        .await;
    Ok(())
}

async fn handle_client_message(
    handle: &ActorHandle,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    auth: &AuthInfo,
    message: Message,
) -> anyhow::Result<bool> {
    execute_client_decision(handle, sender, decode_client_message(auth, &message)).await
}

async fn execute_client_decision(
    handle: &ActorHandle,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    decision: WsClientDecision,
) -> anyhow::Result<bool> {
    match decision {
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
            send_client_rejection(sender, code, msg, client_message_id).await?;
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

async fn send_client_rejection(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    code: &'static str,
    message: String,
    client_message_id: Option<String>,
) -> anyhow::Result<()> {
    send_rejection_ack_if_needed(sender, client_message_id, &message).await?;
    send_ws_error(sender, code, &message).await
}

async fn send_rejection_ack_if_needed(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    client_message_id: Option<String>,
    message: &str,
) -> anyhow::Result<()> {
    match client_message_id {
        Some(client_message_id) => {
            send_input_ack(
                sender,
                Some(client_message_id),
                rejected_input_delivery(message),
            )
            .await
        }
        None => Ok(()),
    }
}

fn rejected_input_delivery(message: &str) -> InputDeliveryResult {
    InputDeliveryResult {
        delivered: false,
        method: "rejected",
        message: Some(message.to_string()),
    }
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

fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(error_body_msg(code, message))).into_response()
}

#[cfg(test)]
mod tests;
