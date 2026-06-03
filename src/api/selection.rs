use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use chrono::Utc;

use crate::api::envelope::{
    api_error, error_body_msg, parse_if_match_version, reserve_version_locked, success_json,
    VERSION_CONFLICT,
};
use crate::api::{fetch_live_summary, AppState, PublishedSelectionState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    PublishSelectionRequest, PublishedSelectionResponse, SessionState, SessionSummary,
};

static SELECTION_VERSION: AtomicU64 = AtomicU64::new(0);

async fn get_published_selection(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<PublishedSelectionResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;

    let snapshot = state.published_selection.read().await.clone();
    let summary = fetch_published_selection_summary(&state, &snapshot).await?;

    Ok(Json(published_selection_response(snapshot, summary)))
}

async fn fetch_published_selection_summary(
    state: &Arc<AppState>,
    snapshot: &PublishedSelectionState,
) -> Result<Option<SessionSummary>, axum::response::Response> {
    let Some(session_id) = snapshot.session_id.as_deref() else {
        return Ok(None);
    };

    match fetch_live_summary(state, session_id).await {
        Ok(summary) => Ok(summary),
        Err(err) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_body_msg("INTERNAL_ERROR", err.to_string())),
        )
            .into_response()),
    }
}

fn published_selection_response(
    snapshot: PublishedSelectionState,
    summary: Option<SessionSummary>,
) -> PublishedSelectionResponse {
    let Some(session_id) = snapshot.session_id else {
        return PublishedSelectionResponse {
            session_id: None,
            session: None,
            published_at: snapshot.published_at,
            error: None,
        };
    };

    match summary {
        Some(summary) if summary.state == SessionState::Exited => PublishedSelectionResponse {
            session_id: Some(session_id),
            session: Some(summary),
            published_at: snapshot.published_at,
            error: Some(error_body_msg(
                "SESSION_EXITED",
                "session has already exited",
            )),
        },
        Some(summary) => PublishedSelectionResponse {
            session_id: Some(session_id),
            session: Some(summary),
            published_at: snapshot.published_at,
            error: None,
        },
        None => PublishedSelectionResponse {
            session_id: Some(session_id),
            session: None,
            published_at: snapshot.published_at,
            error: Some(error_body_msg("SESSION_NOT_FOUND", "session not found")),
        },
    }
}

