use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

use super::error_response;
use crate::api::service::{request_plan_file, PlanFileServiceError};
use crate::api::{remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::SessionCommand;
use crate::types::MermaidArtifactResponse;

const MERMAID_ARTIFACT_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/mermaid-artifact
// ---------------------------------------------------------------------------

pub(super) async fn get_mermaid_artifact(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match fetch_mermaid_artifact_response(&state, &session_id).await {
        Ok(artifact) => (StatusCode::OK, Json(artifact)).into_response(),
        Err(resp) => resp,
    }
}

pub(crate) async fn fetch_mermaid_artifact_response(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<MermaidArtifactResponse, Response> {
    if let Some(artifact) = remote_mermaid_artifact_response(session_id).await? {
        return Ok(artifact);
    }

    let handle = mermaid_artifact_actor_handle(state, session_id).await?;
    request_mermaid_artifact_from_actor(&handle)
        .await
        .map_err(mermaid_artifact_request_error_response)
}

async fn remote_mermaid_artifact_response(
    session_id: &str,
) -> Result<Option<MermaidArtifactResponse>, Response> {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            remote_sessions::fetch_remote_mermaid_artifact(&target, remote_session_id)
                .await
                .map(Some)
                .map_err(|err| err.into_response())
        }
        Ok(None) => Ok(None),
        Err(err) => Err(err.into_response()),
    }
}

async fn mermaid_artifact_actor_handle(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<crate::session::actor::ActorHandle, Response> {
    state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None))
}

#[derive(Debug, PartialEq, Eq)]
enum MermaidArtifactRequestError {
    ActorUnavailable,
    ReplyDropped,
    TimedOut,
}

async fn request_mermaid_artifact_from_actor(
    handle: &crate::session::actor::ActorHandle,
) -> Result<MermaidArtifactResponse, MermaidArtifactRequestError> {
    request_mermaid_artifact_from_actor_with_timeout(handle, MERMAID_ARTIFACT_TIMEOUT).await
}

async fn request_mermaid_artifact_from_actor_with_timeout(
    handle: &crate::session::actor::ActorHandle,
    timeout: Duration,
) -> Result<MermaidArtifactResponse, MermaidArtifactRequestError> {
    let (tx, rx) = oneshot::channel::<MermaidArtifactResponse>();
    if handle
        .send(SessionCommand::GetMermaidArtifact(tx))
        .await
        .is_err()
    {
        return Err(MermaidArtifactRequestError::ActorUnavailable);
    }

    classify_mermaid_artifact_reply(tokio::time::timeout(timeout, rx).await)
}

fn classify_mermaid_artifact_reply(
    result: Result<
        Result<MermaidArtifactResponse, oneshot::error::RecvError>,
        tokio::time::error::Elapsed,
    >,
) -> Result<MermaidArtifactResponse, MermaidArtifactRequestError> {
    match result {
        Ok(Ok(artifact)) => Ok(artifact),
        Ok(Err(_)) => Err(MermaidArtifactRequestError::ReplyDropped),
        Err(_) => Err(MermaidArtifactRequestError::TimedOut),
    }
}

fn mermaid_artifact_request_error_response(error: MermaidArtifactRequestError) -> Response {
    match error {
        MermaidArtifactRequestError::ActorUnavailable => {
            mermaid_artifact_internal_error("session actor unavailable")
        }
        MermaidArtifactRequestError::ReplyDropped => {
            mermaid_artifact_internal_error("actor dropped mermaid artifact reply")
        }
        MermaidArtifactRequestError::TimedOut => {
            mermaid_artifact_internal_error("mermaid artifact request timed out")
        }
    }
}

fn mermaid_artifact_internal_error(message: &str) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
        Some(message.to_string()),
    )
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/plan-file?name=plan.md
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct PlanFileQuery {
    name: String,
}

pub(super) async fn get_plan_file(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<PlanFileQuery>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_plan_file_response(&state, &session_id, &query.name).await
}

async fn fetch_plan_file_response(state: &Arc<AppState>, session_id: &str, name: &str) -> Response {
    match request_plan_file(state, session_id, name).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => plan_file_error_response(error),
    }
}

fn plan_file_error_response(error: PlanFileServiceError) -> Response {
    match error {
        PlanFileServiceError::Remote(err) => err.into_response(),
        PlanFileServiceError::SessionNotFound => {
            error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None)
        }
        PlanFileServiceError::ActorUnavailable => {
            plan_file_internal_error("session actor unavailable")
        }
        PlanFileServiceError::ReplyDropped => {
            plan_file_internal_error("actor dropped plan file reply")
        }
        PlanFileServiceError::TimedOut => plan_file_internal_error("plan file request timed out"),
    }
}

