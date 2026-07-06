use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::api::envelope::error_body;
use crate::api::service::BATCH_CREATE_MAX_DIRS;
use crate::api::{remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::operator_pressure::session_ready_for_operator_group_input;
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};
use crate::session::overlay::default_overlay;
use crate::types::{
    ErrorResponse, LaunchTargetSummary, SessionGroupInputRequest, SessionGroupInputResponse,
    SessionGroupInputResult, SessionState, SessionSummary, MAX_SESSION_INPUT_BYTES,
};

const INPUT_DELIVERY_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_GROUP_INPUT_SESSIONS: usize = BATCH_CREATE_MAX_DIRS;

fn group_input_error_result(
    session_id: String,
    code: impl Into<String>,
    message: Option<String>,
) -> SessionGroupInputResult {
    SessionGroupInputResult {
        session_id,
        ok: false,
        partial: false,
        error: Some(error_body(code, message)),
    }
}

struct ValidatedGroupInputRequest {
    session_ids: Vec<String>,
    text: String,
    input: Vec<u8>,
}

fn validate_group_input_request(
    body: SessionGroupInputRequest,
) -> Result<ValidatedGroupInputRequest, ErrorResponse> {
    if body.session_ids.is_empty() {
        return Err(error_body(
            "VALIDATION_FAILED",
            Some("session_ids must not be empty".to_string()),
        ));
    }
    if body.session_ids.len() > MAX_GROUP_INPUT_SESSIONS {
        return Err(error_body(
            "VALIDATION_FAILED",
            Some(format!(
                "session_ids must include at most {MAX_GROUP_INPUT_SESSIONS} entries"
            )),
        ));
    }

    let text = body.text;
    if text.trim().is_empty() {
        return Err(error_body(
            "VALIDATION_FAILED",
            Some("text must not be empty".to_string()),
        ));
    }
    if group_input_payload_len(&text) > MAX_SESSION_INPUT_BYTES {
        return Err(error_body(
            "INPUT_TOO_LARGE",
            Some(format!(
                "terminal input exceeds {MAX_SESSION_INPUT_BYTES} byte limit"
            )),
        ));
    }

    let session_ids = unique_group_input_session_ids(body.session_ids);
    if session_ids.len() < 2 {
        return Err(error_body(
            "VALIDATION_FAILED",
            Some("session_ids must include at least two unique sessions".to_string()),
        ));
    }

    Ok(ValidatedGroupInputRequest {
        session_ids,
        input: group_input_bytes(&text),
        text,
    })
}

fn unique_group_input_session_ids(session_ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    session_ids
        .into_iter()
        .filter(|session_id| seen.insert(session_id.clone()))
        .collect()
}

fn session_ready_for_group_input(summary: &SessionSummary) -> bool {
    session_ready_for_operator_group_input(summary)
}

#[derive(Debug)]
struct GroupInputSessionError {
    code: &'static str,
    message: Option<String>,
}

impl GroupInputSessionError {
    fn new(code: &'static str, message: Option<String>) -> Self {
        Self { code, message }
    }

    fn not_found() -> Self {
        Self::new("SESSION_NOT_FOUND", None)
    }

    fn into_result(self, session_id: String) -> SessionGroupInputResult {
        group_input_error_result(session_id, self.code, self.message)
    }
}

fn group_input_preflight_error(summary: Option<&SessionSummary>) -> Option<GroupInputSessionError> {
    let Some(summary) = summary else {
        return Some(GroupInputSessionError::not_found());
    };

    if summary.state == SessionState::Exited {
        return Some(GroupInputSessionError::new(
            "SESSION_EXITED",
            Some("session has already exited".to_string()),
        ));
    }

    (!session_ready_for_group_input(summary)).then(|| {
        GroupInputSessionError::new(
            "SESSION_NOT_READY",
            Some("session is not waiting for input".to_string()),
        )
    })
}

