use axum::extract::State;
use axum::routing::get;
use axum::{Extension, Json, Router};
use std::sync::Arc;

use crate::api::service::list_sessions_for_client;
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::operator_pressure::{build_operator_pressure_response, OperatorPressureResponse};

async fn get_operator_pressure(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<OperatorPressureResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let sessions = list_sessions_for_client(&state, true).await;
    Ok(Json(build_operator_pressure_response(&sessions)))
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/operator-pressure", get(get_operator_pressure))
}
