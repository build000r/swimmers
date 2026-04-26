use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::SessionCommand;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, CreateSessionsBatchRequest,
    CreateSessionsBatchResponse, CreateSessionsBatchResult, ErrorResponse, MermaidArtifactResponse,
    PlanFileResponse, RepoTheme, SessionBatchMembership, SessionInputRequest, SessionInputResponse,
    SessionListResponse, SessionPaneTailResponse, SessionState, SessionSummary, TerminalSnapshot,
};

const BATCH_PROMPT_EXCERPT_MAX_CHARS: usize = 72;
const BATCH_LABEL_MAX_CHARS: usize = 28;

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
            Json(CreateSessionResponse {
                session,
                repo_theme,
            }),
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            // The supervisor returns anyhow errors. We detect specific failure
            // modes by inspecting the error message.
            if msg.contains("already exists") || msg.contains("duplicate session") {
                (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse {
                        code: "SESSION_ALREADY_EXISTS".to_string(),
                        message: Some(msg),
                    }),
                )
                    .into_response()
            } else {
                tracing::error!("create_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        code: "INTERNAL_ERROR".to_string(),
                        message: Some(msg),
                    }),
                )
                    .into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/batch
// ---------------------------------------------------------------------------

async fn create_sessions_batch(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionsBatchRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if body.dirs.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                code: "VALIDATION_FAILED".to_string(),
                message: Some("dirs must not be empty".to_string()),
            }),
        )
            .into_response();
    }

    let total = body.dirs.len();
    let spawn_tool = body.spawn_tool;
    let initial_request = body.initial_request;
    let (batch_id, batch_label, batch_created_at, prompt_excerpt) =
        new_batch_context(total, initial_request.as_deref());
    let supervisor = state.supervisor.clone();
    let tasks = body.dirs.into_iter().enumerate().map(|(index, cwd)| {
        let supervisor = supervisor.clone();
        let initial_request = initial_request.clone();
        let batch = session_batch_membership(
            batch_id.clone(),
            batch_label.clone(),
            index,
            total,
            batch_created_at,
            prompt_excerpt.clone(),
        );
        async move {
            let created = supervisor
                .create_session_with_batch(
                    None,
                    Some(cwd.clone()),
                    spawn_tool,
                    initial_request,
                    Some(batch),
                )
                .await;
            create_sessions_batch_result(index, cwd, created)
        }
    });

    let results: Vec<_> = futures::future::join_all(tasks).await;
    let status = if results.iter().all(|result| result.ok) {
        StatusCode::CREATED
    } else {
        StatusCode::MULTI_STATUS
    };

    (status, Json(CreateSessionsBatchResponse { results })).into_response()
}

pub fn session_batch_membership(
    id: String,
    label: String,
    index: usize,
    total: usize,
    created_at: DateTime<Utc>,
    prompt_excerpt: Option<String>,
) -> SessionBatchMembership {
    SessionBatchMembership {
        id,
        label,
        index,
        total,
        created_at,
        prompt_excerpt,
    }
}

pub fn new_batch_context(
    total: usize,
    initial_request: Option<&str>,
) -> (String, String, DateTime<Utc>, Option<String>) {
    let batch_id = format!("batch-{}", Uuid::new_v4().simple());
    let created_at = Utc::now();
    let prompt_excerpt = prompt_excerpt(initial_request);
    let label = batch_label(prompt_excerpt.as_deref(), &batch_id);
    debug_assert!(total > 0);
    (batch_id, label, created_at, prompt_excerpt)
}

fn prompt_excerpt(prompt: Option<&str>) -> Option<String> {
    let normalized = prompt?.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(truncate_chars(normalized, BATCH_PROMPT_EXCERPT_MAX_CHARS))
}

fn batch_label(prompt_excerpt: Option<&str>, batch_id: &str) -> String {
    prompt_excerpt
        .map(|excerpt| truncate_chars(excerpt, BATCH_LABEL_MAX_CHARS))
        .unwrap_or_else(|| {
            let suffix = batch_id
                .strip_prefix("batch-")
                .unwrap_or(batch_id)
                .chars()
                .take(8)
                .collect::<String>();
            format!("batch {suffix}")
        })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}~")
    } else {
        truncated
    }
}

pub fn create_sessions_batch_result(
    index: usize,
    cwd: String,
    created: anyhow::Result<(SessionSummary, Option<RepoTheme>)>,
) -> CreateSessionsBatchResult {
    match created {
        Ok((session, repo_theme)) => CreateSessionsBatchResult {
            index,
            cwd,
            ok: true,
            session: Some(session),
            repo_theme,
            error: None,
        },
        Err(err) => {
            let msg = err.to_string();
            CreateSessionsBatchResult {
                index,
                cwd,
                ok: false,
                session: None,
                repo_theme: None,
                error: Some(create_session_error(&msg)),
            }
        }
    }
}

