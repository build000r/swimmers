use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use chrono::Utc;
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt};
use tokio::sync::oneshot;

use swimmers::api::{AppState, PublishedSelectionState};
use swimmers::host_actions::{
    inspect_git_repo, RepoActionExecutor, RestartExecutor, SystemRepoActionExecutor,
};
use swimmers::native;
use swimmers::openrouter_models::{
    cached_or_default_openrouter_candidates, refresh_openrouter_model_cache,
};
use swimmers::session::actor::SessionCommand;
use swimmers::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayServiceEntry,
};
use swimmers::thought::probe::run_thought_config_probe;
use swimmers::thought::runtime_config::ThoughtConfig;
use swimmers::thought_ui::thought_config_ui_metadata;
use swimmers::types::{
    CreateSessionResponse, DirEntry, DirListResponse, DirRepoActionResponse, GhosttyOpenMode,
    MermaidArtifactResponse, NativeDesktopApp, NativeDesktopOpenResponse,
    NativeDesktopStatusResponse, PlanFileResponse, RepoActionKind, RepoActionState,
    RepoActionStatus, SessionState, SessionSummary, SpawnTool,
};

use super::api::{ThoughtConfigTestResponse, TuiApi};
pub(crate) use swimmers::types::ThoughtConfigResponse;

pub(crate) struct InProcessApi {
    state: Arc<AppState>,
    http: reqwest::Client,
}

impl InProcessApi {
    pub(crate) fn new(state: Arc<AppState>) -> Self {
        let http = reqwest::Client::builder()
            .build()
            .expect("failed to build reqwest client for in-process API");
        Self { state, http }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// TODO(parity): extract shared service fn from src/api/mod.rs:47
async fn fetch_live_summary(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<Option<SessionSummary>, String> {
    let handle = match state.supervisor.get_session(session_id).await {
        Some(handle) => handle,
        None => return Ok(None),
    };
    let (tx, rx) = oneshot::channel();
    handle
        .send(SessionCommand::GetSummary(tx))
        .await
        .map_err(|err| format!("failed to request session summary: {err}"))?;
    let summary = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .map_err(|_| "session summary request timed out".to_string())?
        .map_err(|_| "session summary actor dropped reply".to_string())?;
    Ok(Some(summary))
}

// TODO(parity): extract shared service fn from src/api/native.rs:35
async fn native_status_for_localhost(state: &Arc<AppState>) -> NativeDesktopStatusResponse {
    let app = *state.native_desktop_app.read().await;
    let ghostty_mode = *state.ghostty_open_mode.read().await;
    let mut status = native::support_for_host("localhost", app);
    if app == NativeDesktopApp::Ghostty {
        status.ghostty_mode = Some(ghostty_mode);
    }
    status
}

// ---------------------------------------------------------------------------
// Dirs helpers — verbatim from src/api/dirs.rs
// TODO(parity): extract shared service fns from src/api/dirs.rs
// ---------------------------------------------------------------------------

const GIT_PROBE_CONCURRENCY: usize = 16; // src/api/dirs.rs:32
const HEALTH_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250); // src/api/dirs.rs:161
const HEALTH_PROBE_TOTAL_TIMEOUT: Duration = Duration::from_millis(500); // src/api/dirs.rs:162

struct DirServiceContext {
    base_path: PathBuf,
    services: Vec<OverlayServiceEntry>,
}

// src/api/dirs.rs:57
fn dirs_base_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("DIRS_BASE_PATH") {
        return PathBuf::from(explicit);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    if let Some(config) = dir_resolve_config(&cwd) {
        return config.base_path.clone();
    }
    cwd
}

// src/api/dirs.rs:71
fn dir_resolve_config(path: &Path) -> Option<&'static OverlayDirConfig> {
    let overlay = default_overlay()?;
    overlay.find_dir_config(&path.to_string_lossy())
}

// src/api/dirs.rs:80
fn managed_base_child_names(config: &OverlayDirConfig, base: &Path) -> Option<BTreeSet<String>> {
    if config.services.is_empty() {
        return None;
    }
    let resolved_base = config
        .base_path
        .canonicalize()
        .unwrap_or(config.base_path.clone());
    let canonical_base = base.canonicalize().unwrap_or(base.to_path_buf());
    let mut children = BTreeSet::new();
    for service in &config.services {
        let service_path = resolved_base.join(&service.dir);
        let Ok(canonical) = service_path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&canonical_base) {
            continue;
        }
        let Ok(relative) = canonical.strip_prefix(&canonical_base) else {
            continue;
        };
        let Some(Component::Normal(name)) = relative.components().next() else {
            continue;
        };
        children.insert(name.to_string_lossy().into_owned());
    }
    if children.is_empty() {
        None
    } else {
        Some(children)
    }
}