async fn publish_selection(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PublishSelectionRequest>,
) -> Result<axum::response::Response, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsWrite)?;

    let requested_version = parse_if_match_version(&headers);

    let session_id = body.session_id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let published_at = session_id.as_ref().map(|_| Utc::now());

    // Hold the write lock while reserving and committing the version so the
    // version handed to the client always matches the order in which state
    // writes land.
    let mut selection = state.published_selection.write().await;
    let new_version = reserve_version_locked(&SELECTION_VERSION, requested_version)
        .ok_or_else(|| api_error(&VERSION_CONFLICT))?
        .commit();
    *selection = PublishedSelectionState {
        session_id,
        published_at,
    };
    drop(selection);

    Ok(success_json(
        axum::http::StatusCode::OK,
        &serde_json::json!({ "ok": true, "version": new_version }),
    ))
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/v1/selection",
        get(get_published_selection).put(publish_selection),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::StateEvidence;
    use axum::body::to_bytes;
    use axum::extract::{Json, State};
    use axum::http::HeaderMap;
    use serde_json::Value;
    use std::sync::atomic::Ordering;
    use std::sync::LazyLock;
    use tokio::sync::Mutex;
    use tokio::sync::RwLock;

    static VERSION_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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

    fn selected_snapshot() -> PublishedSelectionState {
        PublishedSelectionState {
            session_id: Some("sess-1".to_string()),
            published_at: Some(Utc::now()),
        }
    }

    fn summary(session_id: &str, state: SessionState) -> SessionSummary {
        SessionSummary::live(
            session_id,
            format!("tmux-{session_id}"),
            state,
            None,
            StateEvidence::new("test"),
            "/tmp/project",
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        )
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    async fn publish_response(
        state: Arc<AppState>,
        session_id: Option<&str>,
        if_match: Option<u64>,
    ) -> axum::response::Response {
        let mut headers = HeaderMap::new();
        if let Some(v) = if_match {
            headers.insert(
                "if-match",
                format!("\"{v}\"").parse().expect("header value"),
            );
        }
        let result = publish_selection(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            headers,
            Json(PublishSelectionRequest {
                session_id: session_id.map(|s| s.to_string()),
            }),
        )
        .await;
        result.unwrap_or_else(|resp| resp)
    }

    #[tokio::test]
    async fn get_published_selection_requires_sessions_read_scope() {
        let state = test_state();

        let response = get_published_selection(Extension(AuthInfo::new(Vec::new())), State(state))
            .await
            .expect_err("missing read scope should fail");

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn get_published_selection_returns_empty_selection_without_live_lookup() {
        let state = test_state();

        let Json(response) = get_published_selection(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
        )
        .await
        .expect("empty selection should succeed");

        assert!(response.session_id.is_none());
        assert!(response.session.is_none());
        assert!(response.published_at.is_none());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn fetch_published_selection_summary_skips_live_lookup_when_empty() {
        let state = test_state();
        let snapshot = PublishedSelectionState::default();

        let summary = fetch_published_selection_summary(&state, &snapshot)
            .await
            .expect("empty selection should not fail");

        assert!(summary.is_none());
    }

    #[tokio::test]
    async fn fetch_published_selection_summary_returns_none_for_missing_session() {
        let state = test_state();
        let snapshot = selected_snapshot();

        let summary = fetch_published_selection_summary(&state, &snapshot)
            .await
            .expect("missing selected session should not fail");

        assert!(summary.is_none());
    }

    #[test]
    fn published_selection_response_includes_live_session() {
        let snapshot = selected_snapshot();
        let published_at = snapshot.published_at;

        let response =
            published_selection_response(snapshot, Some(summary("sess-1", SessionState::Idle)));

        assert_eq!(response.session_id.as_deref(), Some("sess-1"));
        assert_eq!(response.published_at, published_at);
        assert_eq!(
            response
                .session
                .as_ref()
                .map(|session| session.session_id.as_str()),
            Some("sess-1")
        );
        assert!(response.error.is_none());
    }

    #[test]
    fn published_selection_response_marks_exited_session() {
        let response = published_selection_response(
            selected_snapshot(),
            Some(summary("sess-1", SessionState::Exited)),
        );

        assert_eq!(response.session_id.as_deref(), Some("sess-1"));
        assert_eq!(
            response.session.as_ref().map(|session| session.state),
            Some(SessionState::Exited)
        );
        let error = response
            .error
            .expect("exited selection should include error");
        assert_eq!(error.code, "SESSION_EXITED");
        assert_eq!(error.message.as_deref(), Some("session has already exited"));
    }

    #[test]
    fn published_selection_response_marks_missing_session() {
        let response = published_selection_response(selected_snapshot(), None);

        assert_eq!(response.session_id.as_deref(), Some("sess-1"));
        assert!(response.session.is_none());
        let error = response
            .error
            .expect("missing selection should include error");
        assert_eq!(error.code, "SESSION_NOT_FOUND");
        assert_eq!(error.message.as_deref(), Some("session not found"));
    }

    #[tokio::test]
    async fn publish_selection_optimistic_concurrency() {
        let _guard = VERSION_TEST_LOCK.lock().await;
        SELECTION_VERSION.store(0, Ordering::Release);
        let state = test_state();

        // (b) missing header → 200 + new version
        let response = publish_response(state.clone(), Some("sess-1"), None).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        let v1 = json["version"].as_u64().expect("version should be present");
        assert!(v1 > 0);

        // (c) matching version → 200 + bumped version
        let response = publish_response(state.clone(), Some("sess-2"), Some(v1)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        let v2 = json["version"].as_u64().expect("version should be present");
        assert_eq!(v2, v1 + 1);

        // (a) stale version → 412
        let response = publish_response(state.clone(), Some("sess-3"), Some(v1)).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::PRECONDITION_FAILED
        );
        let json = response_json(response).await;
        assert_eq!(json["code"], "VERSION_CONFLICT");
    }

    #[tokio::test]
    async fn publish_selection_optimistic_concurrency_rejects_second_concurrent_if_match() {
        let _guard = VERSION_TEST_LOCK.lock().await;
        SELECTION_VERSION.store(0, Ordering::Release);
        let state = test_state();

        let initial = publish_response(state.clone(), Some("seed"), None).await;
        assert_eq!(initial.status(), axum::http::StatusCode::OK);
        let json = response_json(initial).await;
        let expected_version = json["version"].as_u64().expect("version should be present");

        let (resp_a, resp_b) = tokio::join!(
            publish_response(state.clone(), Some("sess-a"), Some(expected_version)),
            publish_response(state, Some("sess-b"), Some(expected_version)),
        );

        let status_a = resp_a.status();
        let status_b = resp_b.status();
        assert!(status_a == axum::http::StatusCode::OK || status_b == axum::http::StatusCode::OK);
        assert!(
            status_a == axum::http::StatusCode::PRECONDITION_FAILED
                || status_b == axum::http::StatusCode::PRECONDITION_FAILED
        );

        let json_a = response_json(resp_a).await;
        let json_b = response_json(resp_b).await;
        let success_json = if status_a == axum::http::StatusCode::OK {
            &json_a
        } else {
            &json_b
        };
        let conflict_json = if status_a == axum::http::StatusCode::PRECONDITION_FAILED {
            &json_a
        } else {
            &json_b
        };

        assert_eq!(
            success_json["version"],
            serde_json::json!(expected_version + 1)
        );
        assert_eq!(conflict_json["code"], "VERSION_CONFLICT");
    }
}
