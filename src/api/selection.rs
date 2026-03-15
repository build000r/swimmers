use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use chrono::Utc;
use std::sync::Arc;

use crate::api::{fetch_live_summary, AppState, PublishedSelectionState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    ErrorResponse, PublishSelectionRequest, PublishedSelectionResponse, SessionState,
};

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
    Json(body): Json<PublishSelectionRequest>,
) -> Result<Json<serde_json::Value>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsWrite)?;

    let session_id = body.session_id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let published_at = session_id.as_ref().map(|_| Utc::now());
    let mut selection = state.published_selection.write().await;
    *selection = PublishedSelectionState {
        session_id,
        published_at,
    };

    Ok(Json(serde_json::json!({ "ok": true })))
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/v1/selection",
        get(get_published_selection).put(publish_selection),
    )
}