// src/api/dirs.rs:116
fn relative_repo_path(base: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(base).ok()?;
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    Some(components.join("/"))
}

// src/api/dirs.rs:128
fn services_for_directory(path: &Path, context: &DirServiceContext) -> Vec<String> {
    let Some(relative_path) = relative_repo_path(&context.base_path, path) else {
        return Vec::new();
    };
    if relative_path.is_empty() {
        return Vec::new();
    }
    let mut services = BTreeSet::new();
    for service in &context.services {
        if service.dir == relative_path
            || service.dir.starts_with(&format!("{relative_path}/"))
            || relative_path.starts_with(&format!("{}/", service.dir))
        {
            services.insert(service.name.clone());
        }
    }
    services.into_iter().collect()
}

// src/api/dirs.rs:164
async fn overlay_service_health_map(
    services: &[OverlayServiceEntry],
    requested: &[String],
) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    if requested.is_empty() {
        return map;
    }
    let client = reqwest::Client::builder()
        .connect_timeout(HEALTH_PROBE_CONNECT_TIMEOUT)
        .timeout(HEALTH_PROBE_TOTAL_TIMEOUT)
        .build()
        .unwrap_or_default();
    let mut handles = Vec::new();
    for service in services {
        if !requested.contains(&service.name) {
            continue;
        }
        let Some(url) = &service.health_url else {
            map.insert(service.name.clone(), true);
            continue;
        };
        let name = service.name.clone();
        let url = url.clone();
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let ok = client.get(&url).send().await.is_ok();
            (name, ok)
        }));
    }
    for handle in handles {
        if let Ok((name, ok)) = handle.await {
            map.insert(name, ok);
        }
    }
    map
}

// src/api/dirs.rs:274
fn resolve_target_path(base: PathBuf, target: PathBuf) -> Result<(PathBuf, PathBuf), String> {
    let canonical = target
        .canonicalize()
        .map_err(|_| format!("directory not found: {}", target.display()))?;
    let canonical_base = base.canonicalize().unwrap_or(base);
    if !canonical.starts_with(&canonical_base) {
        return Err("path is outside the allowed base directory".to_string());
    }
    Ok((canonical_base, canonical))
}

// src/api/dirs.rs:304
async fn list_group_entries(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let group = group.clone();
    tokio::task::spawn_blocking(move || list_group_entries_sync(&group))
        .await
        .unwrap_or_default()
}

