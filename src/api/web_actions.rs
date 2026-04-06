use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use mermaid_rs_renderer::{compute_layout, parse_mermaid, render_svg, RenderOptions};
use tokio::sync::oneshot;

use crate::api::AppState;
#[cfg(feature = "personal-workflows")]
use crate::api::fetch_live_summary;
use crate::auth::{AuthInfo, AuthScope};
use crate::host_actions::{ArtifactOpener, SystemArtifactOpener};
#[cfg(feature = "personal-workflows")]
use crate::host_actions::{CommitLauncher, SystemCommitLauncher};
use crate::session::actor::SessionCommand;
use crate::types::{ErrorResponse, MermaidArtifactResponse};
#[cfg(feature = "personal-workflows")]
use crate::types::SessionState;

#[cfg(feature = "personal-workflows")]
#[derive(Debug, serde::Serialize)]
struct CommitCodexLaunchResponse {
    session_name: String,
    watch_command: String,
}

#[derive(Debug, serde::Serialize)]
struct MermaidArtifactOpenResponse {
    ok: bool,
    session_id: String,
    path: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    let router = Router::new()
        .route(
            "/v1/sessions/{session_id}/mermaid-artifact/svg",
            get(get_mermaid_artifact_svg),
        )
        .route(
            "/v1/sessions/{session_id}/mermaid-artifact/open",
            post(post_open_mermaid_artifact),
        );

    #[cfg(feature = "personal-workflows")]
    let router = router.route(
        "/v1/sessions/{session_id}/commit-codex",
        post(post_commit_codex),
    );

    router
}

fn json_error(status: StatusCode, code: &str, message: impl Into<Option<String>>) -> Response {
    (
        status,
        Json(
            serde_json::to_value(ErrorResponse {
                code: code.to_string(),
                message: message.into(),
            })
            .unwrap(),
        ),
    )
        .into_response()
}

#[cfg(feature = "personal-workflows")]
async fn post_commit_codex(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    post_commit_codex_with_launcher(state, session_id, &SystemCommitLauncher).await
}

#[cfg(feature = "personal-workflows")]
async fn post_commit_codex_with_launcher<L: CommitLauncher>(
    state: Arc<AppState>,
    session_id: String,
    launcher: &L,
) -> Response {
    let summary = match fetch_live_summary(&state, &session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return json_error(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
        Err(err) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            );
        }
    };

    if summary.state == SessionState::Exited {
        return json_error(
            StatusCode::CONFLICT,
            "SESSION_EXITED",
            Some("session has already exited".to_string()),
        );
    }

    match launcher.launch(&summary) {
        Ok(launch) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(CommitCodexLaunchResponse {
                    session_name: launch.session_name,
                    watch_command: launch.watch_command,
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "COMMIT_CODEX_LAUNCH_FAILED",
            Some(err.to_string()),
        ),
    }
}

