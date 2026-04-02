pub mod dirs;
pub mod native;
pub mod selection;
pub mod sessions;
pub mod web_actions;
pub mod skills;
pub mod thought_config;

use axum::middleware;
use axum::Router;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

use crate::auth;
use crate::config::Config;
use crate::persistence::file_store::FileStore;
use crate::session::actor::SessionCommand;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::protocol::SyncRequestSequence;
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};
use crate::types::{GhosttyOpenMode, NativeDesktopApp, SessionSummary};

#[derive(Debug, Clone, Default)]
pub struct PublishedSelectionState {
    pub session_id: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

pub struct AppState {
    pub supervisor: Arc<SessionSupervisor>,
    pub config: Arc<Config>,
    pub thought_config: Arc<RwLock<ThoughtConfig>>,
    pub native_desktop_app: Arc<RwLock<NativeDesktopApp>>,
    pub ghostty_open_mode: Arc<RwLock<GhosttyOpenMode>>,
    pub sync_request_sequence: Arc<SyncRequestSequence>,
    pub daemon_defaults: Option<DaemonDefaults>,
    pub file_store: Option<Arc<FileStore>>,
    pub published_selection: Arc<RwLock<PublishedSelectionState>>,
}

pub(crate) async fn fetch_live_summary(
    state: &Arc<AppState>,
    session_id: &str,
) -> anyhow::Result<Option<SessionSummary>> {
    let handle = match state.supervisor.get_session(session_id).await {
        Some(handle) => handle,
        None => return Ok(None),
    };

    let (tx, rx) = oneshot::channel();
    handle
        .send(SessionCommand::GetSummary(tx))
        .await
        .map_err(|err| anyhow::anyhow!("failed to request session summary: {err}"))?;

    let summary = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
        .await
        .map_err(|_| anyhow::anyhow!("session summary request timed out"))?
        .map_err(|_| anyhow::anyhow!("session summary actor dropped reply"))?;

    Ok(Some(summary))
}

pub fn api_router(config: Arc<Config>) -> Router<Arc<AppState>> {
    let config_for_middleware = config.clone();

    Router::new()
        .merge(dirs::routes())
        .merge(native::routes())
        .merge(selection::routes())
        .merge(skills::routes())
        .merge(sessions::routes())
        .merge(thought_config::routes())
        .merge(web_actions::routes())
        .layer(middleware::from_fn(move |request, next| {
            auth::auth_middleware(config_for_middleware.clone(), request, next)
        }))
}
