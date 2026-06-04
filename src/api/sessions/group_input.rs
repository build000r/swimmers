use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::api::envelope::error_body;
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::operator_pressure::session_ready_for_operator_group_input;
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};
use crate::types::{
    ErrorResponse, SessionGroupInputRequest, SessionGroupInputResponse, SessionGroupInputResult,
    SessionState, SessionSummary,
};

const INPUT_DELIVERY_ACK_TIMEOUT: Duration = Duration::from_secs(2);

fn group_input_error_result(
    session_id: String,
    code: impl Into<String>,
    message: Option<String>,
) -> SessionGroupInputResult {
    SessionGroupInputResult {
        session_id,
        ok: false,
        error: Some(error_body(code, message)),
    }
}

struct ValidatedGroupInputRequest {
    session_ids: Vec<String>,
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

    let text = body.text.trim().to_string();
    if text.is_empty() {
        return Err(error_body(
            "VALIDATION_FAILED",
            Some("text must not be empty".to_string()),
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

async fn send_group_input_to_ready_session(
    state: &Arc<AppState>,
    session_id: &str,
    input: &[u8],
) -> Result<(), GroupInputSessionError> {
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
) -> Result<(), GroupInputSessionError> {
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
) -> Result<(), GroupInputSessionError> {
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

fn classify_group_input_delivery_ack(
    delivery: InputDeliveryResult,
) -> Result<(), GroupInputSessionError> {
    if delivery.delivered {
        Ok(())
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

    if let Err(error) = send_group_input_to_ready_session(state, &session_id, input).await {
        return error.into_result(session_id);
    }

    SessionGroupInputResult {
        session_id,
        ok: true,
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

pub async fn send_group_input_service(
    state: Arc<AppState>,
    body: SessionGroupInputRequest,
) -> Result<SessionGroupInputResponse, ErrorResponse> {
    let ValidatedGroupInputRequest { session_ids, input } = validate_group_input_request(body)?;
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
        Err(error) => (StatusCode::BAD_REQUEST, Json(error)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