fn create_session_error(msg: &str) -> ErrorResponse {
    let code = if msg.contains("already exists") || msg.contains("duplicate session") {
        "SESSION_ALREADY_EXISTS"
    } else if msg.contains("cwd does not exist") {
        "VALIDATION_FAILED"
    } else {
        "INTERNAL_ERROR"
    };

    ErrorResponse {
        code: code.to_string(),
        message: Some(msg.to_string()),
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
                Json(ErrorResponse {
                    code: "VALIDATION_FAILED".to_string(),
                    message: Some(format!("invalid delete mode: {}", other)),
                }),
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
                    Json(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    }),
                )
                    .into_response()
            } else {
                tracing::error!("delete_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        code: "INTERNAL_ERROR".to_string(),
                        message: Some(msg),
                    }),
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
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::error!("[session {session_id}] dismiss_attention send failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some(e.to_string()),
            }),
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
            Json(ErrorResponse {
                code: "VALIDATION_FAILED".to_string(),
                message: Some("text must not be empty".to_string()),
            }),
        )
            .into_response();
    }

    let summary = match fetch_live_summary(&state, &session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("send_input summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response();
        }
    };

    if summary.state == SessionState::Exited {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                code: "SESSION_EXITED".to_string(),
                message: Some("session has already exited".to_string()),
            }),
        )
            .into_response();
    }

    let handle = match state.supervisor.get_session(&session_id).await {
        Some(handle) => handle,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
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
            Json(ErrorResponse {
                code: "SESSION_NOT_FOUND".to_string(),
                message: Some(err.to_string()),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(SessionInputResponse {
            ok: true,
            session_id,
        }),
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
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<TerminalSnapshot>();
    if handle.send(SessionCommand::GetSnapshot(tx)).await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("session actor unavailable".to_string()),
            }),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(snapshot)) => (StatusCode::OK, Json(snapshot)).into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("actor dropped snapshot reply".to_string()),
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("snapshot request timed out".to_string()),
            }),
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
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
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
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("session actor unavailable".to_string()),
            }),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(text)) => (
            StatusCode::OK,
            Json(SessionPaneTailResponse { session_id, text }),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("actor dropped pane tail reply".to_string()),
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("pane tail request timed out".to_string()),
            }),
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
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
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
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("session actor unavailable".to_string()),
            }),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(artifact)) => (StatusCode::OK, Json(artifact)).into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("actor dropped mermaid artifact reply".to_string()),
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("mermaid artifact request timed out".to_string()),
            }),
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
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
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
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("session actor unavailable".to_string()),
            }),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)).into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("actor dropped plan file reply".to_string()),
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("plan file request timed out".to_string()),
            }),
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
        .route("/v1/sessions/batch", post(create_sessions_batch))
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
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    use serde_json::Value;
    use std::collections::BTreeMap;
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
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
                std::time::Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
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
            batch: None,
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

    const FAKE_TMUX_FOR_CREATE: &str = r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  new-session|attach-session)
    while IFS= read -r line; do
      printf '%s\r\n' "$line"
    done
    ;;
  send-keys|kill-session)
    exit 0
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_path}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_CWD:-/tmp/project}" ;;
      "#{pane_current_command}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_COMMAND:-zsh}" ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
  capture-pane)
    printf 'captured pane\n'
    ;;
  list-sessions)
    exit 0
    ;;
