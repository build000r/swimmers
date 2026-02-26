use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use tracing::error;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::ErrorResponse;

async fn get_thought_config(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ThoughtConfig>, Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let config = state.thought_config.read().await.clone();
    Ok(Json(config))
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
