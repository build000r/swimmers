pub mod dirs;
pub mod health;
pub mod native;
pub mod selection;
pub mod service;
pub mod sessions;
pub mod skills;
pub mod thought_config;
pub mod web_actions;

use axum::middleware;
use axum::Router;
use chrono::{DateTime, Utc};
use std::sync::{Arc, OnceLock};
use tokio::sync::{oneshot, RwLock};

use crate::auth;
use crate::config::Config;
use crate::host_actions::RepoActionTracker;
use crate::persistence::file_store::FileStore;
use crate::session::actor::SessionCommand;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::health::BridgeHealthState;
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
    pub daemon_defaults: OnceLock<DaemonDefaults>,
    pub file_store: OnceLock<Arc<FileStore>>,
    pub bridge_health: Arc<BridgeHealthState>,
    pub published_selection: Arc<RwLock<PublishedSelectionState>>,
    pub repo_actions: RepoActionTracker,
}

impl AppState {
    pub fn current_daemon_defaults(&self) -> Option<DaemonDefaults> {
        self.daemon_defaults.get().cloned()
    }

    pub fn current_file_store(&self) -> Option<Arc<FileStore>> {
        self.file_store.get().cloned()
    }

    pub fn set_daemon_defaults(&self, defaults: DaemonDefaults) -> bool {
        self.daemon_defaults.set(defaults).is_ok()
    }

    pub fn set_file_store(&self, store: Arc<FileStore>) -> bool {
        self.file_store.set(store).is_ok()
    }
}

pub fn once_lock_with<T>(value: Option<T>) -> OnceLock<T> {
    let lock = OnceLock::new();
    if let Some(value) = value {
        let _ = lock.set(value);
    }
    lock
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

    let router = Router::new()
        .merge(native::routes())
        .merge(selection::routes())
        .merge(sessions::routes())
        .merge(thought_config::routes())
        .merge(web_actions::routes());

    #[cfg(feature = "personal-workflows")]
    let router = router.merge(dirs::routes());

    #[cfg(feature = "personal-workflows")]
    let router = router.merge(skills::routes());

    router
        .layer(middleware::from_fn(move |request, next| {
            auth::auth_middleware(config_for_middleware.clone(), request, next)
        }))
        .merge(health::health_router())
}
