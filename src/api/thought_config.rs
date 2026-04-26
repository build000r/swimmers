use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use tracing::error;

use crate::api::envelope::{
    api_error, api_error_msg, parse_if_match_version, reserve_version_locked, success_json,
    INTERNAL_ERROR, PERSISTENCE_UNAVAILABLE, VALIDATION_FAILED, VERSION_CONFLICT,
};
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::openrouter_models::cached_or_default_openrouter_candidates;
use crate::thought::probe::run_thought_config_probe;
use crate::thought::protocol::{build_sync_request, SyncRequest};
use crate::thought::runtime_config::ThoughtConfig;
use crate::thought_ui::thought_config_ui_metadata;
use crate::types::ThoughtConfigResponse;

static THOUGHT_CONFIG_VERSION: AtomicU64 = AtomicU64::new(0);

async fn get_thought_config(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ThoughtConfigResponse>, Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let config = state.thought_config.read().await.clone();
    Ok(Json(ThoughtConfigResponse {
        config,
        daemon_defaults: state.current_daemon_defaults(),
        ui: thought_config_ui_metadata(&cached_or_default_openrouter_candidates()),
    }))
}

async fn get_thought_sync_preview(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SyncRequest>, Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let config = state.thought_config.read().await.clone();
    let sessions = state.supervisor.collect_session_snapshots().await;
    let request = build_sync_request(state.sync_request_sequence.peek_next(), &config, &sessions);
    Ok(Json(request))
}

async fn put_thought_config(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ThoughtConfig>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let config = match body.normalize_and_validate() {
        Ok(config) => config,
        Err(err) => return api_error_msg(&VALIDATION_FAILED, err.to_string()),
    };

    let store = match state.current_file_store() {
        Some(store) => store,
        None => {
            return api_error_msg(
                &PERSISTENCE_UNAVAILABLE,
                "thought config persistence is unavailable",
            );
        }
    };

    let requested_version = parse_if_match_version(&headers);

    // Hold the runtime-config write lock across the version reservation, the
    // disk write, and the in-memory update. This guarantees three things:
    //   * the version returned to the client matches the order in which
    //     state writes commit (no concurrent reorder),
    //   * a failed disk save drops the reservation without committing — the
    //     counter never advances and retries with the original `If-Match`
    //     keep working,
    //   * disk and in-memory state can never diverge.
    let mut runtime_config = state.thought_config.write().await;
    let reservation = match reserve_version_locked(&THOUGHT_CONFIG_VERSION, requested_version) {
        Some(reservation) => reservation,
        None => return api_error(&VERSION_CONFLICT),
    };

    if let Err(err) = store.save_thought_config(&config).await {
        error!(error = %err, "failed to persist thought runtime config");
        return api_error_msg(&INTERNAL_ERROR, "failed to persist thought config");
    }

    *runtime_config = config.clone();
    let new_version = reservation.commit();
    drop(runtime_config);

    let mut body = serde_json::to_value(&config).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = body.as_object_mut() {
        obj.insert("version".to_string(), serde_json::json!(new_version));
    }
    success_json(StatusCode::OK, &body)
}

