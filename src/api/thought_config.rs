use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use tracing::error;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::thought::protocol::build_sync_request;
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};
use crate::types::ErrorResponse;

#[derive(serde::Serialize)]
struct ThoughtConfigResponse {
    #[serde(flatten)]
    config: ThoughtConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon_defaults: Option<DaemonDefaults>,
}

async fn get_thought_config(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ThoughtConfigResponse>, Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let config = state.thought_config.read().await.clone();
    Ok(Json(ThoughtConfigResponse {
        config,
        daemon_defaults: state.daemon_defaults.clone(),
    }))
}

async fn get_thought_sync_preview(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<clawgs::emit::protocol::SyncRequest>, Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let config = state.thought_config.read().await.clone();
    let sessions = state.supervisor.collect_session_snapshots().await;
    let request = build_sync_request(state.sync_request_sequence.peek_next(), &config, &sessions);
    Ok(Json(request))
}

async fn put_thought_config(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<ThoughtConfig>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let config = match body.normalize_and_validate() {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    code: "VALIDATION_FAILED".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response();
        }
    };

    let store = match state.file_store.as_ref() {
        Some(store) => store,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    code: "PERSISTENCE_UNAVAILABLE".to_string(),
                    message: Some("thought config persistence is unavailable".to_string()),
                }),
            )
                .into_response();
        }
    };

    if let Err(err) = store.save_thought_config(&config).await {
        error!(error = %err, "failed to persist thought runtime config");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("failed to persist thought config".to_string()),
            }),
        )
            .into_response();
    }

    {
        let mut runtime_config = state.thought_config.write().await;
        *runtime_config = config.clone();
    }

    (StatusCode::OK, Json(config)).into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v1/thought-config",
            get(get_thought_config).put(put_thought_config),
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
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use tokio::sync::mpsc;
    use tokio::sync::RwLock;

    fn test_state(
        file_store: Option<Arc<crate::persistence::file_store::FileStore>>,
    ) -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: None,
            file_store,
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
            token_count: 55,
            context_limit: 100,
            thought: Some("reviewing diff".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::Llm,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
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

    #[tokio::test]
    async fn put_thought_config_rejects_invalid_payloads() {
        let response = put_thought_config(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state(None)),
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
    async fn put_thought_config_requires_persistence_store() {
        let response = put_thought_config(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state(None)),
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
}
