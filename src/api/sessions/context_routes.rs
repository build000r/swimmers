use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    LaunchTargetSummary, SessionAgentContextResponse, SessionSummary, SessionTranscriptResponse,
};

use super::error_response;
use super::structured_context::{read_agent_context_for_summary, read_transcript_for_summary};

#[derive(Debug, Deserialize)]
pub(super) struct TranscriptQuery {
    pub(super) turn_id: Option<String>,
    pub(super) after: Option<u64>,
    pub(super) limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/agent-context
// ---------------------------------------------------------------------------

pub(super) async fn get_agent_context(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_agent_context_response(&state, &session_id).await
}

async fn fetch_agent_context_response(state: &Arc<AppState>, session_id: &str) -> Response {
    if let Some(response) = remote_agent_context_response_for_session_id(session_id).await {
        return response;
    }

    local_agent_context_response(state, session_id).await
}

async fn remote_agent_context_response_for_session_id(session_id: &str) -> Option<Response> {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            Some(remote_agent_context_response(&target, remote_session_id).await)
        }
        Ok(None) => None,
        Err(err) => Some(err.into_response()),
    }
}

pub(super) async fn remote_agent_context_response(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Response {
    match remote_sessions::fetch_remote_agent_context(target, remote_session_id).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn local_agent_context_response(state: &Arc<AppState>, session_id: &str) -> Response {
    let summary = match agent_context_summary(state, session_id).await {
        Ok(summary) => summary,
        Err(response) => return response,
    };

    agent_context_read_response(read_agent_context_for_summary(summary).await)
}

async fn agent_context_summary(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<SessionSummary, Response> {
    match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => Ok(summary),
        Ok(None) => Err(error_response(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            None,
        )),
        Err(err) => {
            tracing::error!("agent context summary lookup failed: {err}");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            ))
        }
    }
}

pub(super) fn agent_context_read_response(
    result: anyhow::Result<SessionAgentContextResponse>,
) -> Response {
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => {
            tracing::error!("agent context read failed: {err}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/transcript
// ---------------------------------------------------------------------------

pub(super) async fn get_transcript(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<TranscriptQuery>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_transcript_response(&state, &session_id, query).await
}

pub(super) async fn fetch_transcript_response(
    state: &Arc<AppState>,
    session_id: &str,
    query: TranscriptQuery,
) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            remote_transcript_response(&target, remote_session_id, query).await
        }
        Ok(None) => local_transcript_response(state, session_id, query).await,
        Err(err) => err.into_response(),
    }
}

pub(super) async fn remote_transcript_response(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
    query: TranscriptQuery,
) -> Response {
    match remote_sessions::fetch_remote_transcript(
        target,
        remote_session_id,
        query.turn_id.as_deref(),
        query.after,
        query.limit,
    )
    .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn local_transcript_response(
    state: &Arc<AppState>,
    session_id: &str,
    query: TranscriptQuery,
) -> Response {
    let summary = match transcript_summary_for_response(state, session_id).await {
        Ok(summary) => summary,
        Err(response) => return response,
    };

    transcript_read_response(read_transcript_for_summary(summary, query).await)
}

async fn transcript_summary_for_response(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<SessionSummary, Response> {
    match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => Ok(summary),
        Ok(None) => Err(error_response(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            None,
        )),
        Err(err) => {
            tracing::error!("transcript summary lookup failed: {err}");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            ))
        }
    }
}

pub(super) fn transcript_read_response(
    result: anyhow::Result<SessionTranscriptResponse>,
) -> Response {
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => {
            tracing::error!("transcript read failed: {err}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            )
        }
    }
}
