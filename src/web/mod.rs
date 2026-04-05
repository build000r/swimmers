use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
use tokio::sync::{mpsc, oneshot};

use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope, OBSERVER_SCOPES, OPERATOR_SCOPES};
use crate::config::{AuthMode, Config};
use crate::session::actor::{
    ActorHandle, OutputFrame, ReplayCursor, SessionCommand, SubscribeOutcome,
};
use crate::types::ErrorResponse;

const APP_JS_ROUTE: &str = "/app.js";
const RENDERED_SURFACE_JS_ROUTE: &str = "/rendered_surface.js";
const INPUT_SUPPORT_JS_ROUTE: &str = "/input_support.js";
const APP_CSS_ROUTE: &str = "/app.css";
const FRANKENTERM_JS_ROUTE: &str = "/assets/frankenterm/FrankenTerm.js";
const FRANKENTERM_WASM_ROUTE: &str = "/assets/frankenterm/FrankenTerm_bg.wasm";
const FRANKENTERM_FONT_ROUTE: &str = "/assets/frankenterm/pragmasevka-nf-subset.woff2";
const PUBLISHED_VIEW_ROUTE: &str = "/selected";
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_FRANKENTUI_PKG_CANDIDATES: &[&str] = &[
    "/Users/b/projects/frankentui/pkg",
    "/Users/b/repos/opensource/frankentui/pkg",
    "/Users/b/repos/frankentui/pkg",
];

static NEXT_WS_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

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
        .route("/ws/sessions/{session_id}", get(session_ws))
}

#[derive(Debug, Serialize)]
struct BootPayload {
    franken_term_available: bool,
    franken_term_js_url: &'static str,
    franken_term_wasm_url: &'static str,
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
        franken_term_available: resolve_frankentui_pkg_dir().is_some(),
        franken_term_js_url: FRANKENTERM_JS_ROUTE,
        franken_term_wasm_url: FRANKENTERM_WASM_ROUTE,
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
        <pre class="terminal-fallback hidden" id="terminal-fallback"></pre>
        <div class="loading-overlay visible" id="loading-overlay" aria-hidden="true">
          <div class="loading-label" id="loading-label">Loading FrankenTerm…</div>
          <div class="loading-bar"><div class="loading-bar-fill"></div></div>
        </div>
      </main>

      <div class="modal-root" id="modal-root" aria-hidden="true">
        <div class="modal-backdrop" id="modal-backdrop"></div>

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
              <span>Input</span>
              <textarea id="send-input" rows="5" placeholder="Type a command or paste text. Send appends a newline."></textarea>
            </label>
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

        <section class="surface-sheet hidden" id="create-sheet" aria-labelledby="create-sheet-title">
          <div class="sheet-header">
            <p class="sheet-eyebrow">Repository</p>
            <h2 id="create-sheet-title">Create Session</h2>
          </div>
          <div class="sheet-copy" id="dirs-summary">Browse directories before creating a session.</div>
          <div class="sheet-grid">
            <section class="sheet-panel">
              <div class="sheet-panel-header">
                <h3>Directory Browser</h3>
                <label class="toggle-row">
                  <input id="dirs-managed-only" type="checkbox" />
                  <span>Managed only</span>
                </label>
              </div>
              <div class="sheet-toolbar">
                <input id="dirs-path" type="text" placeholder="/absolute/path" autocomplete="off" />
                <button class="ghost-button" id="dirs-load-button" type="button">Load</button>
                <button class="ghost-button" id="dirs-up-button" type="button">Up</button>
              </div>
              <div class="browser-list" id="dirs-list" role="listbox" aria-label="Directory entries"></div>
            </section>

            <form class="sheet-form sheet-panel" id="create-form">
              <label class="field">
                <span>Working Directory</span>
                <input id="create-cwd" type="text" placeholder="/absolute/path" autocomplete="off" />
              </label>
              <label class="field">
                <span>Tool</span>
                <select id="create-tool">
                  <option value="codex">Codex</option>
                  <option value="claude">Claude</option>
                </select>
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

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: Option<String>,
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

    let Some(handle) = state.supervisor.get_session(&session_id).await else {
        return json_error(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            "session not found",
        );
    };

    ws.on_upgrade(move |socket| handle_session_ws(socket, state, handle, session_id, auth))
}

async fn handle_session_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    handle: ActorHandle,
    session_id: String,
    auth: AuthInfo,
) {
    if let Err(err) = session_ws_inner(socket, state, handle, session_id.clone(), auth).await {
        tracing::warn!(session_id, "browser attach closed with error: {err}");
    }
}