fn group_input_batch_scope_error(summaries: &[SessionSummary]) -> Option<(&'static str, String)> {
    let batch_ids = summaries
        .iter()
        .map(|summary| summary.batch.as_ref().map(|batch| batch.id.as_str()));
    GroupInputBatchScope::from_batch_ids(batch_ids).error()
}

async fn group_input_summary_map(state: &Arc<AppState>) -> HashMap<String, SessionSummary> {
    state
        .supervisor
        .list_sessions()
        .await
        .into_iter()
        .map(|summary| (summary.session_id.clone(), summary))
        .collect()
}

fn group_input_batch_scope_error_for_targets(
    session_ids: &[String],
    summaries: &HashMap<String, SessionSummary>,
) -> Option<(&'static str, String)> {
    let found_summaries = session_ids
        .iter()
        .filter_map(|session_id| summaries.get(session_id).cloned())
        .collect::<Vec<_>>();
    group_input_batch_scope_error(&found_summaries)
}

fn group_input_batch_error_response(
    session_ids: Vec<String>,
    summaries: &HashMap<String, SessionSummary>,
    code: &'static str,
    message: String,
) -> SessionGroupInputResponse {
    let results = session_ids
        .into_iter()
        .map(|session_id| group_input_batch_error_result(session_id, summaries, code, &message))
        .collect();
    SessionGroupInputResponse::from_results(results)
}

fn group_input_batch_error_result(
    session_id: String,
    summaries: &HashMap<String, SessionSummary>,
    code: &'static str,
    message: &str,
) -> SessionGroupInputResult {
    if summaries.contains_key(&session_id) {
        group_input_error_result(session_id, code, Some(message.to_string()))
    } else {
        group_input_error_result(session_id, "SESSION_NOT_FOUND", None)
    }
}

fn group_input_all_error_response(
    session_ids: Vec<String>,
    code: impl Into<String>,
    message: String,
) -> SessionGroupInputResponse {
    let code = code.into();
    let results = session_ids
        .into_iter()
        .map(|session_id| group_input_error_result(session_id, code.clone(), Some(message.clone())))
        .collect();
    SessionGroupInputResponse::from_results(results)
}

#[derive(Default)]
struct GroupInputBatchScope<'a> {
    has_unbatched: bool,
    first_batch_id: Option<&'a str>,
    has_batch_mismatch: bool,
}

impl<'a> GroupInputBatchScope<'a> {
    fn from_batch_ids(batch_ids: impl IntoIterator<Item = Option<&'a str>>) -> Self {
        batch_ids
            .into_iter()
            .fold(Self::default(), Self::with_batch_id)
    }

    fn with_batch_id(mut self, batch_id: Option<&'a str>) -> Self {
        self.has_unbatched |= batch_id.is_none();
        self.has_batch_mismatch |= self
            .first_batch_id
            .zip(batch_id)
            .is_some_and(|(first, current)| first != current);
        self.first_batch_id = self.first_batch_id.or(batch_id);
        self
    }

    fn error(self) -> Option<(&'static str, String)> {
        [
            self.has_unbatched
                .then_some(("SESSION_NOT_IN_BATCH", "session is not part of a batch")),
            self.has_batch_mismatch.then_some((
                "SESSION_BATCH_MISMATCH",
                "sessions are not in the same batch",
            )),
        ]
        .into_iter()
        .flatten()
        .next()
        .map(|(code, message)| (code, message.to_string()))
    }
}

fn group_input_bytes(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.extend_from_slice(b"\r\r");
    bytes
}

fn group_input_payload_len(text: &str) -> usize {
    text.len().saturating_add(2)
}

async fn send_group_input_to_ready_session(
    state: &Arc<AppState>,
    session_id: &str,
    input: &[u8],
) -> Result<bool, GroupInputSessionError> {
    let handle = state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or_else(GroupInputSessionError::not_found)?;
    deliver_group_input_to_actor(session_id, &handle, input).await
}

