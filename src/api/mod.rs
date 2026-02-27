pub mod bootstrap;
pub mod dirs;
pub mod sessions;
pub mod skills;
pub mod thought_config;

use axum::middleware;
use axum::Router;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth;
use crate::config::Config;
use crate::persistence::file_store::FileStore;
use crate::realtime;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};

pub struct AppState {
    pub supervisor: Arc<SessionSupervisor>,
    pub config: Arc<Config>,
    pub thought_config: Arc<RwLock<ThoughtConfig>>,
    pub daemon_defaults: Option<DaemonDefaults>,
    pub file_store: Option<Arc<FileStore>>,
}

pub fn api_router(config: Arc<Config>) -> Router<Arc<AppState>> {
    let config_for_middleware = config.clone();

    Router::new()
        .merge(bootstrap::routes())
        .merge(dirs::routes())
        .merge(skills::routes())
        .merge(sessions::routes())
        .merge(thought_config::routes())
        .nest("/v1/realtime", realtime::handler::ws_router())
        .layer(middleware::from_fn(move |request, next| {
            auth::auth_middleware(config_for_middleware.clone(), request, next)
        }))
}