// src/api/dirs.rs:311
fn list_group_entries_sync(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let mut seen = BTreeSet::new();
    let mut entries: Vec<(DirEntry, u64)> = Vec::new();
    for source_dir in &group.dirs {
        let Ok(read_dir) = std::fs::read_dir(source_dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            let entry_path = entry.path();
            let has_children = std::fs::read_dir(&entry_path)
                .map(|rd| {
                    rd.flatten().any(|child| {
                        child.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                            && !child.file_name().to_string_lossy().starts_with('.')
                    })
                })
                .unwrap_or(false);
            let full_path = entry_path
                .canonicalize()
                .unwrap_or(entry_path)
                .to_string_lossy()
                .into_owned();
            entries.push((
                DirEntry {
                    name,
                    has_children,
                    is_running: None,
                    repo_dirty: None,
                    repo_action: None,
                    group: None,
                    full_path: Some(full_path),
                    has_restart: None,
                    open_url: None,
                },
                0,
            ));
        }
    }
    entries.sort_by(|(a, _), (b, _)| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.into_iter().map(|(entry, _)| entry).collect()
}

// ---------------------------------------------------------------------------
// Repo-action helpers — verbatim from src/api/dirs.rs
// ---------------------------------------------------------------------------

impl InProcessApi {
    // src/api/dirs.rs:770
    async fn start_restart_action(
        &self,
        requested_path: &str,
        kind: RepoActionKind,
    ) -> Result<DirRepoActionResponse, String> {
        let requested_path = requested_path.trim();
        if requested_path.is_empty() {
            return Err("path is required".to_string());
        }
        let base = dirs_base_path();
        let target = PathBuf::from(requested_path);
        let (canonical_base, canonical) = resolve_target_path(base, target)?;

        let config = dir_resolve_config(&canonical_base)
            .ok_or_else(|| "no overlay configuration found for this path".to_string())?;

        let context = DirServiceContext {
            base_path: config.base_path.clone(),
            services: config.services.clone(),
        };
        let matched_services = services_for_directory(&canonical, &context);
        if matched_services.is_empty() {
            return Err("no overlay service is mapped to this folder".to_string());
        }

        let commands: Vec<(String, String)> = config
            .services
            .iter()
            .filter(|s| matched_services.contains(&s.name))
            .filter_map(|s| s.restart.as_ref().map(|cmd| (s.name.clone(), cmd.clone())))
            .collect();

        if commands.is_empty() {
            return Err("matched services have no restart command configured".to_string());
        }

        let executor: Arc<dyn RepoActionExecutor> = Arc::new(RestartExecutor { commands });

        self.state
            .repo_actions
            .start(canonical.clone(), kind, executor)
            .await
            .map_err(|err| err.to_string())?;

        let status = self
            .state
            .repo_actions
            .status_for(&canonical)
            .await
            .unwrap_or(RepoActionStatus {
                kind,
                state: RepoActionState::Running,
                detail: None,
            });

        Ok(DirRepoActionResponse {
            ok: true,
            path: canonical.to_string_lossy().into_owned(),
            status,
        })
    }

    // src/api/dirs.rs:858
    async fn start_repo_action_with_executor(
        &self,
        requested_path: &str,
        kind: RepoActionKind,
        executor: Arc<dyn RepoActionExecutor>,
    ) -> Result<DirRepoActionResponse, String> {
        let requested_path = requested_path.trim();
        if requested_path.is_empty() {
            return Err("path is required".to_string());
        }
        let base = dirs_base_path();
        let target = PathBuf::from(requested_path);
        let (_canonical_base, canonical) = resolve_target_path(base, target)?;

        let repo_summary = inspect_git_repo(&canonical)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| "path is not inside a git repository".to_string())?;

        if !repo_summary.dirty {
            return Err("repo has no pending changes to commit".to_string());
        }

        self.state
            .repo_actions
            .start(repo_summary.repo_root.clone(), kind, executor)
            .await
            .map_err(|err| err.to_string())?;

        let status = self
            .state
            .repo_actions
            .status_for(&repo_summary.repo_root)
            .await
            .unwrap_or(RepoActionStatus {
                kind,
                state: RepoActionState::Running,
                detail: None,
            });

        Ok(DirRepoActionResponse {
            ok: true,
            path: repo_summary.repo_root.to_string_lossy().into_owned(),
            status,
        })
    }
}

// ---------------------------------------------------------------------------
// TuiApi implementation
// ---------------------------------------------------------------------------

