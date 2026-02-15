pub mod bootstrap;
pub mod sessions;

use axum::middleware;
use axum::Router;
use std::sync::Arc;

use crate::auth;
use crate::config::Config;
use crate::session::supervisor::SessionSupervisor;

pub struct AppState {
    pub supervisor: Arc<SessionSupervisor>,
    pub config: Arc<Config>,
}

pub fn api_router(config: Arc<Config>) -> Router<Arc<AppState>> {
    let config_for_middleware = config.clone();

    Router::new()
        .merge(bootstrap::routes())
        .merge(sessions::routes())
        .layer(middleware::from_fn(move |request, next| {
            auth::auth_middleware(config_for_middleware.clone(), request, next)
        }))
}
