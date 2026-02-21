pub mod bootstrap;
pub mod dirs;
pub mod sessions;
pub mod skills;

use axum::middleware;
use axum::Router;
use std::sync::Arc;

use crate::auth;
use crate::config::Config;
use crate::realtime;
use crate::session::supervisor::SessionSupervisor;

pub struct AppState {
    pub supervisor: Arc<SessionSupervisor>,
    pub config: Arc<Config>,
}

pub fn api_router(config: Arc<Config>) -> Router<Arc<AppState>> {
    let config_for_middleware = config.clone();

    Router::new()
        .merge(bootstrap::routes())
        .merge(dirs::routes())
        .merge(skills::routes())
        .merge(sessions::routes())
        .nest("/v1/realtime", realtime::handler::ws_router())
        .layer(middleware::from_fn(move |request, next| {
            auth::auth_middleware(config_for_middleware.clone(), request, next)
        }))
}