async fn deliver_group_input_to_actor(
    session_id: &str,
    handle: &ActorHandle,
    input: &[u8],
) -> Result<bool, GroupInputSessionError> {
    let (ack_tx, ack_rx) = oneshot::channel();
    if let Err(err) = handle
        .send(SessionCommand::WriteInputAck {
            data: input.to_vec(),
            ack: ack_tx,
        })
        .await
    {
        tracing::error!("[session {session_id}] send_group_input failed: {err}");
        return Err(GroupInputSessionError::new(
            "SESSION_NOT_FOUND",
            Some(err.to_string()),
        ));
    }

    wait_for_group_input_delivery_ack(ack_rx, INPUT_DELIVERY_ACK_TIMEOUT).await
}

async fn wait_for_group_input_delivery_ack(
    ack_rx: oneshot::Receiver<InputDeliveryResult>,
    timeout: Duration,
) -> Result<bool, GroupInputSessionError> {
    match tokio::time::timeout(timeout, ack_rx).await {
        Ok(Ok(delivery)) => classify_group_input_delivery_ack(delivery),
        Ok(Err(_)) => Err(GroupInputSessionError::new(
            "INPUT_DELIVERY_UNKNOWN",
            Some("session actor dropped input delivery ack".to_string()),
        )),
        Err(_) => Err(GroupInputSessionError::new(
            "INPUT_DELIVERY_TIMEOUT",
            Some("timed out waiting for input delivery confirmation".to_string()),
        )),
    }
}

/// Returns Ok(partial) where `partial` mirrors the single-input path: the
/// submit landed (some-vs-none contract keeps it Ok) but may be incomplete
/// (swimmers-bjsu).
fn classify_group_input_delivery_ack(
    delivery: InputDeliveryResult,
) -> Result<bool, GroupInputSessionError> {
    if delivery.delivered {
        Ok(delivery.is_partial())
    } else {
        Err(GroupInputSessionError::new(
            "INPUT_DELIVERY_FAILED",
            delivery.message,
        ))
    }
}

async fn send_group_input_to_session(
    state: &Arc<AppState>,
    session_id: String,
    summary: Option<SessionSummary>,
    input: &[u8],
) -> SessionGroupInputResult {
    if let Some(error) = group_input_preflight_error(summary.as_ref()) {
        return error.into_result(session_id);
    }

    let partial = match send_group_input_to_ready_session(state, &session_id, input).await {
        Ok(partial) => partial,
        Err(error) => return error.into_result(session_id),
    };

    SessionGroupInputResult {
        session_id,
        ok: true,
        partial,
        error: None,
    }
}

async fn send_group_input_to_targets(
    state: &Arc<AppState>,
    session_ids: Vec<String>,
    summaries: &HashMap<String, SessionSummary>,
    input: &[u8],
) -> SessionGroupInputResponse {
    let results = futures::future::join_all(session_ids.into_iter().map(|session_id| {
        let summary = summaries.get(&session_id).cloned();
        send_group_input_to_session(state, session_id, summary, input)
    }))
    .await;
    SessionGroupInputResponse::from_results(results)
}

#[derive(Debug)]
struct RemoteGroupInputTarget {
    target: LaunchTargetSummary,
    remote_session_ids: Vec<String>,
}

#[derive(Debug)]
struct RemoteGroupInputScopeError {
    code: String,
    message: String,
}

impl RemoteGroupInputScopeError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn from_remote_error(error: remote_sessions::RemoteSessionError) -> Self {
        Self::new(error.code(), error.message().to_string())
    }

    fn into_error_response(self) -> ErrorResponse {
        error_body(self.code, Some(self.message))
    }
}

fn remote_group_input_target(
    session_ids: &[String],
) -> Result<Option<RemoteGroupInputTarget>, RemoteGroupInputScopeError> {
    let targets = default_overlay()
        .map(|overlay| overlay.all_launch_targets())
        .unwrap_or_default();
    remote_group_input_target_for_targets(session_ids, &targets)
}