fn plan_file_internal_error(message: &str) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
        Some(message.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{once_lock_with, PublishedSelectionState};
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::actor::ActorHandle;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{GhosttyOpenMode, NativeDesktopApp, PlanFileResponse};
    use axum::body::to_bytes;
    use serde_json::Value;
    use tokio::sync::{mpsc, RwLock};

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: once_lock_with(None),
            file_store: once_lock_with(None),
            bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
                Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn get_plan_file_returns_actor_payload() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-plan", "tmux-plan", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetPlanFile { name, reply } = cmd {
                    assert_eq!(name, "plan.md");
                    let _ = reply.send(PlanFileResponse {
                        session_id: "sess-plan".to_string(),
                        name,
                        content: Some("# Plan\n".to_string()),
                        error: None,
                    });
                    break;
                }
            }
        });

        let response = get_plan_file(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-plan".to_string()),
            Query(PlanFileQuery {
                name: "plan.md".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-plan");
        assert_eq!(json["name"], "plan.md");
        assert_eq!(json["content"], "# Plan\n");
    }

    #[tokio::test]
    async fn get_plan_file_returns_not_found_for_missing_session() {
        let response = get_plan_file(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("missing-plan".to_string()),
            Query(PlanFileQuery {
                name: "plan.md".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_plan_file_returns_internal_error_when_actor_drops_reply() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-dropped-plan",
                "tmux-dropped-plan",
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            if let Some(SessionCommand::GetPlanFile { name, reply }) = cmd_rx.recv().await {
                assert_eq!(name, "plan.md");
                drop(reply);
            }
        });

        let response = get_plan_file(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-dropped-plan".to_string()),
            Query(PlanFileQuery {
                name: "plan.md".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INTERNAL_ERROR");
        assert_eq!(json["message"], "actor dropped plan file reply");
    }

    #[tokio::test]
    async fn request_plan_file_service_reports_timeout_with_short_budget() {
        let state = test_state();
        let (cmd_tx, _cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-timeout-plan",
                "tmux-timeout-plan",
                cmd_tx,
            ))
            .await;

        let err = crate::api::service::request_plan_file_with_timeout(
            &state,
            "sess-timeout-plan",
            "plan.md",
            Duration::from_millis(1),
        )
        .await
        .expect_err("plan request should time out");

        assert!(matches!(
            err,
            crate::api::service::PlanFileServiceError::TimedOut
        ));
    }

    #[tokio::test]
    async fn get_mermaid_artifact_returns_actor_payload() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-mermaid",
                "tmux-mermaid",
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetMermaidArtifact(reply) = cmd {
                    let _ = reply.send(MermaidArtifactResponse {
                        session_id: "sess-mermaid".to_string(),
                        available: true,
                        path: Some("/tmp/project/diagram.mmd".to_string()),
                        updated_at: Some(chrono::Utc::now()),
                        source: Some("graph TD\nA-->B\n".to_string()),
                        error: None,
                        slice_name: None,
                        plan_files: None,
                    });
                    break;
                }
            }
        });

        let response = get_mermaid_artifact(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-mermaid".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-mermaid");
        assert_eq!(json["available"], true);
        assert_eq!(json["path"], "/tmp/project/diagram.mmd");
        assert_eq!(json["source"], "graph TD\nA-->B\n");
    }

    #[tokio::test]
    async fn request_mermaid_artifact_from_actor_returns_actor_payload() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle("sess-mermaid", "tmux-mermaid", cmd_tx);

        tokio::spawn(async move {
            if let Some(SessionCommand::GetMermaidArtifact(reply)) = cmd_rx.recv().await {
                let _ = reply.send(MermaidArtifactResponse {
                    session_id: "sess-mermaid".to_string(),
                    available: true,
                    path: Some("/tmp/project/diagram.mmd".to_string()),
                    updated_at: None,
                    source: Some("graph TD\nA-->B\n".to_string()),
                    error: None,
                    slice_name: None,
                    plan_files: None,
                });
            }
        });

        let artifact = request_mermaid_artifact_from_actor(&handle)
            .await
            .expect("artifact");

        assert_eq!(artifact.session_id, "sess-mermaid");
        assert!(artifact.available);
        assert_eq!(artifact.path.as_deref(), Some("/tmp/project/diagram.mmd"));
    }

    #[tokio::test]
    async fn request_mermaid_artifact_from_actor_reports_send_failure() {
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        drop(cmd_rx);
        let handle = ActorHandle::test_handle("sess-mermaid", "tmux-mermaid", cmd_tx);

        let err = request_mermaid_artifact_from_actor(&handle)
            .await
            .expect_err("closed actor command channel");

        assert_eq!(err, MermaidArtifactRequestError::ActorUnavailable);
    }

    #[tokio::test]
    async fn request_mermaid_artifact_from_actor_reports_dropped_reply() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle("sess-mermaid", "tmux-mermaid", cmd_tx);

        tokio::spawn(async move {
            if let Some(SessionCommand::GetMermaidArtifact(reply)) = cmd_rx.recv().await {
                drop(reply);
            }
        });

        let err = request_mermaid_artifact_from_actor(&handle)
            .await
            .expect_err("dropped artifact reply");

        assert_eq!(err, MermaidArtifactRequestError::ReplyDropped);
    }

    #[tokio::test]
    async fn request_mermaid_artifact_from_actor_reports_timeout() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle("sess-mermaid", "tmux-mermaid", cmd_tx);

        tokio::spawn(async move {
            if let Some(SessionCommand::GetMermaidArtifact(reply)) = cmd_rx.recv().await {
                tokio::time::sleep(Duration::from_millis(50)).await;
                drop(reply);
            }
        });

        let err =
            request_mermaid_artifact_from_actor_with_timeout(&handle, Duration::from_millis(1))
                .await
                .expect_err("artifact request timeout");

        assert_eq!(err, MermaidArtifactRequestError::TimedOut);
    }

    #[tokio::test]
    async fn get_mermaid_artifact_returns_internal_error_when_actor_drops_reply() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-dropped-mermaid",
                "tmux-dropped-mermaid",
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            if let Some(SessionCommand::GetMermaidArtifact(reply)) = cmd_rx.recv().await {
                drop(reply);
            }
        });

        let response = get_mermaid_artifact(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-dropped-mermaid".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INTERNAL_ERROR");
        assert_eq!(json["message"], "actor dropped mermaid artifact reply");
    }
}
