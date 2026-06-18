use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;

pub use crate::api::service::{
    create_sessions_batch_result, new_batch_context, session_batch_membership,
    BATCH_CREATE_CONCURRENCY, BATCH_CREATE_MAX_DIRS,
};
use crate::api::session_git_diff::get_git_diff;
use crate::api::AppState;

#[cfg(test)]
use crate::api::remote_sessions;
#[cfg(test)]
use crate::auth::AuthInfo;
#[cfg(test)]
use crate::config::SessionDeleteMode;
#[cfg(test)]
use crate::session::actor::{InputDeliveryResult, SessionCommand};
#[cfg(test)]
use crate::types::{
    AdoptSessionRequest, CreateSessionRequest, CreateSessionsBatchRequest, SessionInputRequest,
    SessionState, TerminalSnapshot,
};
#[cfg(test)]
use crate::types::{
    LaunchTargetSummary, SessionAgentContextResponse, SessionSummary, SessionTranscriptResponse,
};
#[cfg(test)]
use axum::http::StatusCode;
#[cfg(test)]
use axum::response::Response;
#[cfg(test)]
use axum::Extension;

mod context_routes;
mod core_routes;
mod group_input;
mod pane_tail;
#[path = "session_mermaid.rs"]
mod session_mermaid;
mod structured_context;
mod timeline;

#[cfg(test)]
use self::context_routes::{
    agent_context_read_response, fetch_transcript_response, remote_agent_context_response,
    remote_transcript_response, transcript_read_response,
};
use self::context_routes::{get_agent_context, get_transcript, TranscriptQuery};
use self::core_routes::{
    adopt_session, create_session, create_sessions_batch, delete_session, dismiss_attention,
    error_response, get_snapshot, list_sessions, send_input,
};
#[cfg(test)]
use self::core_routes::{
    delete_session_error_response, parse_delete_session_mode, request_terminal_snapshot,
    session_input_delivery_response, snapshot_error_response, DeleteSessionQuery,
    SnapshotRequestError,
};
use self::group_input::send_group_input;
pub use self::group_input::send_group_input_service;
use self::pane_tail::get_pane_tail;
pub(crate) use self::session_mermaid::fetch_mermaid_artifact_response;
use self::session_mermaid::{get_mermaid_artifact, get_plan_file};
use self::timeline::{get_timeline, pinned_item, timeline_excerpt, TimelineBuilder};

#[cfg(test)]
use self::timeline::{
    append_artifact_event, git_diff_has_no_changes, git_diff_timeline_detail,
    git_diff_timeline_summary,
};

#[cfg(test)]
use crate::types::{
    MermaidArtifactResponse, SessionGitDiffResponse, SessionTimelineEvent, SessionTimelinePinned,
    SessionTimelinePinnedItem,
};

#[cfg(test)]
use self::pane_tail::{
    remote_pane_tail_response, request_pane_tail_from_actor,
    request_pane_tail_from_actor_with_timeout, PaneTailError, PANE_TAIL_LINES,
};

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/sessions", get(list_sessions).post(create_session))
        .route("/v1/sessions/adopt", post(adopt_session))
        .route("/v1/sessions/reattach", post(adopt_session))
        .route("/v1/sessions/batch", post(create_sessions_batch))
        .route("/v1/sessions/group-input", post(send_group_input))
        .route("/v1/sessions/{session_id}", delete(delete_session))
        .route(
            "/v1/sessions/{session_id}/attention/dismiss",
            post(dismiss_attention),
        )
        .route("/v1/sessions/{session_id}/input", post(send_input))
        .route(
            "/v1/sessions/{session_id}/agent-context",
            get(get_agent_context),
        )
        .route("/v1/sessions/{session_id}/transcript", get(get_transcript))
        .route("/v1/sessions/{session_id}/timeline", get(get_timeline))
        .route("/v1/sessions/{session_id}/git-diff", get(get_git_diff))
        .route("/v1/sessions/{session_id}/snapshot", get(get_snapshot))
        .route("/v1/sessions/{session_id}/pane-tail", get(get_pane_tail))
        .route(
            "/v1/sessions/{session_id}/mermaid-artifact",
            get(get_mermaid_artifact),
        )
        .route("/v1/sessions/{session_id}/plan-file", get(get_plan_file))
}

#[cfg(test)]
mod tests;
