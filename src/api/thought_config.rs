use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use tracing::error;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
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
    Router::new().route(
        "/v1/thought-config",
        get(get_thought_config).put(put_thought_config),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use axum::body::to_bytes;
    use axum::extract::{Json, State};
    use axum::response::IntoResponse;
    use serde_json::Value;
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
            daemon_defaults: None,
            file_store,
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
        })
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
}
