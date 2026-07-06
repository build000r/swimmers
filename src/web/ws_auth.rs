use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures::StreamExt;
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::api::AppState;
use crate::auth::{
    validate_trusted_request_headers, AuthInfo, AuthScope, OBSERVER_SCOPES, OPERATOR_SCOPES,
};
use crate::config::{AuthMode, Config};
use crate::session::actor::ActorHandle;

use super::{
    handle_session_ws, handle_token_session_ws, json_error, send_ws_error, WsReceiver, WsSender,
};

const WS_AUTH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
pub(super) struct WsQuery {
    pub(super) token: Option<String>,
    pub(super) resume_from_seq: Option<u64>,
    pub(super) framed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WsOutputMode {
    Raw,
    Framed,
}

impl WsQuery {
    pub(super) fn output_mode(&self) -> WsOutputMode {
        WsOutputMode::from_framed_query(self.framed.as_deref())
    }
}

impl WsOutputMode {
    fn from_framed_query(value: Option<&str>) -> Self {
        if query_flag_enabled(value) {
            Self::Framed
        } else {
            Self::Raw
        }
    }

    pub(super) fn protocol_output(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Framed => "framed_v1",
        }
    }
}

pub(super) enum SessionWsRoutePlan {
    Trusted { handle: ActorHandle, auth: AuthInfo },
    Token,
}

impl SessionWsRoutePlan {
    pub(super) fn into_response(
        self,
        ws: WebSocketUpgrade,
        state: Arc<AppState>,
        session_id: String,
        resume_from_seq: Option<u64>,
        output_mode: WsOutputMode,
    ) -> Response {
        match self {
            Self::Trusted { handle, auth } => ws.on_upgrade(move |socket| {
                handle_session_ws(
                    socket,
                    state,
                    handle,
                    session_id,
                    auth,
                    resume_from_seq,
                    output_mode,
                )
            }),
            Self::Token => ws.on_upgrade(move |socket| {
                handle_token_session_ws(socket, state, session_id, resume_from_seq, output_mode)
            }),
        }
    }
}

#[allow(clippy::result_large_err)]
pub(super) async fn session_ws_route_plan(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    session_id: &str,
    query: &WsQuery,
) -> Result<SessionWsRoutePlan, Response> {
    match state.config.auth_mode {
        AuthMode::LocalTrust | AuthMode::TailnetTrust => {
            trusted_session_ws_route_plan(state, headers, session_id).await
        }
        AuthMode::Token => token_session_ws_route_plan(query),
    }
}

#[allow(clippy::result_large_err)]
async fn trusted_session_ws_route_plan(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    session_id: &str,
) -> Result<SessionWsRoutePlan, Response> {
    validate_trusted_session_ws_request(&state.config, headers)?;
    let auth = AuthInfo::new(OPERATOR_SCOPES.to_vec());
    auth.require_scope(AuthScope::SessionsRead)?;
    let handle = require_session_ws_handle(state, session_id).await?;
    Ok(SessionWsRoutePlan::Trusted { handle, auth })
}

#[allow(clippy::result_large_err)]
fn validate_trusted_session_ws_request(
    config: &Config,
    headers: &HeaderMap,
) -> Result<(), Response> {
    validate_trusted_request_headers(config, headers)
}

#[allow(clippy::result_large_err)]
fn token_session_ws_route_plan(query: &WsQuery) -> Result<SessionWsRoutePlan, Response> {
    reject_session_ws_query_token(query)?;
    Ok(SessionWsRoutePlan::Token)
}

#[allow(clippy::result_large_err)]
fn reject_session_ws_query_token(query: &WsQuery) -> Result<(), Response> {
    if query.token.is_none() {
        return Ok(());
    }

    Err(json_error(
        StatusCode::BAD_REQUEST,
        "WS_QUERY_TOKEN_UNSUPPORTED",
        "WebSocket token query parameters are not supported; send an auth message after opening the socket",
    ))
}

