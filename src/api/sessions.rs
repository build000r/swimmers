use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::SessionCommand;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, ErrorResponse, MermaidArtifactResponse,
    PlanFileResponse, SessionInputRequest, SessionInputResponse, SessionListResponse,
    SessionPaneTailResponse, SessionState, TerminalSnapshot,
};

// ---------------------------------------------------------------------------
// GET /v1/sessions
// ---------------------------------------------------------------------------

async fn list_sessions(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    // Keep the hot polling path cheap. Bootstrap/startup populates repo assets
    // and session discovery; repeated list calls should serve current in-memory
    // state instead of re-running tmux discovery and asset collection.
    let sessions = state.supervisor.list_sessions().await;
    // The version counter is not tracked by the supervisor itself; we use 0
    // as a placeholder. A proper monotonic version can be added to the
    // supervisor later if clients need ETag-style cache validation.
    Ok(Json(SessionListResponse {
        sessions,
        version: 0,
        repo_themes: Default::default(),
    }))
}

// ---------------------------------------------------------------------------
// POST /v1/sessions
// ---------------------------------------------------------------------------

async fn create_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    match state
        .supervisor
        .create_session(body.name, body.cwd, body.spawn_tool, body.initial_request)
        .await
    {
        Ok((session, repo_theme)) => (
            StatusCode::CREATED,
            Json(
                serde_json::to_value(CreateSessionResponse {
                    session,
                    repo_theme,
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            // The supervisor returns anyhow errors. We detect specific failure
            // modes by inspecting the error message.
            if msg.contains("already exists") || msg.contains("duplicate session") {
                (
                    StatusCode::CONFLICT,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "SESSION_ALREADY_EXISTS".to_string(),
                            message: Some(msg),
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            } else {
                tracing::error!("create_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "INTERNAL_ERROR".to_string(),
                            message: Some(msg),
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /v1/sessions/{session_id}
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DeleteSessionQuery {
    mode: Option<String>,
}

async fn delete_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<DeleteSessionQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    let delete_mode = match query.mode.as_deref() {
        None | Some("detach_bridge") => crate::config::SessionDeleteMode::DetachBridge,
        Some("kill_tmux") => crate::config::SessionDeleteMode::KillTmux,
        Some(other) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "VALIDATION_FAILED".to_string(),
                        message: Some(format!("invalid delete mode: {}", other)),
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    match state
        .supervisor
        .delete_session(&session_id, delete_mode)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "SESSION_NOT_FOUND".to_string(),
                            message: None,
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            } else {
                tracing::error!("delete_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "INTERNAL_ERROR".to_string(),
                            message: Some(msg),
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/attention/dismiss
// ---------------------------------------------------------------------------

async fn dismiss_attention(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::error!("[session {session_id}] dismiss_attention send failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(e.to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/input
// ---------------------------------------------------------------------------

async fn send_input(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SessionInputRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if body.text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "VALIDATION_FAILED".to_string(),
                    message: Some("text must not be empty".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    let summary = match fetch_live_summary(&state, &session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("send_input summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "INTERNAL_ERROR".to_string(),
                        message: Some(err.to_string()),
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    if summary.state == SessionState::Exited {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "SESSION_EXITED".to_string(),
                    message: Some("session has already exited".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    let handle = match state.supervisor.get_session(&session_id).await {
        Some(handle) => handle,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    if let Err(err) = handle
        .send(SessionCommand::WriteInput(body.text.into_bytes()))
        .await
    {
        tracing::error!("[session {session_id}] send_input failed: {err}");
        return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: Some(err.to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(SessionInputResponse {
                ok: true,
                session_id,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/snapshot
// ---------------------------------------------------------------------------

async fn get_snapshot(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<TerminalSnapshot>();
    if handle.send(SessionCommand::GetSnapshot(tx)).await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("session actor unavailable".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(snapshot)) => (
            StatusCode::OK,
            Json(serde_json::to_value(snapshot).unwrap()),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("actor dropped snapshot reply".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("snapshot request timed out".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/pane-tail
// ---------------------------------------------------------------------------

async fn get_pane_tail(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<String>();
    if handle
        .send(SessionCommand::GetPaneTail {
            lines: 300,
            reply: tx,
        })
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("session actor unavailable".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(text)) => (
            StatusCode::OK,
            Json(serde_json::to_value(SessionPaneTailResponse { session_id, text }).unwrap()),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("actor dropped pane tail reply".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("pane tail request timed out".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/mermaid-artifact
// ---------------------------------------------------------------------------

async fn get_mermaid_artifact(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<MermaidArtifactResponse>();
    if handle
        .send(SessionCommand::GetMermaidArtifact(tx))
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("session actor unavailable".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(artifact)) => (
            StatusCode::OK,
            Json(serde_json::to_value(artifact).unwrap()),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("actor dropped mermaid artifact reply".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("mermaid artifact request timed out".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/plan-file?name=plan.md
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PlanFileQuery {
    name: String,
}

async fn get_plan_file(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<PlanFileQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<PlanFileResponse>();
    if handle
        .send(SessionCommand::GetPlanFile {
            name: query.name,
            reply: tx,
        })
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("session actor unavailable".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(response)) => (
            StatusCode::OK,
            Json(serde_json::to_value(response).unwrap()),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("actor dropped plan file reply".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("plan file request timed out".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/sessions", get(list_sessions).post(create_session))
        .route("/v1/sessions/{session_id}", delete(delete_session))
        .route(
            "/v1/sessions/{session_id}/attention/dismiss",
            post(dismiss_attention),
        )
        .route("/v1/sessions/{session_id}/input", post(send_input))
        .route("/v1/sessions/{session_id}/snapshot", get(get_snapshot))
        .route("/v1/sessions/{session_id}/pane-tail", get(get_pane_tail))
        .route(
            "/v1/sessions/{session_id}/mermaid-artifact",
            get(get_mermaid_artifact),
        )
        .route("/v1/sessions/{session_id}/plan-file", get(get_plan_file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::{OBSERVER_SCOPES, OPERATOR_SCOPES};
    use crate::config::Config;
    use crate::session::actor::ActorHandle;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{ThoughtSource, ThoughtState, TransportHealth};
    use axum::body::to_bytes;
    use axum::extract::{Json, Path, Query, State};
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use std::ffi::{OsStr, OsString};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path as FsPath;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tempfile::{tempdir, TempDir};
    use tokio::sync::{mpsc, RwLock};

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: None,
            file_store: None,
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
        })
    }

    fn summary(session_id: &str, state: SessionState) -> crate::types::SessionSummary {
        crate::types::SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: format!("tmux-{session_id}"),
            state,
            current_command: None,
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: crate::types::fallback_rest_state(state, ThoughtState::Holding),
            commit_candidate: false,
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
        }
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    struct TestPathGuard(Option<OsString>);

    impl Drop for TestPathGuard {
        fn drop(&mut self) {
            if let Some(value) = self.0.take() {
                std::env::set_var("PATH", value);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }

    fn write_executable(path: &FsPath, contents: &str) {
        std::fs::write(path, contents).expect("write executable");
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    fn prepend_test_path(bin_dir: &FsPath, original_path: Option<&OsStr>) {
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = original_path {
            entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
        }
        std::env::set_var("PATH", std::env::join_paths(entries).expect("path"));
    }

    fn install_fake_tmux(script: &str) -> (TempDir, TestPathGuard) {
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        write_executable(&bin_dir.join("tmux"), script);
        let original_path = std::env::var_os("PATH");
        prepend_test_path(&bin_dir, original_path.as_deref());
        (dir, TestPathGuard(original_path))
    }

    async fn spawn_summary_handle(summary: crate::types::SessionSummary) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(
            summary.session_id.clone(),
            summary.tmux_name.clone(),
            cmd_tx,
        );
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    #[tokio::test]
    async fn create_session_requires_write_scope() {
        let response = create_session(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionRequest {
                name: None,
                cwd: None,
                spawn_tool: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn delete_session_rejects_invalid_mode() {
        let response = delete_session(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-missing".to_string()),
            Query(DeleteSessionQuery {
                mode: Some("invalid".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
    }

    #[tokio::test]
    async fn send_input_rejects_empty_text() {
        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: String::new(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
    }

    #[tokio::test]
    async fn send_input_forwards_text_to_session_actor() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
            .await;

        let worker = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary("sess-1", SessionState::Idle));
                    }
                    SessionCommand::WriteInput(bytes) => return bytes,
                    _ => {}
                }
            }
            Vec::new()
        });

        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(worker.await.expect("worker"), b"status".to_vec());
    }

    #[tokio::test]
    async fn get_snapshot_returns_actor_snapshot() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-snap", "tmux-snap", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSnapshot(reply) = cmd {
                    let _ = reply.send(TerminalSnapshot {
                        session_id: "sess-snap".to_string(),
                        latest_seq: 9,
                        truncated: false,
                        screen_text: "hello from tmux".to_string(),
                    });
                    break;
                }
            }
        });

        let response = get_snapshot(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-snap".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-snap");
        assert_eq!(json["screen_text"], "hello from tmux");
    }

    #[tokio::test]
    async fn list_sessions_perf_gate_batches_tmux_lookup_within_budget() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, _path_guard) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 0.20
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
work-3	1	1	0.0:%3
work-4	1	1	0.0:%4
work-5	1	1	0.0:%5
work-6	1	1	0.0:%6
EOF
    ;;
  display-message)
    sleep 0.20
    printf '0.0:%%1\n'
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
        );

        let state = test_state();
        let mut expected_ids = Vec::new();
        for index in 1..=6 {
            let session_id = format!("sess-{index}");
            let mut live_summary = summary(&session_id, SessionState::Idle);
            live_summary.tmux_name = format!("work-{index}");
            state
                .supervisor
                .insert_test_handle(spawn_summary_handle(live_summary).await)
                .await;
            expected_ids.push(session_id);
        }

        let started = Instant::now();
        let Json(payload) = list_sessions(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state),
        )
        .await
        .expect("session list should succeed");
        let elapsed = started.elapsed();

        let mut actual_ids = payload
            .sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<Vec<_>>();
        actual_ids.sort();
        expected_ids.sort();

        assert_eq!(actual_ids, expected_ids);
        assert!(
            elapsed < Duration::from_millis(500),
            "expected /v1/sessions under 500ms, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn get_pane_tail_returns_actor_text() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetPaneTail { lines, reply } = cmd {
                    assert_eq!(lines, 300);
                    let _ = reply.send("recent pane output".to_string());
                    break;
                }
            }
        });

        let response = get_pane_tail(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-tail".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-tail");
        assert_eq!(json["text"], "recent pane output");
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
                        updated_at: Some(Utc::now()),
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
}