fn remote_group_input_target_for_targets(
    session_ids: &[String],
    targets: &[LaunchTargetSummary],
) -> Result<Option<RemoteGroupInputTarget>, RemoteGroupInputScopeError> {
    let mut remote_target: Option<LaunchTargetSummary> = None;
    let mut remote_session_ids = Vec::new();
    let mut has_local = false;

    for session_id in session_ids {
        match remote_sessions::denamespace_for_configured_targets(session_id, targets)
            .map_err(RemoteGroupInputScopeError::from_remote_error)?
        {
            Some((target, remote_session_id)) => {
                if has_local {
                    return Err(RemoteGroupInputScopeError::new(
                        "REMOTE_GROUP_INPUT_MIXED_SCOPE",
                        "group input cannot mix local and remote sessions",
                    ));
                }
                if remote_target
                    .as_ref()
                    .is_some_and(|existing| existing.id != target.id)
                {
                    return Err(RemoteGroupInputScopeError::new(
                        "REMOTE_GROUP_INPUT_MIXED_TARGETS",
                        "remote group input requires sessions from one launch target",
                    ));
                }
                remote_target.get_or_insert(target);
                remote_session_ids.push(remote_session_id.to_string());
            }
            None => {
                if remote_target.is_some() {
                    return Err(RemoteGroupInputScopeError::new(
                        "REMOTE_GROUP_INPUT_MIXED_SCOPE",
                        "group input cannot mix local and remote sessions",
                    ));
                }
                has_local = true;
            }
        }
    }

    Ok(remote_target.map(|target| RemoteGroupInputTarget {
        target,
        remote_session_ids,
    }))
}

async fn send_remote_group_input_to_target(
    session_ids: Vec<String>,
    target: RemoteGroupInputTarget,
    text: String,
) -> SessionGroupInputResponse {
    match remote_sessions::send_remote_group_input(&target.target, target.remote_session_ids, text)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            group_input_all_error_response(session_ids, error.code(), error.message().to_string())
        }
    }
}

pub async fn send_group_input_service(
    state: Arc<AppState>,
    body: SessionGroupInputRequest,
) -> Result<SessionGroupInputResponse, ErrorResponse> {
    let ValidatedGroupInputRequest {
        session_ids,
        text,
        input,
    } = validate_group_input_request(body)?;

    match remote_group_input_target(&session_ids) {
        Ok(Some(target)) => {
            return Ok(send_remote_group_input_to_target(session_ids, target, text).await);
        }
        Ok(None) => {}
        Err(error) => return Err(error.into_error_response()),
    }

    let summaries = group_input_summary_map(&state).await;

    if let Some((code, message)) =
        group_input_batch_scope_error_for_targets(&session_ids, &summaries)
    {
        return Ok(group_input_batch_error_response(
            session_ids,
            &summaries,
            code,
            message,
        ));
    }

    Ok(send_group_input_to_targets(&state, session_ids, &summaries, &input).await)
}

pub(super) async fn send_group_input(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionGroupInputRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match send_group_input_service(state, body).await {
        Ok(response) => {
            let status = if response.skipped == 0 {
                StatusCode::OK
            } else {
                StatusCode::MULTI_STATUS
            };
            (status, Json(response)).into_response()
        }
        Err(error) => (group_input_error_status(&error), Json(error)).into_response(),
    }
}