async fn session_ws_inner(
    socket: WebSocket,
    state: Arc<AppState>,
    handle: ActorHandle,
    session_id: String,
    auth: AuthInfo,
) -> anyhow::Result<()> {
    let client_id = NEXT_WS_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let replay_cursor = request_replay_cursor(&handle).await?;
    let resume_from_seq = replay_cursor.replay_window_start_seq.saturating_sub(1);
    let (mut output_rx, subscribe_outcome) =
        subscribe_to_output(&state, &handle, client_id, Some(resume_from_seq)).await?;
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
        },
        "summary": summary,
    });
    sender
        .send(Message::Text(ready_payload.to_string().into()))
        .await?;

    if let SubscribeOutcome::ReplayTruncated {
        requested_resume_from_seq,
        replay_window_start_seq,
        latest_seq,
    } = subscribe_outcome
    {
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

    while let Some(result) = tokio::select! {
        maybe_message = receiver.next() => maybe_message.map(Either::Incoming),
        maybe_frame = output_rx.recv() => maybe_frame.map(Either::Frame),
    } {
        match result {
            Either::Incoming(Ok(message)) => {
                if !handle_client_message(&handle, &mut sender, &auth, message).await? {
                    break;
                }
            }
            Either::Incoming(Err(err)) => return Err(err.into()),
            Either::Frame(OutputFrame { data, .. }) => {
                sender.send(Message::Binary(data.into())).await?;
            }
        }
    }

    let _ = handle.send(SessionCommand::Unsubscribe { client_id }).await;
    Ok(())
}

enum Either {
    Incoming(Result<Message, axum::Error>),
    Frame(OutputFrame),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserClientMessage {
    InputText { data: String },
    Resize { cols: u16, rows: u16 },
    Ping,
}

/// Pure routing decision derived from an incoming WebSocket message. No I/O.
#[derive(Debug)]
enum WsClientDecision {
    Close,
    Ignore,
    SendPong(Vec<u8>),
    ReplyPong,
    SendError { code: &'static str, message: String },
    Forward(SessionCommand),
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
                };
            }
            if bytes.is_empty() {
                WsClientDecision::Ignore
            } else {
                WsClientDecision::Forward(SessionCommand::WriteInput(bytes.to_vec()))
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
            }
        }
    };
    match parsed {
        BrowserClientMessage::Ping => WsClientDecision::ReplyPong,
        BrowserClientMessage::InputText { data } => {
            if !auth.has_scope(AuthScope::StreamWrite) {
                return WsClientDecision::SendError {
                    code: "READ_ONLY",
                    message: "observer connections cannot send terminal input".to_string(),
                };
            }
            if data.is_empty() {
                WsClientDecision::Ignore
            } else {
                WsClientDecision::Forward(SessionCommand::WriteInput(data.into_bytes()))
            }
        }
        BrowserClientMessage::Resize { cols, rows } => {
            if !auth.has_scope(AuthScope::StreamWrite) {
                return WsClientDecision::SendError {
                    code: "READ_ONLY",
                    message: "observer connections cannot resize terminal sessions".to_string(),
                };
            }
            WsClientDecision::Forward(SessionCommand::Resize { cols, rows })
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
        WsClientDecision::SendError { code, message: msg } => {
            send_ws_error(sender, code, &msg).await?;
        }
        WsClientDecision::Forward(cmd) => {
            handle
                .send(cmd)
                .await
                .map_err(|err| anyhow::anyhow!("failed to forward command: {err}"))?;
        }
    }

    Ok(true)
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

fn resolve_ws_auth(config: &Config, token: Option<&str>) -> Result<AuthInfo, Response> {
    match config.auth_mode {
        AuthMode::LocalTrust => Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        AuthMode::Token => {
            let Some(token) = token else {
                return Err(json_error(
                    StatusCode::UNAUTHORIZED,
                    "NOT_AUTHENTICATED",
                    "Missing or invalid authentication token",
                ));
            };

            if config.auth_token.as_deref() == Some(token) {
                return Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec()));
            }

            if config.observer_token.as_deref() == Some(token) {
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

    #[tokio::test]
    async fn index_shell_includes_new_web_parity_sheets() {
        let html = html_string(render_index(false).await).await;
        assert!(html.contains("thought-config-sheet"));
        assert!(html.contains("native-sheet"));
        assert!(html.contains("mermaid-sheet"));
        assert!(html.contains("dirs-list"));
        assert!(html.contains("window.__SWIMMERS_BOOT__"));
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
            WsClientDecision::Forward(SessionCommand::WriteInput(data)) => {
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
            WsClientDecision::Forward(SessionCommand::WriteInput(data)) => {
                assert_eq!(data, b"hello")
            }
            other => panic!("unexpected: {other:?}"),
        }
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
            WsClientDecision::Forward(SessionCommand::Resize { cols, rows }) => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