#[allow(clippy::result_large_err)]
async fn require_session_ws_handle(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<ActorHandle, Response> {
    state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or_else(|| {
            json_error(
                StatusCode::NOT_FOUND,
                "SESSION_NOT_FOUND",
                "session not found",
            )
        })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserWsAuthMessage {
    Auth { token: String },
}

#[derive(Debug)]
pub(super) enum WsAuthDecision {
    Authenticated(AuthInfo),
    Close,
    Reject {
        code: &'static str,
        message: &'static str,
    },
}

pub(super) enum WsAuthFirstMessage {
    Message(Message),
    Closed,
    Timeout,
}

pub(super) async fn authenticate_session_ws(
    config: &Config,
    sender: &mut WsSender,
    receiver: &mut WsReceiver,
) -> anyhow::Result<Option<AuthInfo>> {
    let first = next_ws_auth_message(receiver).await?;
    let decision = decode_ws_auth_first_message(config, first);
    execute_ws_auth_decision(sender, decision).await
}

pub(super) async fn next_ws_auth_message(
    receiver: &mut WsReceiver,
) -> Result<WsAuthFirstMessage, axum::Error> {
    match tokio::time::timeout(WS_AUTH_TIMEOUT, receiver.next()).await {
        Ok(Some(Ok(message))) => Ok(WsAuthFirstMessage::Message(message)),
        Ok(Some(Err(err))) => Err(err),
        Ok(None) => Ok(WsAuthFirstMessage::Closed),
        Err(_) => Ok(WsAuthFirstMessage::Timeout),
    }
}

pub(super) fn decode_ws_auth_first_message(
    config: &Config,
    first: WsAuthFirstMessage,
) -> WsAuthDecision {
    match first {
        WsAuthFirstMessage::Message(message) => decode_ws_auth_message(config, &message),
        WsAuthFirstMessage::Closed => WsAuthDecision::Close,
        WsAuthFirstMessage::Timeout => WsAuthDecision::Reject {
            code: "WS_AUTH_TIMEOUT",
            message: "token-mode WebSocket connections must authenticate before terminal traffic",
        },
    }
}

async fn execute_ws_auth_decision(
    sender: &mut WsSender,
    decision: WsAuthDecision,
) -> anyhow::Result<Option<AuthInfo>> {
    match decision {
        WsAuthDecision::Authenticated(auth) => Ok(Some(auth)),
        WsAuthDecision::Close => Ok(None),
        WsAuthDecision::Reject { code, message } => {
            send_ws_error(sender, code, message).await?;
            Ok(None)
        }
    }
}

pub(super) fn decode_ws_auth_message(config: &Config, message: &Message) -> WsAuthDecision {
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

pub(super) fn query_flag_enabled(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "framed" | "framed_v1"
    )
}

#[allow(clippy::result_large_err)]
pub(super) fn resolve_ws_auth(config: &Config, token: Option<&str>) -> Result<AuthInfo, Response> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, HeaderValue};
    use tokio::sync::RwLock;

    fn ws_headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_str(host).expect("valid host header"),
        );
        if let Some(origin) = origin {
            headers.insert(
                header::ORIGIN,
                HeaderValue::from_str(origin).expect("valid origin header"),
            );
        }
        headers
    }

    fn ws_query() -> WsQuery {
        WsQuery {
            token: None,
            resume_from_seq: None,
            framed: None,
        }
    }

    fn test_state_with_config(config: Config) -> Arc<AppState> {
        let config = Arc::new(config);
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

    fn local_trust_state() -> Arc<AppState> {
        test_state_with_config(Config::default())
    }

    fn response_status(result: Result<SessionWsRoutePlan, Response>) -> StatusCode {
        match result {
            Ok(_) => panic!("expected route plan rejection"),
            Err(response) => response.status(),
        }
    }

    #[tokio::test]
    async fn session_ws_local_trust_rejects_hostile_host_before_upgrade() {
        let state = local_trust_state();
        let headers = ws_headers("attacker.test:3210", None);

        let status =
            response_status(session_ws_route_plan(&state, &headers, "missing", &ws_query()).await);

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn session_ws_local_trust_rejects_hostile_origin_before_upgrade() {
        let state = local_trust_state();
        let headers = ws_headers("127.0.0.1:3210", Some("https://attacker.test"));

        let status =
            response_status(session_ws_route_plan(&state, &headers, "missing", &ws_query()).await);

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn session_ws_local_trust_allows_same_origin_loopback_before_session_lookup() {
        let state = local_trust_state();
        let headers = ws_headers("127.0.0.1:3210", Some("http://127.0.0.1:3210"));

        let status =
            response_status(session_ws_route_plan(&state, &headers, "missing", &ws_query()).await);

        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