fn group_input_error_status(error: &ErrorResponse) -> StatusCode {
    if error.code == "INPUT_TOO_LARGE" {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_target(id: &str) -> LaunchTargetSummary {
        LaunchTargetSummary {
            id: id.to_string(),
            label: id.to_string(),
            kind: "swimmers_api".to_string(),
            base_url: Some("http://127.0.0.1:3210".to_string()),
            auth_token_env: None,
            ssh_alias: None,
            remote_attach_command_template: None,
            bootstrap_hint: None,
            path_mappings: Vec::new(),
        }
    }

    #[test]
    fn group_input_bytes_appends_double_enter_for_agent_delivery() {
        assert_eq!(group_input_bytes("ship it"), b"ship it\r\r");
    }

    #[test]
    fn group_input_batch_scope_accepts_single_batch() {
        let scope = GroupInputBatchScope::from_batch_ids([Some("batch-a"), Some("batch-a")]);

        assert_eq!(scope.error(), None);
    }

    #[test]
    fn group_input_batch_scope_rejects_unbatched_sessions_first() {
        let scope = GroupInputBatchScope::from_batch_ids([None, Some("batch-a"), Some("batch-b")]);

        let (code, message) = scope.error().expect("scope error");
        assert_eq!(code, "SESSION_NOT_IN_BATCH");
        assert_eq!(message, "session is not part of a batch");
    }

    #[test]
    fn group_input_batch_scope_rejects_mixed_batches() {
        let scope = GroupInputBatchScope::from_batch_ids([Some("batch-a"), Some("batch-b")]);

        let (code, message) = scope.error().expect("scope error");
        assert_eq!(code, "SESSION_BATCH_MISMATCH");
        assert_eq!(message, "sessions are not in the same batch");
    }

    #[test]
    fn remote_group_input_target_accepts_same_configured_target() {
        let ids = vec![
            remote_sessions::namespace_session_id("remote-a", "sess-1"),
            remote_sessions::namespace_session_id("remote-a", "sess-2"),
        ];
        let route = remote_group_input_target_for_targets(&ids, &[remote_target("remote-a")])
            .expect("route")
            .expect("remote target");

        assert_eq!(route.target.id, "remote-a");
        assert_eq!(route.remote_session_ids, vec!["sess-1", "sess-2"]);
    }

    #[test]
    fn remote_group_input_target_rejects_mixed_local_and_remote() {
        let ids = vec![
            "local-ready".to_string(),
            remote_sessions::namespace_session_id("remote-a", "sess-1"),
        ];
        let error = remote_group_input_target_for_targets(&ids, &[remote_target("remote-a")])
            .expect_err("mixed local remote must be rejected");

        assert_eq!(error.code, "REMOTE_GROUP_INPUT_MIXED_SCOPE");
        assert_eq!(
            error.message,
            "group input cannot mix local and remote sessions"
        );
    }

    #[test]
    fn remote_group_input_target_rejects_mixed_remote_targets() {
        let ids = vec![
            remote_sessions::namespace_session_id("remote-a", "sess-1"),
            remote_sessions::namespace_session_id("remote-b", "sess-2"),
        ];
        let error = remote_group_input_target_for_targets(
            &ids,
            &[remote_target("remote-a"), remote_target("remote-b")],
        )
        .expect_err("mixed remote targets must be rejected");

        assert_eq!(error.code, "REMOTE_GROUP_INPUT_MIXED_TARGETS");
        assert_eq!(
            error.message,
            "remote group input requires sessions from one launch target"
        );
    }

    #[test]
    fn remote_group_input_target_uses_longest_configured_target_id() {
        let ids = vec![
            remote_sessions::namespace_session_id("zone::west", "sess-1"),
            remote_sessions::namespace_session_id("zone::west", "sess-2"),
        ];
        let route = remote_group_input_target_for_targets(
            &ids,
            &[remote_target("zone"), remote_target("zone::west")],
        )
        .expect("route")
        .expect("remote target");

        assert_eq!(route.target.id, "zone::west");
        assert_eq!(route.remote_session_ids, vec!["sess-1", "sess-2"]);
    }

    #[tokio::test]
    async fn send_group_input_delivery_ack_reports_timeout() {
        let (_ack_tx, ack_rx) = oneshot::channel();
        let error = wait_for_group_input_delivery_ack(ack_rx, Duration::from_millis(1))
            .await
            .expect_err("timeout error");

        assert_eq!(error.code, "INPUT_DELIVERY_TIMEOUT");
        assert_eq!(
            error.message.as_deref(),
            Some("timed out waiting for input delivery confirmation")
        );
    }
}
