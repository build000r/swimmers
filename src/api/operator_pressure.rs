use axum::extract::State;
use axum::routing::get;
use axum::{Extension, Json, Router};
use std::sync::Arc;

use crate::api::{remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::operator_pressure::{build_operator_pressure_response, OperatorPressureResponse};

async fn get_operator_pressure(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<OperatorPressureResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let mut sessions = state.supervisor.list_sessions().await;
    sessions.extend(remote_sessions::list_remote_sessions().await);
    Ok(Json(build_operator_pressure_response(&sessions)))
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/operator-pressure", get(get_operator_pressure))
}