impl TuiApi for InProcessApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        // Mirrors: src/api/sessions.rs:23 (list_sessions)
        Box::pin(async move { Ok(self.state.supervisor.list_sessions().await) })
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        // Mirrors: src/api/thought_config.rs:19 (get_thought_config)
        Box::pin(async move {
            let config = self.state.thought_config.read().await.clone();
            Ok(ThoughtConfigResponse {
                config,
                daemon_defaults: self.state.daemon_defaults.clone(),
                ui: thought_config_ui_metadata(&cached_or_default_openrouter_candidates()),
            })
        })
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        // Mirrors: src/api/thought_config.rs:43 (put_thought_config)
        Box::pin(async move {
            let config = config
                .normalize_and_validate()
                .map_err(|err| err.to_string())?;

            let store = self
                .state
                .file_store
                .as_ref()
                .ok_or_else(|| "thought config persistence is unavailable".to_string())?;

            store.save_thought_config(&config).await.map_err(|err| {
                tracing::error!(error = %err, "failed to persist thought runtime config");
                "failed to persist thought config".to_string()
            })?;

            {
                let mut runtime_config = self.state.thought_config.write().await;
                *runtime_config = config.clone();
            }

            Ok(config)
        })
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        // Mirrors: src/api/thought_config.rs:100 (post_thought_config_test)
        Box::pin(async move {
            let config = config
                .normalize_and_validate()
                .map_err(|err| err.to_string())?;
            Ok(run_thought_config_probe(&config).await)
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        // Mirrors: src/bin/swimmers_tui/api.rs:493 (ApiClient::refresh_openrouter_candidates)
        Box::pin(async move {
            match refresh_openrouter_model_cache(&self.http).await {
                Ok(cache) if !cache.models.is_empty() => Ok(cache.models),
                Ok(_) => Ok(cached_or_default_openrouter_candidates()),
                Err(err) => Err(err),
            }
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        // Mirrors: src/api/sessions.rs:434 (get_mermaid_artifact)
        let session_id = session_id.to_string();
        Box::pin(async move {
            let handle = self
                .state
                .supervisor
                .get_session(&session_id)
                .await
                .ok_or_else(|| "session not found".to_string())?;
            let (tx, rx) = oneshot::channel();
            handle
                .send(SessionCommand::GetMermaidArtifact(tx))
                .await
                .map_err(|_| "session actor unavailable".to_string())?;
            tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .map_err(|_| "mermaid artifact request timed out".to_string())?
                .map_err(|_| "actor dropped mermaid artifact reply".to_string())
        })
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        // Mirrors: src/api/sessions.rs:502 (get_plan_file)
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
            let handle = self
                .state
                .supervisor
                .get_session(&session_id)
                .await
                .ok_or_else(|| "session not found".to_string())?;
            let (tx, rx) = oneshot::channel();
            handle
                .send(SessionCommand::GetPlanFile { name, reply: tx })
                .await
                .map_err(|_| "session actor unavailable".to_string())?;
            tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .map_err(|_| "plan file request timed out".to_string())?
                .map_err(|_| "actor dropped plan file reply".to_string())
        })
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        // Mirrors: src/api/native.rs:48 (native_status)
        Box::pin(async move { Ok(native_status_for_localhost(&self.state).await) })
    }

    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        // Mirrors: src/api/native.rs:58 (set_native_app)
        Box::pin(async move {
            {
                let mut native_app = self.state.native_desktop_app.write().await;
                *native_app = app;
            }
            Ok(native_status_for_localhost(&self.state).await)
        })
    }

    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        // Mirrors: src/api/native.rs:77 (set_native_mode)
        Box::pin(async move {
            {
                let mut ghostty_mode = self.state.ghostty_open_mode.write().await;
                *ghostty_mode = mode;
            }
            Ok(native_status_for_localhost(&self.state).await)
        })
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        // Mirrors: src/api/selection.rs:68 (publish_selection)
        let session_id = session_id.and_then(|v| {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        Box::pin(async move {
            let published_at = session_id.as_ref().map(|_| Utc::now());
            let mut selection = self.state.published_selection.write().await;
            *selection = PublishedSelectionState {
                session_id,
                published_at,
            };
            Ok(())
        })
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        // Mirrors: src/api/native.rs:96 (native_open)
        let session_id = session_id.to_string();
        Box::pin(async move {
            let app = *self.state.native_desktop_app.read().await;
            let ghostty_mode = *self.state.ghostty_open_mode.read().await;
            let status = native::support_for_host("localhost", app);
            if !status.supported {
                return Err(status
                    .reason
                    .unwrap_or_else(|| "native desktop unavailable".to_string()));
            }

            let summary = fetch_live_summary(&self.state, &session_id)
                .await?
                .ok_or_else(|| "session not found".to_string())?;

            if summary.state == SessionState::Exited {
                return Err("session has already exited".to_string());
            }

            native::open_native_session(
                app,
                ghostty_mode,
                &summary.session_id,
                &summary.tmux_name,
                &summary.cwd,
            )
            .await
            .map_err(|err| err.to_string())
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        group: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        // Mirrors: src/api/dirs.rs:380 (list_dirs)
        // TODO(parity): extract shared service fn from src/api/dirs.rs:380
        let path = path.map(|v| v.to_string());
        let group = group.map(|v| v.to_string());
        Box::pin(async move {
            let base = dirs_base_path();

            if let Some(group_name) = &group {
                let canonical_base = base.canonicalize().unwrap_or(base.clone());
                let dir_config = dir_resolve_config(&canonical_base);
                let overlay_group = dir_config
                    .and_then(|config| config.groups.iter().find(|g| &g.name == group_name));
                let Some(overlay_group) = overlay_group else {
                    return Err(format!("no group named '{group_name}' in overlay"));
                };
                let entries = list_group_entries(overlay_group).await;
                return Ok(DirListResponse {
                    path: canonical_base.to_string_lossy().into_owned(),
                    entries,
                    overlay_label: dir_config.map(|c| c.label.clone()),
                    groups: dir_config
                        .map(|c| c.groups.iter().map(|g| g.name.clone()).collect())
                        .unwrap_or_default(),
                });
            }

            let target = match &path {
                Some(p) if !p.is_empty() => PathBuf::from(p),
                _ => base.clone(),
            };

            let (canonical_base, canonical) = resolve_target_path(base, target)?;

            let read_dir = std::fs::read_dir(&canonical).map_err(|err| err.to_string())?;

            let dir_config = dir_resolve_config(&canonical_base);

            let managed_children = if managed_only && canonical == canonical_base {
                dir_config.and_then(|config| managed_base_child_names(config, &canonical_base))
            } else {
                None
            };

            let service_context = dir_config.map(|config| DirServiceContext {
                base_path: config.base_path.clone(),
                services: config.services.clone(),
            });

            struct PendingEntry {
                name: String,
                entry_path: PathBuf,
                has_children: bool,
                modified_at: u64,
                services: Vec<String>,
            }

            let mut pending: Vec<PendingEntry> = Vec::new();
            let mut unique_services: BTreeSet<String> = BTreeSet::new();
            for entry in read_dir.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if let Some(allowed) = &managed_children {
                    if !allowed.contains(&name) {
                        continue;
                    }
                }
                let entry_path = entry.path();
                let has_children = std::fs::read_dir(&entry_path)
                    .map(|rd| {
                        rd.flatten().any(|child| {
                            child.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                                && !child.file_name().to_string_lossy().starts_with('.')
                        })
                    })
                    .unwrap_or(false);
                let modified_at = entry
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0);
                let services = service_context
                    .as_ref()
                    .map(|ctx| services_for_directory(&entry_path, ctx))
                    .unwrap_or_default();
                for service in &services {
                    unique_services.insert(service.clone());
                }
                pending.push(PendingEntry {
                    name,
                    entry_path,
                    has_children,
                    modified_at,
                    services,
                });
            }

            let repo_actions = self.state.repo_actions.clone();
            let probe_inputs: Vec<(PathBuf, _)> = pending
                .iter()
                .map(|pe| (pe.entry_path.clone(), repo_actions.clone()))
                .collect();

            let probes: Vec<(Option<bool>, Option<RepoActionStatus>)> =
                stream::iter(probe_inputs)
                    .map(|(entry_path, repo_actions)| async move {
                        let repo_summary =
                            inspect_git_repo(&entry_path).await.ok().flatten().and_then(
                                |summary| {
                                    let canonical_entry =
                                        entry_path.canonicalize().unwrap_or(entry_path.clone());
                                    (summary.repo_root == canonical_entry).then_some(summary)
                                },
                            );
                        let repo_dirty = repo_summary.as_ref().map(|s| s.dirty);
                        let repo_action = match repo_summary.as_ref() {
                            Some(s) => repo_actions.status_for(&s.repo_root).await,
                            None => None,
                        };
                        (repo_dirty, repo_action)
                    })
                    .buffered(GIT_PROBE_CONCURRENCY)
                    .collect()
                    .await;

            let health_map = if let Some(config) = dir_config {
                let svc_names: Vec<String> = unique_services.into_iter().collect();
                overlay_service_health_map(&config.services, &svc_names).await
            } else {
                HashMap::new()
            };

            let svc_meta: HashMap<&str, &OverlayServiceEntry> = dir_config
                .map(|config| {
                    config
                        .services
                        .iter()
                        .map(|s| (s.name.as_str(), s))
                        .collect()
                })
                .unwrap_or_default();

            let mut entries: Vec<(DirEntry, u64)> = pending
                .into_iter()
                .zip(probes)
                .map(|(pe, (repo_dirty, repo_action))| {
                    let is_running = if pe.services.is_empty() {
                        None
                    } else {
                        Some(
                            pe.services
                                .iter()
                                .any(|svc| health_map.get(svc).copied().unwrap_or(false)),
                        )
                    };
                    let has_restart = pe
                        .services
                        .iter()
                        .any(|svc| {
                            svc_meta
                                .get(svc.as_str())
                                .and_then(|e| e.restart.as_ref())
                                .is_some()
                        })
                        .then_some(true);
                    let open_url = pe.services.iter().find_map(|svc| {
                        svc_meta
                            .get(svc.as_str())
                            .and_then(|e| e.open_url.clone().or_else(|| e.health_url.clone()))
                    });
                    (
                        DirEntry {
                            name: pe.name,
                            has_children: pe.has_children,
                            is_running,
                            repo_dirty,
                            repo_action,
                            group: None,
                            full_path: None,
                            has_restart,
                            open_url,
                        },
                        pe.modified_at,
                    )
                })
                .collect();

            entries.sort_by(|(a, a_mod), (b, b_mod)| {
                b_mod
                    .cmp(a_mod)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            let entries: Vec<DirEntry> = entries.into_iter().map(|(e, _)| e).collect();

            let groups = dir_config
                .map(|config| {
                    config
                        .groups
                        .iter()
                        .map(|g| g.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            Ok(DirListResponse {
                path: canonical.to_string_lossy().into_owned(),
                entries,
                overlay_label: dir_config.map(|c| c.label.clone()),
                groups,
            })
        })
    }

    fn start_repo_action(
        &self,
        path: &str,
        kind: RepoActionKind,
    ) -> BoxFuture<'_, Result<DirRepoActionResponse, String>> {
        // Mirrors: src/api/dirs.rs:745 (start_dir_repo_action)
        // TODO(parity): extract shared service fn from src/api/dirs.rs:745
        let path = path.to_string();
        Box::pin(async move {
            match kind {
                RepoActionKind::Restart => self.start_restart_action(&path, kind).await,
                RepoActionKind::Open => Err("open actions are handled client-side".to_string()),
                _ => {
                    self.start_repo_action_with_executor(
                        &path,
                        kind,
                        Arc::new(SystemRepoActionExecutor),
                    )
                    .await
                }
            }
        })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        // Mirrors: src/api/sessions.rs:46 (create_session)
        let cwd = cwd.to_string();
        Box::pin(async move {
            let (session, repo_theme) = self
                .state
                .supervisor
                .create_session(None, Some(cwd), Some(spawn_tool), initial_request)
                .await
                .map_err(|err| err.to_string())?;
            Ok(CreateSessionResponse {
                session,
                repo_theme,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swimmers::config::Config;
    use swimmers::session::supervisor::SessionSupervisor;
    use swimmers::thought::protocol::SyncRequestSequence;
    use tokio::sync::RwLock;

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: None,
            file_store: None,
            bridge_health: Arc::new(swimmers::thought::health::BridgeHealthState::new_with_tick(
                Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: swimmers::host_actions::RepoActionTracker::default(),
        })
    }

    #[tokio::test]
    async fn fetch_sessions_returns_empty_list() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_sessions().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_native_status_returns_ok() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_native_status().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn publish_selection_round_trip() {
        let state = test_state();
        let published = state.published_selection.clone();
        let api = InProcessApi::new(state);

        let result = api.publish_selection(Some("test-session")).await;
        assert!(result.is_ok());
        {
            let sel = published.read().await;
            assert_eq!(sel.session_id.as_deref(), Some("test-session"));
            assert!(sel.published_at.is_some());
        }

        let result = api.publish_selection(None).await;
        assert!(result.is_ok());
        {
            let sel = published.read().await;
            assert!(sel.session_id.is_none());
            assert!(sel.published_at.is_none());
        }
    }

    #[tokio::test]
    async fn fetch_thought_config_returns_defaults() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_thought_config().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.daemon_defaults.is_none());
    }
}
