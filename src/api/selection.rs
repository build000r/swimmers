use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use chrono::Utc;

use crate::api::envelope::{
    api_error, parse_if_match_version, reserve_version_locked, success_json, VERSION_CONFLICT,
};
use crate::api::{fetch_live_summary, AppState, PublishedSelectionState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    ErrorResponse, PublishSelectionRequest, PublishedSelectionResponse, SessionState,
};

static SELECTION_VERSION: AtomicU64 = AtomicU64::new(0);

async fn get_published_selection(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<PublishedSelectionResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;

    let snapshot = state.published_selection.read().await.clone();
    let Some(session_id) = snapshot.session_id.clone() else {
        return Ok(Json(PublishedSelectionResponse {
            session_id: None,
            session: None,
            published_at: snapshot.published_at,
            error: None,
        }));
    };

    match fetch_live_summary(&state, &session_id).await {
        Ok(Some(summary)) if summary.state == SessionState::Exited => {
            Ok(Json(PublishedSelectionResponse {
                session_id: Some(session_id),
                session: Some(summary),
                published_at: snapshot.published_at,
                error: Some(ErrorResponse {
                    code: "SESSION_EXITED".to_string(),
                    message: Some("session has already exited".to_string()),
                }),
            }))
        }
        Ok(Some(summary)) => Ok(Json(PublishedSelectionResponse {
            session_id: Some(session_id),
            session: Some(summary),
            published_at: snapshot.published_at,
            error: None,
        })),
        Ok(None) => Ok(Json(PublishedSelectionResponse {
            session_id: Some(session_id),
            session: None,
            published_at: snapshot.published_at,
            error: Some(ErrorResponse {
                code: "SESSION_NOT_FOUND".to_string(),
                message: Some("session not found".to_string()),
            }),
        })),
        Err(err) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some(err.to_string()),
            }),
        )
            .into_response()),
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