esac
"##;

    fn generated_dir_name_sets() -> Vec<Vec<String>> {
        let mut runner = TestRunner::deterministic();
        let name = proptest::string::string_regex("[a-z]{1,8}").expect("valid regex");
        let strategy = proptest::collection::btree_set(name, 1..=4);
        (0..4)
            .map(|_| {
                strategy
                    .new_tree(&mut runner)
                    .expect("generate dir names")
                    .current()
                    .into_iter()
                    .collect()
            })
            .collect()
    }

    fn create_case_dirs(root: &FsPath, case_index: usize, names: &[String]) -> Vec<String> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let path = root.join(format!("case-{case_index}-{index}-{name}"));
                std::fs::create_dir_all(&path).expect("create test cwd");
                path.to_string_lossy().into_owned()
            })
            .collect()
    }

    async fn create_batch(state: Arc<AppState>, dirs: Vec<String>) -> axum::response::Response {
        create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(CreateSessionsBatchRequest {
                dirs,
                spawn_tool: None,
                initial_request: None,
            }),
        )
        .await
        .into_response()
    }

    async fn cleanup_created_sessions(state: &Arc<AppState>, json: &Value) {
        let Some(results) = json["results"].as_array() else {
            return;
        };
        for result in results {
            let Some(session_id) = result["session"]["session_id"].as_str() else {
                continue;
            };
            let _ = state
                .supervisor
                .delete_session(session_id, crate::config::SessionDeleteMode::DetachBridge)
                .await;
        }
    }

    fn cwd_result_classes(json: &Value) -> BTreeMap<String, bool> {
        json["results"]
            .as_array()
            .expect("results array")
            .iter()
            .map(|result| {
                (
                    result["cwd"].as_str().expect("cwd").to_string(),
                    result["ok"].as_bool().expect("ok"),
                )
            })
            .collect()
    }

    fn success_count(json: &Value) -> usize {
        json["results"]
            .as_array()
            .expect("results array")
            .iter()
            .filter(|result| result["ok"].as_bool() == Some(true))
            .count()
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
    async fn create_sessions_batch_requires_write_scope() {
        let response = create_sessions_batch(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionsBatchRequest {
                dirs: vec!["/tmp/project".to_string()],
                spawn_tool: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_sessions_batch_rejects_empty_dirs() {
        let response = create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionsBatchRequest {
                dirs: Vec::new(),
                spawn_tool: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
        assert_eq!(json["message"], "dirs must not be empty");
    }

    #[tokio::test]
    async fn create_sessions_batch_assigns_shared_batch_metadata() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");
        let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);

        let response = create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Json(CreateSessionsBatchRequest {
                dirs,
                spawn_tool: None,
                initial_request: Some("wire jwt refresh + tests".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::CREATED);
        let json = response_json(response).await;
        let results = json["results"].as_array().expect("results");
        let first_batch = &results[0]["session"]["batch"];
        let second_batch = &results[1]["session"]["batch"];

        assert!(first_batch["id"]
            .as_str()
            .expect("batch id")
            .starts_with("batch-"));
        assert_eq!(second_batch["id"], first_batch["id"]);
        assert_eq!(first_batch["label"], "wire jwt refresh + tests");
        assert_eq!(first_batch["prompt_excerpt"], "wire jwt refresh + tests");
        assert_eq!(first_batch["index"], 0);
        assert_eq!(second_batch["index"], 1);
        assert_eq!(first_batch["total"], 2);
        assert_eq!(second_batch["total"], 2);
        assert!(first_batch["created_at"].is_string());

        cleanup_created_sessions(&state, &json).await;
    }

    #[tokio::test]
    async fn create_sessions_batch_mr_permutation_preserves_cwd_result_classes() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");

        for (case_index, names) in generated_dir_name_sets().into_iter().enumerate() {
            let dirs = create_case_dirs(root.path(), case_index, &names);
            let reversed_dirs = dirs.iter().rev().cloned().collect::<Vec<_>>();

            let response = create_batch(state.clone(), dirs.clone()).await;
            assert_eq!(response.status(), StatusCode::CREATED);
            let forward_json = response_json(response).await;

            let response = create_batch(state.clone(), reversed_dirs).await;
            assert_eq!(response.status(), StatusCode::CREATED);
            let reversed_json = response_json(response).await;

            assert_eq!(
                cwd_result_classes(&forward_json),
                cwd_result_classes(&reversed_json)
            );

            cleanup_created_sessions(&state, &forward_json).await;
            cleanup_created_sessions(&state, &reversed_json).await;
        }
    }

    #[tokio::test]
    async fn create_sessions_batch_mr_additive_valid_dir_increases_success_count() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");
        let base_dirs =
            create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
        let mut extended_dirs = base_dirs.clone();
        extended_dirs.extend(create_case_dirs(root.path(), 1, &["docs".to_string()]));

        let response = create_batch(state.clone(), base_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let base_json = response_json(response).await;

        let response = create_batch(state.clone(), extended_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let extended_json = response_json(response).await;

        assert_eq!(success_count(&extended_json), success_count(&base_json) + 1);
        assert_eq!(
            extended_json["results"].as_array().expect("results").len(),
            base_json["results"].as_array().expect("results").len() + 1
        );

        cleanup_created_sessions(&state, &base_json).await;
        cleanup_created_sessions(&state, &extended_json).await;
    }

    #[tokio::test]
    async fn create_sessions_batch_mr_invalid_dir_injection_is_exclusive() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");
        let valid_dirs = create_case_dirs(
            root.path(),
            0,
            &["frontend".to_string(), "backend".to_string()],
        );
        let missing_dir = root.path().join("missing").to_string_lossy().into_owned();
        let dirs = vec![
            valid_dirs[0].clone(),
            missing_dir.clone(),
            valid_dirs[1].clone(),
        ];

        let response = create_batch(state.clone(), dirs).await;
        assert_eq!(response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(response).await;
        let results = json["results"].as_array().expect("results");

        assert_eq!(results.len(), 3);
        assert_eq!(success_count(&json), 2);
        assert_eq!(results[1]["index"], 1);
        assert_eq!(results[1]["cwd"], missing_dir);
        assert_eq!(results[1]["ok"], false);
        assert_eq!(results[1]["error"]["code"], "VALIDATION_FAILED");
        assert!(results[0]["session"]["session_id"].is_string());
        assert!(results[2]["session"]["session_id"].is_string());

        cleanup_created_sessions(&state, &json).await;
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
    async fn list_sessions_perf_gate_skips_hung_tmux_active_pane_lookup() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, _path_guard) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 2
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
EOF
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
        for index in 1..=2 {
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
        .expect("session list should succeed when tmux stalls");
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
            elapsed < Duration::from_millis(900),
            "expected /v1/sessions to degrade gracefully when tmux list-panes stalls, got {:?}",
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