async fn get_mermaid_artifact_svg(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match render_mermaid_artifact_svg(&state, &session_id).await {
        Ok(svg) => (
            [
                (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            svg,
        )
            .into_response(),
        Err(resp) => resp,
    }
}

async fn render_mermaid_artifact_svg(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<String, Response> {
    let artifact = fetch_mermaid_artifact(state, session_id).await?;
    let Some(source) = artifact
        .source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
    else {
        return Err(json_error(
            StatusCode::NOT_FOUND,
            "MERMAID_ARTIFACT_SOURCE_UNAVAILABLE",
            artifact
                .error
                .clone()
                .or(Some("mermaid artifact source is unavailable".to_string())),
        ));
    };

    render_mermaid_svg(source).map_err(|err| {
        json_error(
            StatusCode::BAD_REQUEST,
            "MERMAID_ARTIFACT_RENDER_FAILED",
            Some(err),
        )
    })
}

async fn post_open_mermaid_artifact(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    post_open_mermaid_artifact_with_opener(state, session_id, &SystemArtifactOpener).await
}

async fn post_open_mermaid_artifact_with_opener<O: ArtifactOpener>(
    state: Arc<AppState>,
    session_id: String,
    opener: &O,
) -> Response {
    let artifact = match fetch_mermaid_artifact(&state, &session_id).await {
        Ok(artifact) => artifact,
        Err(resp) => return resp,
    };

    let Some(path) = artifact
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return json_error(
            StatusCode::NOT_FOUND,
            "MERMAID_ARTIFACT_PATH_UNAVAILABLE",
            artifact
                .error
                .clone()
                .or(Some("mermaid artifact path is unavailable".to_string())),
        );
    };

    match opener.open(path) {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(MermaidArtifactOpenResponse {
                    ok: true,
                    session_id,
                    path: path.to_string(),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MERMAID_ARTIFACT_OPEN_FAILED",
            Some(err.to_string()),
        ),
    }
}

async fn fetch_mermaid_artifact(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<MermaidArtifactResponse, Response> {
    let handle = match state.supervisor.get_session(session_id).await {
        Some(handle) => handle,
        None => {
            return Err(json_error(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None));
        }
    };

    let (tx, rx) = oneshot::channel::<MermaidArtifactResponse>();
    if handle
        .send(SessionCommand::GetMermaidArtifact(tx))
        .await
        .is_err()
    {
        return Err(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some("session actor unavailable".to_string()),
        ));
    }

    match tokio::time::timeout(Duration::from_secs(5), rx).await {
        Ok(Ok(artifact)) => Ok(artifact),
        Ok(Err(_)) => Err(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some("actor dropped mermaid artifact reply".to_string()),
        )),
        Err(_) => Err(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some("mermaid artifact request timed out".to_string()),
        )),
    }
}

fn render_mermaid_svg(source: &str) -> Result<String, String> {
    let parsed = parse_mermaid(source).map_err(|err| err.to_string())?;
    let options = RenderOptions::default();
    let layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
    Ok(render_svg(&layout, &options.theme, &options.layout))
}

#[cfg(test)]
#[allow(dead_code)] // test helpers are feature-gated at their call sites
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OBSERVER_SCOPES;
    use crate::config::Config;
    #[cfg(feature = "personal-workflows")]
    use crate::host_actions::CommitCodexLaunch;
    use crate::session::actor::{ActorHandle, SessionCommand};
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{
        RestState, SessionState, SessionSummary, ThoughtSource, ThoughtState, TransportHealth,
    };
    use axum::body::to_bytes;
    use axum::extract::{Extension, Path, State};
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use std::io;
    use std::sync::Mutex;
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

    fn summary(session_id: &str, state: SessionState) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: format!("tmux-{session_id}"),
            state,
            current_command: None,
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: Some("reviewing diff".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Active,
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

    async fn install_summary_handle(state: &Arc<AppState>, session: SessionSummary) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                session.session_id.clone(),
                session.tmux_name.clone(),
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSummary(reply) = cmd {
                    let _ = reply.send(session.clone());
                }
            }
        });
    }

    async fn install_mermaid_handle(
        state: &Arc<AppState>,
        session: SessionSummary,
        artifact: MermaidArtifactResponse,
    ) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                session.session_id.clone(),
                session.tmux_name.clone(),
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetMermaidArtifact(reply) = cmd {
                    let _ = reply.send(artifact.clone());
                }
            }
        });
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    #[cfg(feature = "personal-workflows")]
    #[tokio::test]
    async fn commit_codex_requires_write_scope() {
        let response = post_commit_codex(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-1".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "personal-workflows")]
    #[tokio::test]
    async fn commit_codex_rejects_exited_session() {
        let state = test_state();
        install_summary_handle(&state, summary("sess-1", SessionState::Exited)).await;

        let response = post_commit_codex_with_launcher(
            state,
            "sess-1".to_string(),
            &FakeCommitLauncher::default(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_EXITED");
    }

    #[cfg(feature = "personal-workflows")]
    #[tokio::test]
    async fn commit_codex_launches_with_session_summary() {
        let state = test_state();
        install_summary_handle(&state, summary("sess-1", SessionState::Busy)).await;
        let launcher = FakeCommitLauncher::default();

        let response =
            post_commit_codex_with_launcher(state, "sess-1".to_string(), &launcher).await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_name"], "commit-7-123");
        assert_eq!(json["watch_command"], "tmux a -t commit-7-123");
        assert_eq!(launcher.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn mermaid_svg_requires_read_scope() {
        let response = get_mermaid_artifact_svg(
            Extension(AuthInfo::new(Vec::new())),
            State(test_state()),
            Path("sess-1".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn mermaid_svg_renders_source() {
        let state = test_state();
        install_mermaid_handle(
            &state,
            summary("sess-1", SessionState::Busy),
            MermaidArtifactResponse {
                session_id: "sess-1".to_string(),
                available: true,
                path: Some("/tmp/diagram.mmd".to_string()),
                updated_at: None,
                source: Some("flowchart TD\nA-->B\n".to_string()),
                error: None,
                slice_name: None,
                plan_files: None,
            },
        )
        .await;

        let response = render_mermaid_artifact_svg(&state, "sess-1")
            .await
            .expect("svg response");

        assert!(response.contains("<svg"));
        assert!(response.contains("A"));
    }

    #[tokio::test]
    async fn mermaid_svg_rejects_missing_source() {
        let state = test_state();
        install_mermaid_handle(
            &state,
            summary("sess-1", SessionState::Busy),
            MermaidArtifactResponse {
                session_id: "sess-1".to_string(),
                available: true,
                path: Some("/tmp/diagram.mmd".to_string()),
                updated_at: None,
                source: None,
                error: Some("artifact missing".to_string()),
                slice_name: None,
                plan_files: None,
            },
        )
        .await;

        let response = render_mermaid_artifact_svg(&state, "sess-1")
            .await
            .expect_err("missing source should be rejected");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn mermaid_open_requires_write_scope() {
        let response = post_open_mermaid_artifact(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-1".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn mermaid_open_invokes_opener_with_path() {
        let state = test_state();
        install_mermaid_handle(
            &state,
            summary("sess-1", SessionState::Busy),
            MermaidArtifactResponse {
                session_id: "sess-1".to_string(),
                available: true,
                path: Some("/tmp/diagram.mmd".to_string()),
                updated_at: None,
                source: Some("flowchart TD\nA-->B\n".to_string()),
                error: None,
                slice_name: None,
                plan_files: None,
            },
        )
        .await;
        let opener = FakeArtifactOpener::default();

        let response =
            post_open_mermaid_artifact_with_opener(state, "sess-1".to_string(), &opener).await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["ok"], true);
        assert_eq!(json["path"], "/tmp/diagram.mmd");
        assert_eq!(
            opener.calls.lock().unwrap().as_slice(),
            ["/tmp/diagram.mmd"]
        );
    }

    #[tokio::test]
    async fn mermaid_open_rejects_missing_path() {
        let state = test_state();
        install_mermaid_handle(
            &state,
            summary("sess-1", SessionState::Busy),
            MermaidArtifactResponse {
                session_id: "sess-1".to_string(),
                available: true,
                path: None,
                updated_at: None,
                source: Some("flowchart TD\nA-->B\n".to_string()),
                error: Some("artifact path unavailable".to_string()),
                slice_name: None,
                plan_files: None,
            },
        )
        .await;

        let response = post_open_mermaid_artifact_with_opener(
            state,
            "sess-1".to_string(),
            &FakeArtifactOpener::default(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "MERMAID_ARTIFACT_PATH_UNAVAILABLE");
    }

    #[cfg(feature = "personal-workflows")]
    #[derive(Default)]
    struct FakeCommitLauncher {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[cfg(feature = "personal-workflows")]
    impl CommitLauncher for FakeCommitLauncher {
        fn launch(&self, session: &SessionSummary) -> io::Result<CommitCodexLaunch> {
            self.calls.lock().unwrap().push(session.session_id.clone());
            Ok(CommitCodexLaunch {
                session_name: "commit-7-123".to_string(),
                watch_command: "tmux a -t commit-7-123".to_string(),
            })
        }
    }

    #[derive(Default)]
    struct FakeArtifactOpener {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl ArtifactOpener for FakeArtifactOpener {
        fn open(&self, path: &str) -> io::Result<()> {
            self.calls.lock().unwrap().push(path.to_string());
            Ok(())
        }
    }
}