async fn post_thought_config_test(
    Extension(auth): Extension<AuthInfo>,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<ThoughtConfig>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let config = match body.normalize_and_validate() {
        Ok(config) => config,
        Err(err) => return api_error_msg(&VALIDATION_FAILED, err.to_string()),
    };

    success_json(StatusCode::OK, &run_thought_config_probe(&config).await)
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v1/thought-config",
            get(get_thought_config).put(put_thought_config),
        )
        .route(
            "/v1/thought-config/test",
            axum::routing::post(post_thought_config_test),
        )
        .route("/v1/thought/sync-preview", get(get_thought_sync_preview))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::{AuthScope, OBSERVER_SCOPES, OPERATOR_SCOPES};
    use crate::config::Config;
    use crate::session::actor::{ActorHandle, SessionCommand};
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::types::{
        RestState, SessionState, TerminalSnapshot, ThoughtSource, ThoughtState, TransportHealth,
    };
    use axum::body::to_bytes;
    use axum::extract::{Json, State};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use std::sync::atomic::Ordering;
    use std::sync::LazyLock;
    use tokio::sync::mpsc;
    use tokio::sync::Mutex;
    use tokio::sync::RwLock;

    static VERSION_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn test_state(
        file_store: Option<Arc<crate::persistence::file_store::FileStore>>,
    ) -> Arc<AppState> {
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
            file_store: crate::api::once_lock_with(file_store),
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
            token_count: 55,
            context_limit: 100,
            thought: Some("reviewing diff".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::Llm,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
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

    #[tokio::test]
    async fn put_thought_config_rejects_invalid_payloads() {
        let response = put_thought_config(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state(None)),
            HeaderMap::new(),
            Json(ThoughtConfig {
                cadence_hot_ms: 1,
                ..ThoughtConfig::default()
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
    }

    #[tokio::test]
    async fn get_thought_config_includes_ui_metadata() {
        let response = get_thought_config(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state(None)),
        )
        .await
        .expect("thought config response");

        assert!(!response.0.ui.backends.is_empty());
        assert!(response
            .0
            .ui
            .backends
            .iter()
            .any(|backend| backend.key == "openrouter"));
    }

    #[tokio::test]
    async fn put_thought_config_requires_persistence_store() {
        let response = put_thought_config(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state(None)),
            HeaderMap::new(),
            Json(ThoughtConfig::default()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json = response_json(response).await;
        assert_eq!(json["code"], "PERSISTENCE_UNAVAILABLE");
    }

    #[tokio::test]
    async fn get_thought_sync_preview_returns_live_request() {
        let state = test_state(None);
        {
            let mut config = state.thought_config.write().await;
            config.agent_prompt = Some("Hook wakeup prompt".to_string());
        }

        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary("sess-1", SessionState::Idle));
                    }
                    SessionCommand::GetSnapshot(reply) => {
                        let _ = reply.send(TerminalSnapshot {
                            session_id: "sess-1".to_string(),
                            latest_seq: 9,
                            truncated: false,
                            screen_text: "working".to_string(),
                        });
                        break;
                    }
                    _ => {}
                }
            }
        });

        let response = get_thought_sync_preview(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state),
        )
        .await
        .expect("preview should succeed");

        let json = serde_json::to_value(response.0).expect("preview should serialize");
        assert_eq!(json["type"], "sync");
        assert_eq!(json["sessions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["sessions"][0]["session_id"], "sess-1");
        assert_eq!(json["sessions"][0]["replay_text"], "working");
        assert_eq!(json["sessions"][0]["rest_state"], "drowsy");
        assert_eq!(json["config"]["enabled"], true);
        assert_eq!(json["config"]["agent_prompt"], "Hook wakeup prompt");
    }

    #[tokio::test]
    async fn get_thought_sync_preview_handles_zero_sessions() {
        let response = get_thought_sync_preview(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state(None)),
        )
        .await
        .expect("preview should succeed");

        let json = serde_json::to_value(response.0).expect("preview should serialize");
        assert_eq!(json["type"], "sync");
        assert_eq!(json["sessions"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_thought_sync_preview_requires_read_scope() {
        let response = get_thought_sync_preview(
            Extension(AuthInfo::new(vec![AuthScope::SessionsWrite])),
            State(test_state(None)),
        )
        .await
        .expect_err("preview should require read scope");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn get_thought_sync_preview_is_read_only_for_request_ids() {
        let state = test_state(None);

        let first = get_thought_sync_preview(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state.clone()),
        )
        .await
        .expect("first preview should succeed");
        let second = get_thought_sync_preview(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state.clone()),
        )
        .await
        .expect("second preview should succeed");

        assert_eq!(first.0.id, "1");
        assert_eq!(second.0.id, "1");
        assert_eq!(state.sync_request_sequence.peek_next(), 1);
    }

    async fn put_config_response(
        state: Arc<AppState>,
        config: ThoughtConfig,
        if_match: Option<u64>,
    ) -> axum::response::Response {
        let mut headers = HeaderMap::new();
        if let Some(v) = if_match {
            headers.insert(
                "if-match",
                format!("\"{v}\"").parse().expect("header value"),
            );
        }
        put_thought_config(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            headers,
            Json(config),
        )
        .await
        .into_response()
    }

    #[tokio::test]
    async fn put_thought_config_optimistic_concurrency() {
        let _guard = VERSION_TEST_LOCK.lock().await;
        THOUGHT_CONFIG_VERSION.store(0, Ordering::Release);
        let dir = tempfile::tempdir().expect("tempdir");
        let store = crate::persistence::file_store::FileStore::new(dir.path())
            .await
            .expect("file store");
        let state = test_state(Some(store));

        // (b) missing header → 200 + new version
        let response = put_config_response(state.clone(), ThoughtConfig::default(), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let v1 = json["version"].as_u64().expect("version should be present");
        assert!(v1 > 0);

        // (c) matching version → 200 + bumped version
        let response = put_config_response(state.clone(), ThoughtConfig::default(), Some(v1)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let v2 = json["version"].as_u64().expect("version should be present");
        assert_eq!(v2, v1 + 1);

        // (a) stale version → 412
        let response = put_config_response(state.clone(), ThoughtConfig::default(), Some(v1)).await;
        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VERSION_CONFLICT");
    }

    #[tokio::test]
    async fn put_thought_config_does_not_advance_version_when_disk_save_fails() {
        // Regression: previously the counter was bumped before the disk write,
        // so a failed save permanently consumed a version slot and broke the
        // next If-Match retry. The fix delays the counter bump until after
        // the save succeeds.
        use fs2::FileExt;

        let _guard = VERSION_TEST_LOCK.lock().await;
        THOUGHT_CONFIG_VERSION.store(0, Ordering::Release);
        let dir = tempfile::tempdir().expect("tempdir");
        let store = crate::persistence::file_store::FileStore::new(dir.path())
            .await
            .expect("file store");
        let state = test_state(Some(store));

        // Establish a starting version so the regression target (If-Match = v1
        // after a failed save) has a non-trivial precondition to verify.
        let response = put_config_response(state.clone(), ThoughtConfig::default(), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let v1 = json["version"].as_u64().expect("version should be present");

        // Hold the file_store cross-process lock so the next save fails fast.
        let lock_path = dir.path().join(".lock");
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .expect("open lock file");
        lock_file.lock_exclusive().expect("hold lock");

        let response = put_config_response(state.clone(), ThoughtConfig::default(), Some(v1)).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        lock_file.unlock().expect("release lock");

        // The original `If-Match: v1` must still succeed: the failed save must
        // not have silently advanced the version counter.
        let response = put_config_response(state.clone(), ThoughtConfig::default(), Some(v1)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let v2 = json["version"].as_u64().expect("version should be present");
        assert_eq!(v2, v1 + 1);
    }

    #[tokio::test]
    async fn put_thought_config_optimistic_concurrency_rejects_second_concurrent_if_match() {
        let _guard = VERSION_TEST_LOCK.lock().await;
        THOUGHT_CONFIG_VERSION.store(0, Ordering::Release);
        let dir = tempfile::tempdir().expect("tempdir");
        let store = crate::persistence::file_store::FileStore::new(dir.path())
            .await
            .expect("file store");
        let state = test_state(Some(store));

        let initial = put_config_response(state.clone(), ThoughtConfig::default(), None).await;
        assert_eq!(initial.status(), StatusCode::OK);
        let json = response_json(initial).await;
        let expected_version = json["version"].as_u64().expect("version should be present");

        let (resp_a, resp_b) = tokio::join!(
            put_config_response(
                state.clone(),
                ThoughtConfig::default(),
                Some(expected_version)
            ),
            put_config_response(state, ThoughtConfig::default(), Some(expected_version)),
        );

        let status_a = resp_a.status();
        let status_b = resp_b.status();
        assert!(status_a == StatusCode::OK || status_b == StatusCode::OK);
        assert!(
            status_a == StatusCode::PRECONDITION_FAILED
                || status_b == StatusCode::PRECONDITION_FAILED
        );

        let json_a = response_json(resp_a).await;
        let json_b = response_json(resp_b).await;
        let success_json = if status_a == StatusCode::OK {
            &json_a
        } else {
            &json_b
        };
        let conflict_json = if status_a == StatusCode::PRECONDITION_FAILED {
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
