pub mod bootstrap;
pub mod sessions;

use axum::Router;
use std::sync::Arc;

use crate::config::Config;
use crate::session::supervisor::SessionSupervisor;

pub struct AppState {
    pub supervisor: Arc<SessionSupervisor>,
    pub config: Arc<Config>,
}

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(bootstrap::routes())
        .merge(sessions::routes())
}
