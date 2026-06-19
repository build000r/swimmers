use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::http::StatusCode;

use super::ApiServiceError;
use crate::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayLaunchConfig, OverlayServiceEntry,
};
use crate::types::{DirEntry, DirGroupMembershipDelta, DirGroupMemberships};

const BV_WORKSPACES_DIR: &str = "workspaces";

pub struct OverlayServiceContext {
    pub base_path: PathBuf,
    pub services: Vec<OverlayServiceEntry>,
}

#[derive(serde::Deserialize)]
struct BvWorkspaceFile {
    #[serde(default)]
    repos: Vec<BvWorkspaceRepo>,
}

#[derive(serde::Deserialize)]
struct BvWorkspaceRepo {
    #[serde(default)]
    path: Option<String>,
}

/// Resolve the overlay dir config for the given path.
pub fn resolve_dir_config(path: &Path) -> Option<&'static OverlayDirConfig> {
    let overlay = default_overlay()?;
    overlay.find_dir_config(&path.to_string_lossy())
}

pub(super) fn effective_dir_config_for_base(base: &Path) -> Option<OverlayDirConfig> {
    let workspace_groups = workspace_dir_groups_for_base(base);
    if let Some(mut config) = resolve_dir_config(base).cloned() {
        if !workspace_groups.is_empty() {
            let overlay_groups = std::mem::take(&mut config.groups);
            config.groups = workspace_groups;
            merge_dir_groups(&mut config.groups, overlay_groups);
        }
        return Some(config);
    }

    (!workspace_groups.is_empty()).then(|| OverlayDirConfig {
        label: "workspaces".to_string(),
        base_path: base.to_path_buf(),
        services: Vec::new(),
        groups: workspace_groups,
        launch: OverlayLaunchConfig::local_only(),
    })
}

fn workspace_dir_groups_for_base(base: &Path) -> Vec<OverlayDirGroup> {
    let mut groups = Vec::new();
    for root in workspace_roots_for_base(base) {
        merge_dir_groups(&mut groups, workspace_yaml_groups(base, &root));
        merge_dir_groups(&mut groups, plain_workspace_folder_groups(&root));
    }
    groups
}

fn workspace_roots_for_base(base: &Path) -> Vec<PathBuf> {
    let candidates = [
        base.join(".bv").join(BV_WORKSPACES_DIR),
        base.join(BV_WORKSPACES_DIR),
    ];
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let canonical = path.canonicalize().unwrap_or(path);
            seen.insert(canonical.clone()).then_some(canonical)
        })
        .collect()
}

fn workspace_yaml_groups(base: &Path, root: &Path) -> Vec<OverlayDirGroup> {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut groups = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_yaml_path(&path) {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
        else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(workspace) = serde_yaml::from_str::<BvWorkspaceFile>(&content) else {
            continue;
        };
        let paths = workspace
            .repos
            .into_iter()
            .filter_map(|repo| repo.path)
            .map(|path| expand_workspace_repo_path(base, &path))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        if let Some(group) = dir_group_from_exact_paths(name, paths) {
            groups.push(group);
        }
    }
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    groups
}

fn plain_workspace_folder_groups(root: &Path) -> Vec<OverlayDirGroup> {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut groups = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
        else {
            continue;
        };
        let paths = workspace_folder_entry_paths(&path);
        if let Some(group) = dir_group_from_exact_paths(name, paths) {
            groups.push(group);
        }
    }
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    groups
}

fn workspace_folder_entry_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    read_dir
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy();
            (!name.starts_with('.') && path.is_dir()).then_some(path)
        })
        .collect()
}

fn dir_group_from_exact_paths(name: String, paths: Vec<PathBuf>) -> Option<OverlayDirGroup> {
    let mut seen = BTreeSet::new();
    let paths = paths
        .into_iter()
        .filter_map(|path| {
            let canonical = path.canonicalize().unwrap_or(path);
            seen.insert(canonical.clone()).then_some(canonical)
        })
        .collect::<Vec<_>>();
    (!paths.is_empty()).then_some(OverlayDirGroup {
        name,
        paths,
        dirs: Vec::new(),
    })
}

fn merge_dir_groups(target: &mut Vec<OverlayDirGroup>, additions: Vec<OverlayDirGroup>) {
    for mut group in additions {
        if let Some(existing) = target
            .iter_mut()
            .find(|existing| existing.name == group.name)
        {
            extend_unique_paths(&mut existing.paths, std::mem::take(&mut group.paths));
            extend_unique_paths(&mut existing.dirs, std::mem::take(&mut group.dirs));
        } else {
            target.push(group);
        }
    }
}

fn extend_unique_paths(target: &mut Vec<PathBuf>, additions: Vec<PathBuf>) {
    let mut seen = target
        .iter()
        .map(|path| unique_path_key(path))
        .collect::<BTreeSet<_>>();
    target.extend(additions.into_iter().filter_map(|path| {
        path_key_if_unique(&seen, &path).map(|key| {
            seen.insert(key);
            path
        })
    }));
}

fn path_key_if_unique(seen: &BTreeSet<PathBuf>, path: &Path) -> Option<PathBuf> {
    let key = unique_path_key(path);
    (!seen.contains(&key)).then_some(key)
}

fn unique_path_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn expand_workspace_repo_path(base: &Path, raw: &str) -> PathBuf {
    let expanded = expand_workspace_path(raw.trim());
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn expand_workspace_path(raw: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return raw.to_string();
    };
    if raw == "~" {
        return home.to_string_lossy().into_owned();
    }
    raw.strip_prefix("~/")
        .map(|suffix| home.join(suffix).to_string_lossy().into_owned())
        .unwrap_or_else(|| raw.to_string())
}

fn is_yaml_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension, "yaml" | "yml"))
        .unwrap_or(false)
}

/// Compute which top-level children of `base` are "managed" by the overlay.
pub fn managed_base_child_names(
    config: &OverlayDirConfig,
    base: &Path,
) -> Option<BTreeSet<String>> {
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
        let service_path = service_dir_path(&resolved_base, &service.dir);
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

pub(super) fn service_dir_path(base: &Path, dir: &str) -> PathBuf {
    let path = PathBuf::from(dir);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

pub fn services_for_directory(path: &Path, context: &OverlayServiceContext) -> Vec<String> {
    let target = canonical_path(path);
    let canonical_base = canonical_path(&context.base_path);
    if target == canonical_base {
        return Vec::new();
    };

    context
        .services
        .iter()
        .filter(|service| service_matches_target_directory(service, &canonical_base, &target))
        .map(|service| service.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn service_matches_target_directory(
    service: &OverlayServiceEntry,
    canonical_base: &Path,
    target: &Path,
) -> bool {
    let service_path = service_dir_path(canonical_base, &service.dir);
    let canonical_service = canonical_path(&service_path);
    paths_overlap(&canonical_service, target)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

pub fn resolve_target_path(
    base: PathBuf,
    target: PathBuf,
) -> Result<(PathBuf, PathBuf), ApiServiceError> {
    resolve_target_path_with_group_roots(base, target, None)
}

pub fn resolve_target_path_with_group_roots(
    base: PathBuf,
    target: PathBuf,
    config: Option<&OverlayDirConfig>,
) -> Result<(PathBuf, PathBuf), ApiServiceError> {
    let canonical = target.canonicalize().map_err(|_| {
        ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            format!("directory not found: {}", target.display()),
        )
    })?;

    let canonical_base = base.canonicalize().unwrap_or(base);
    if !target_path_allowed(&canonical_base, &canonical, config) {
        return Err(ApiServiceError::new(
            StatusCode::FORBIDDEN,
            "DIR_OUTSIDE_BASE",
            "path is outside the allowed base directory",
        ));
    }

    Ok((canonical_base, canonical))
}

fn target_path_allowed(
    canonical_base: &Path,
    canonical: &Path,
    config: Option<&OverlayDirConfig>,
) -> bool {
    canonical.starts_with(canonical_base)
        || config
            .map(|config| {
                config
                    .groups
                    .iter()
                    .any(|group| overlay_group_allows_navigation_path(group, canonical))
            })
            .unwrap_or(false)
}

fn overlay_group_allows_navigation_path(group: &OverlayDirGroup, canonical: &Path) -> bool {
    group
        .paths
        .iter()
        .map(|path| canonical_path(path))
        .any(|path| canonical.starts_with(path))
        || group
            .dirs
            .iter()
            .map(|dir| canonical_path(dir))
            .any(|dir| canonical.starts_with(dir))
}

/// List entries from a virtual directory group, combining children from all
/// source directories. Each entry carries its full absolute path since entries
/// come from multiple distinct parents.
pub async fn list_group_entries(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let group = group.clone();
    tokio::task::spawn_blocking(move || list_group_entries_sync(&group))
        .await
        .unwrap_or_default()
}

pub fn list_group_entries_sync(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let mut seen_paths = BTreeSet::new();
    let mut entries: Vec<(DirEntry, u64)> = Vec::new();

    for entry_path in &group.paths {
        if let Some(entry) = group_path_entry(entry_path, &mut seen_paths) {
            entries.push(entry);
        }
    }

    for source_dir in &group.dirs {
        append_source_dir_entries(source_dir, &mut seen_paths, &mut entries);
    }

    sort_group_entries(&mut entries);
    entries.into_iter().map(|(entry, _)| entry).collect()
}

fn group_path_entry(
    entry_path: &Path,
    seen_paths: &mut BTreeSet<String>,
) -> Option<(DirEntry, u64)> {
    let name = visible_file_name(entry_path)?;
    let full_path = canonical_path_string(entry_path);
    seen_paths.insert(full_path.clone()).then(|| {
        (
            group_dir_entry(name, false, Some(full_path)),
            modified_secs(entry_path),
        )
    })
}

fn append_source_dir_entries(
    source_dir: &Path,
    seen_paths: &mut BTreeSet<String>,
    entries: &mut Vec<(DirEntry, u64)>,
) {
    let Ok(read_dir) = std::fs::read_dir(source_dir) else {
        return;
    };

    entries.extend(
        read_dir
            .flatten()
            .filter_map(|entry| source_dir_child_entry(entry, seen_paths)),
    );
}

fn source_dir_child_entry(
    entry: std::fs::DirEntry,
    seen_paths: &mut BTreeSet<String>,
) -> Option<(DirEntry, u64)> {
    if !entry.file_type().ok()?.is_dir() {
        return None;
    }

    let name = entry.file_name().to_string_lossy().into_owned();
    if name.starts_with('.') {
        return None;
    }

    let entry_path = entry.path();
    let full_path = canonical_path_string(&entry_path);
    if !seen_paths.insert(full_path.clone()) {
        return None;
    }

    Some((
        group_dir_entry(name, has_visible_child_dirs(&entry_path), Some(full_path)),
        dir_entry_modified_secs(&entry),
    ))
}

fn visible_file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
}

fn group_dir_entry(name: String, has_children: bool, full_path: Option<String>) -> DirEntry {
    DirEntry {
        name,
        has_children,
        is_running: None,
        repo_dirty: None,
        repo_action: None,
        group: None,
        groups: Vec::new(),
        full_path,
        has_restart: None,
        open_url: None,
    }
}

fn dir_entry_modified_secs(entry: &std::fs::DirEntry) -> u64 {
    entry
        .metadata()
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn sort_group_entries(entries: &mut [(DirEntry, u64)]) {
    entries.sort_by(|(left, _), (right, _)| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.full_path.cmp(&right.full_path))
    });
}

pub fn list_effective_group_entries_sync(
    group: &OverlayDirGroup,
    memberships: &DirGroupMemberships,
) -> Vec<DirEntry> {
    let Some(delta) = memberships.groups.get(&group.name) else {
        return list_group_entries_sync(group);
    };

    let mut seen_paths = BTreeSet::new();
    let mut entries = effective_entries_after_excludes(group, delta, &mut seen_paths);
    append_included_delta_entries(delta, &mut seen_paths, &mut entries);

    sort_group_entries(&mut entries);
    entries.into_iter().map(|(entry, _)| entry).collect()
}

fn effective_entries_after_excludes(
    group: &OverlayDirGroup,
    delta: &DirGroupMembershipDelta,
    seen_paths: &mut BTreeSet<String>,
) -> Vec<(DirEntry, u64)> {
    list_group_entries_sync(group)
        .into_iter()
        .filter(|entry| !entry_is_excluded(entry, delta))
        .map(|entry| existing_effective_entry(entry, seen_paths))
        .collect()
}

fn entry_is_excluded(entry: &DirEntry, delta: &DirGroupMembershipDelta) -> bool {
    entry
        .full_path
        .as_deref()
        .map(|path| delta.exclude_paths.contains(path))
        .unwrap_or(false)
}

fn existing_effective_entry(entry: DirEntry, seen_paths: &mut BTreeSet<String>) -> (DirEntry, u64) {
    let modified_at = entry
        .full_path
        .as_deref()
        .map(|path| modified_secs(Path::new(path)))
        .unwrap_or(0);
    if let Some(path) = entry.full_path.as_deref() {
        seen_paths.insert(path.to_string());
    }
    (entry, modified_at)
}

fn append_included_delta_entries(
    delta: &DirGroupMembershipDelta,
    seen_paths: &mut BTreeSet<String>,
    entries: &mut Vec<(DirEntry, u64)>,
) {
    entries.extend(
        delta
            .include_paths
            .iter()
            .filter_map(|raw_path| included_delta_entry(raw_path, delta, seen_paths)),
    );
}

fn included_delta_entry(
    raw_path: &str,
    delta: &DirGroupMembershipDelta,
    seen_paths: &mut BTreeSet<String>,
) -> Option<(DirEntry, u64)> {
    if delta.exclude_paths.contains(raw_path) {
        return None;
    }

    included_path_entry(&PathBuf::from(raw_path), seen_paths)
}

fn included_path_entry(path: &Path, seen_paths: &mut BTreeSet<String>) -> Option<(DirEntry, u64)> {
    if !path.is_dir() {
        return None;
    }

    let name = visible_file_name(path)?;
    let full_path = canonical_path_string(path);
    seen_paths.insert(full_path.clone()).then(|| {
        (
            group_dir_entry(name, false, Some(full_path)),
            modified_secs(path),
        )
    })
}

pub(super) async fn list_effective_group_entries(
    group: &OverlayDirGroup,
    memberships: &DirGroupMemberships,
) -> Vec<DirEntry> {
    let group = group.clone();
    let memberships = memberships.clone();
    tokio::task::spawn_blocking(move || list_effective_group_entries_sync(&group, &memberships))
        .await
        .unwrap_or_default()
}

pub(super) fn canonical_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn overlay_group_contains_path(group: &OverlayDirGroup, canonical_path: &Path) -> bool {
    if group
        .paths
        .iter()
        .any(|path| path.canonicalize().unwrap_or_else(|_| path.clone()) == canonical_path)
    {
        return true;
    }

    let Some(parent) = canonical_path.parent() else {
        return false;
    };
    group
        .dirs
        .iter()
        .any(|dir| dir.canonicalize().unwrap_or_else(|_| dir.clone()) == parent)
}

pub(super) fn effective_groups_for_path(
    config: &OverlayDirConfig,
    memberships: &DirGroupMemberships,
    path: &Path,
) -> Vec<String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized = canonical.to_string_lossy();
    let mut groups = Vec::new();
    for group in &config.groups {
        let delta = memberships.groups.get(&group.name);
        if delta
            .map(|delta| delta.exclude_paths.contains(normalized.as_ref()))
            .unwrap_or(false)
        {
            continue;
        }
        let overlay_member = overlay_group_contains_path(group, &canonical);
        let user_member = delta
            .map(|delta| delta.include_paths.contains(normalized.as_ref()))
            .unwrap_or(false);
        if overlay_member || user_member {
            groups.push(group.name.clone());
        }
    }
    groups
}

pub(super) fn annotate_dir_entry_groups(
    entries: &mut [DirEntry],
    parent: &Path,
    config: &OverlayDirConfig,
    memberships: &DirGroupMemberships,
) {
    for entry in entries {
        let path = entry
            .full_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| parent.join(&entry.name));
        entry.groups = effective_groups_for_path(config, memberships, &path);
    }
}

pub(super) fn has_visible_child_dirs(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|read_dir| {
            read_dir.flatten().any(|child| {
                child.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                    && !child.file_name().to_string_lossy().starts_with('.')
            })
        })
        .unwrap_or(false)
}

pub(super) fn modified_secs(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app_state() -> std::sync::Arc<crate::api::AppState> {
        let config = std::sync::Arc::new(crate::config::Config::default());
        let supervisor = crate::session::supervisor::SessionSupervisor::new(config.clone());
        std::sync::Arc::new(crate::api::AppState {
            supervisor,
            config,
            thought_config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::thought::runtime_config::ThoughtConfig::default(),
            )),
            native_desktop_app: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::types::NativeDesktopApp::Iterm,
            )),
            ghostty_open_mode: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::types::GhosttyOpenMode::Swap,
            )),
            sync_request_sequence: std::sync::Arc::new(
                crate::thought::protocol::SyncRequestSequence::new(),
            ),
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: std::sync::Arc::new(
                crate::thought::health::BridgeHealthState::new_with_tick(
                    std::time::Duration::from_secs(15),
                ),
            ),
            published_selection: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::api::PublishedSelectionState::default(),
            )),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    fn write_workspace_yaml(path: &Path, repo: &Path) {
        std::fs::write(
            path,
            format!(
                "---\nrepos:\n- name: swimmers\n  path: \"{}\"\n  prefix: swimmers\n",
                repo.to_string_lossy()
            ),
        )
        .expect("write workspace yaml");
    }

    fn service_entry(name: &str, dir: impl Into<String>) -> OverlayServiceEntry {
        OverlayServiceEntry {
            name: name.into(),
            dir: dir.into(),
            health_url: None,
            restart: None,
            open_url: None,
        }
    }

    #[test]
    fn extend_unique_paths_skips_existing_target_duplicate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing = dir.path().join("existing");
        let addition = dir.path().join("addition");
        std::fs::create_dir_all(&existing).expect("existing");
        std::fs::create_dir_all(&addition).expect("addition");

        let mut target = vec![existing.clone()];
        extend_unique_paths(&mut target, vec![existing.clone(), addition.clone()]);

        assert_eq!(target, vec![existing, addition]);
    }

    #[test]
    fn extend_unique_paths_skips_duplicate_additions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(&first).expect("first");
        std::fs::create_dir_all(&second).expect("second");

        let mut target = Vec::new();
        extend_unique_paths(
            &mut target,
            vec![first.clone(), first.clone(), second.clone(), second.clone()],
        );

        assert_eq!(target, vec![first, second]);
    }

    #[test]
    fn extend_unique_paths_preserves_unique_append_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing = dir.path().join("existing");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let third = dir.path().join("third");
        for path in [&existing, &first, &second, &third] {
            std::fs::create_dir_all(path).expect("test path");
        }

        let mut target = vec![existing.clone()];
        extend_unique_paths(
            &mut target,
            vec![
                first.clone(),
                existing.clone(),
                second.clone(),
                first.clone(),
                third.clone(),
            ],
        );

        assert_eq!(target, vec![existing, first, second, third]);
    }

    #[test]
    fn merge_dir_groups_extends_unique_paths_and_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing_path = dir.path().join("existing-path");
        let added_path = dir.path().join("added-path");
        let existing_dir = dir.path().join("existing-dir");
        let added_dir = dir.path().join("added-dir");
        for path in [&existing_path, &added_path, &existing_dir, &added_dir] {
            std::fs::create_dir_all(path).expect("test path");
        }

        let mut target = vec![OverlayDirGroup {
            name: "workspace".into(),
            paths: vec![existing_path.clone()],
            dirs: vec![existing_dir.clone()],
        }];
        merge_dir_groups(
            &mut target,
            vec![OverlayDirGroup {
                name: "workspace".into(),
                paths: vec![
                    existing_path.clone(),
                    added_path.clone(),
                    added_path.clone(),
                ],
                dirs: vec![existing_dir.clone(), added_dir.clone(), added_dir.clone()],
            }],
        );

        assert_eq!(target.len(), 1);
        assert_eq!(target[0].paths, vec![existing_path, added_path]);
        assert_eq!(target[0].dirs, vec![existing_dir, added_dir]);
    }

    #[test]
    fn list_group_entries_keeps_distinct_paths_with_duplicate_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exact = dir.path().join("shared");
        let source = dir.path().join("source");
        let duplicate = source.join("shared");
        let unique = source.join("unique");
        std::fs::create_dir_all(&exact).expect("exact");
        std::fs::create_dir_all(&duplicate).expect("duplicate");
        std::fs::create_dir_all(&unique).expect("unique");

        let group = OverlayDirGroup {
            name: "workspace".into(),
            paths: vec![exact.clone(), exact.clone()],
            dirs: vec![source],
        };

        let entries = list_group_entries_sync(&group);
        let shared = entries
            .iter()
            .filter(|entry| entry.name == "shared")
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 3);
        assert_eq!(shared.len(), 2);
        assert!(shared.iter().all(|entry| !entry.has_children));
        assert_eq!(
            shared[0].full_path.as_deref(),
            Some(
                exact
                    .canonicalize()
                    .expect("canonical exact")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            shared[1].full_path.as_deref(),
            Some(
                duplicate
                    .canonicalize()
                    .expect("canonical duplicate")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["shared", "shared", "unique"]
        );
    }

    #[test]
    fn list_group_entries_skips_hidden_and_non_dir_children() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let visible = source.join("visible");
        let hidden = source.join(".hidden");
        let visible_child = visible.join("child");
        std::fs::create_dir_all(&visible_child).expect("visible child");
        std::fs::create_dir_all(&hidden).expect("hidden");
        std::fs::write(source.join("file.txt"), "not a directory").expect("file");

        let group = OverlayDirGroup {
            name: "workspace".into(),
            paths: vec![dir.path().join(".exact-hidden")],
            dirs: vec![source],
        };

        let entries = list_group_entries_sync(&group);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "visible");
        assert!(entries[0].has_children);
    }

    #[test]
    fn list_group_entries_has_children_ignores_hidden_dirs_and_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let parent = source.join("parent");
        std::fs::create_dir_all(parent.join(".hidden-child")).expect("hidden child");
        std::fs::write(parent.join("file.txt"), "not a directory").expect("file");

        let group = OverlayDirGroup {
            name: "workspace".into(),
            paths: Vec::new(),
            dirs: vec![source],
        };

        let entries = list_group_entries_sync(&group);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "parent");
        assert!(!entries[0].has_children);
    }

    #[test]
    fn services_for_directory_matches_relative_absolute_and_nested_targets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let service_parent = base.join("services");
        let relative_service = service_parent.join("api");
        let relative_child = relative_service.join("src");
        let absolute_service = dir.path().join("outside").join("worker");
        std::fs::create_dir_all(&relative_child).expect("relative child");
        std::fs::create_dir_all(&absolute_service).expect("absolute service");

        let context = OverlayServiceContext {
            base_path: base.clone(),
            services: vec![
                service_entry("svc-api", "services/api"),
                service_entry("svc-worker", absolute_service.to_string_lossy()),
            ],
        };

        assert_eq!(
            services_for_directory(&base, &context),
            Vec::<String>::new()
        );
        assert_eq!(
            services_for_directory(&service_parent, &context),
            vec!["svc-api"]
        );
        assert_eq!(
            services_for_directory(&relative_service, &context),
            vec!["svc-api"]
        );
        assert_eq!(
            services_for_directory(&relative_child, &context),
            vec!["svc-api"]
        );
        assert_eq!(
            services_for_directory(&absolute_service, &context),
            vec!["svc-worker"]
        );
    }

    #[test]
    fn bv_workspace_yaml_files_become_exact_groups_and_skip_invalid_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = base.join("opensource").join("swimmers");
        let workspaces = base.join(".bv").join(BV_WORKSPACES_DIR);
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::create_dir_all(&workspaces).expect("workspaces");
        write_workspace_yaml(&workspaces.join("orchestration.yaml"), &repo);
        std::fs::write(workspaces.join("broken.yaml"), "repos: [").expect("broken yaml");

        let groups = workspace_dir_groups_for_base(&base);
        let names = groups
            .iter()
            .map(|group| group.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["orchestration"]);

        let entries = list_group_entries_sync(
            groups
                .iter()
                .find(|group| group.name == "orchestration")
                .expect("orchestration group"),
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "swimmers");
        assert_eq!(
            entries[0].full_path.as_deref(),
            Some(
                repo.canonicalize()
                    .expect("canonical repo")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn bv_workspace_root_may_be_symlinked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = base.join("opensource").join("skillbox");
        let target = dir.path().join("shared-workspaces");
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::create_dir_all(&target).expect("target workspaces");
        std::fs::create_dir_all(base.join(".bv")).expect("bv dir");
        write_workspace_yaml(&target.join("skillbox.yaml"), &repo);
        std::os::unix::fs::symlink(&target, base.join(".bv").join(BV_WORKSPACES_DIR))
            .expect("workspace symlink");

        let groups = workspace_dir_groups_for_base(&base);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "skillbox");
        assert_eq!(
            groups[0].paths,
            vec![repo.canonicalize().expect("canonical repo")]
        );
    }

    #[test]
    fn plain_workspaces_folder_subdirs_become_groups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = base.join(BV_WORKSPACES_DIR).join("mobile").join("ios-app");
        std::fs::create_dir_all(&repo).expect("repo");

        let groups = workspace_dir_groups_for_base(&base);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "mobile");

        let entries = list_group_entries_sync(&groups[0]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "ios-app");
        assert!(!entries[0].has_children);
        assert_eq!(
            entries[0].full_path.as_deref(),
            Some(
                repo.canonicalize()
                    .expect("canonical repo")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    #[tokio::test]
    async fn list_group_dir_response_uses_bv_workspace_groups_without_overlay() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = dir.path().join("outside").join("swimmers");
        let workspaces = base.join(".bv").join(BV_WORKSPACES_DIR);
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::create_dir_all(&workspaces).expect("workspaces");
        write_workspace_yaml(&workspaces.join("orchestration.yaml"), &repo);

        let response = super::super::list_group_dir_response(
            &test_app_state(),
            base.clone(),
            "orchestration",
            &DirGroupMemberships::default(),
        )
        .await
        .expect("workspace group response");

        assert_eq!(response.groups, vec!["orchestration".to_string()]);
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].name, "swimmers");
        assert_eq!(
            response.entries[0].groups,
            vec!["orchestration".to_string()]
        );
    }

    #[test]
    fn workspace_group_paths_are_allowed_for_membership_updates_outside_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = dir.path().join("outside").join("swimmers");
        let workspaces = base.join(".bv").join(BV_WORKSPACES_DIR);
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::create_dir_all(&workspaces).expect("workspaces");
        write_workspace_yaml(&workspaces.join("orchestration.yaml"), &repo);
        let config = effective_dir_config_for_base(&base).expect("workspace config");

        let resolved = super::super::resolve_group_membership_path(
            &base.canonicalize().expect("canonical base"),
            &repo.to_string_lossy(),
            &config,
        )
        .expect("workspace repo path allowed");

        assert_eq!(resolved, repo.canonicalize().expect("canonical repo"));
    }

    #[test]
    fn resolve_target_path_with_group_roots_allows_group_descendants_outside_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = dir.path().join("outside").join("swimmers");
        let child = repo.join("src");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::create_dir_all(&child).expect("child");
        let config = OverlayDirConfig {
            label: "test".into(),
            base_path: base.clone(),
            services: Vec::new(),
            groups: vec![OverlayDirGroup {
                name: "orchestration".into(),
                paths: vec![repo.clone()],
                dirs: Vec::new(),
            }],
            launch: OverlayLaunchConfig::local_only(),
        };

        let strict = resolve_target_path(base.clone(), child.clone())
            .expect_err("plain base resolver should remain strict");
        assert_eq!(strict.status, StatusCode::FORBIDDEN);
        assert_eq!(strict.code, "DIR_OUTSIDE_BASE");

        let (_, resolved) =
            resolve_target_path_with_group_roots(base, child.clone(), Some(&config))
                .expect("group descendant path should be browsable");

        assert_eq!(resolved, child.canonicalize().expect("canonical child"));
    }

    #[test]
    fn resolve_target_path_with_group_roots_allows_source_dir_descendants_outside_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let source = dir.path().join("workspaces");
        let repo = source.join("swimmers");
        let child = repo.join("src");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::create_dir_all(&child).expect("child");
        let config = OverlayDirConfig {
            label: "test".into(),
            base_path: base.clone(),
            services: Vec::new(),
            groups: vec![OverlayDirGroup {
                name: "workspace".into(),
                paths: Vec::new(),
                dirs: vec![source],
            }],
            launch: OverlayLaunchConfig::local_only(),
        };

        let (_, resolved) =
            resolve_target_path_with_group_roots(base, child.clone(), Some(&config))
                .expect("group source descendant path should be browsable");

        assert_eq!(resolved, child.canonicalize().expect("canonical child"));
    }

    #[test]
    fn effective_groups_merge_overlay_includes_and_excludes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let frontend = dir.path().join("frontend-app");
        let backend = dir.path().join("backend-app");
        let skills_root = dir.path().join("skills");
        let skill = skills_root.join("alpha-skill");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&skill).expect("skill");

        let config = OverlayDirConfig {
            label: "test".into(),
            base_path: dir.path().to_path_buf(),
            services: Vec::new(),
            groups: vec![
                OverlayDirGroup {
                    name: "frontend".into(),
                    paths: vec![frontend.clone()],
                    dirs: Vec::new(),
                },
                OverlayDirGroup {
                    name: "backend".into(),
                    paths: vec![backend.clone()],
                    dirs: Vec::new(),
                },
                OverlayDirGroup {
                    name: "skills".into(),
                    paths: Vec::new(),
                    dirs: vec![skills_root],
                },
            ],
            launch: OverlayLaunchConfig::local_only(),
        };

        let mut memberships = DirGroupMemberships::default();
        memberships
            .groups
            .entry("frontend".into())
            .or_default()
            .include_paths
            .insert(canonical_path_string(&backend));
        memberships
            .groups
            .entry("backend".into())
            .or_default()
            .exclude_paths
            .insert(canonical_path_string(&backend));
        memberships
            .groups
            .entry("skills".into())
            .or_default()
            .exclude_paths
            .insert(canonical_path_string(&skill));

        assert_eq!(
            effective_groups_for_path(&config, &memberships, &frontend),
            vec!["frontend".to_string()]
        );
        assert_eq!(
            effective_groups_for_path(&config, &memberships, &backend),
            vec!["frontend".to_string()]
        );
        assert!(effective_groups_for_path(&config, &memberships, &skill).is_empty());
    }

    #[test]
    fn list_effective_group_entries_filters_and_keeps_duplicate_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first_shared = dir.path().join("a").join("shared");
        let duplicate_shared = dir.path().join("b").join("shared");
        let excluded = dir.path().join("c").join("excluded");
        let file_only = dir.path().join("d").join("file-only");
        let missing = dir.path().join("missing").join("repo");
        std::fs::create_dir_all(&first_shared).expect("first shared");
        std::fs::create_dir_all(&duplicate_shared).expect("duplicate shared");
        std::fs::create_dir_all(&excluded).expect("excluded");
        std::fs::create_dir_all(file_only.parent().expect("file parent")).expect("file parent");
        std::fs::write(&file_only, "not a directory").expect("file only");

        let first_shared_path = canonical_path_string(&first_shared);
        let duplicate_shared_path = canonical_path_string(&duplicate_shared);
        let excluded_path = canonical_path_string(&excluded);
        let file_only_path = canonical_path_string(&file_only);
        let missing_path = canonical_path_string(&missing);
        let group = OverlayDirGroup {
            name: "frontend".into(),
            paths: Vec::new(),
            dirs: Vec::new(),
        };
        let mut memberships = DirGroupMemberships::default();
        let delta = memberships.groups.entry("frontend".into()).or_default();
        delta.include_paths.insert(duplicate_shared_path.clone());
        delta.include_paths.insert(first_shared_path.clone());
        delta.include_paths.insert(excluded_path.clone());
        delta.include_paths.insert(file_only_path);
        delta.include_paths.insert(missing_path);
        delta.exclude_paths.insert(excluded_path);

        let entries = list_effective_group_entries_sync(&group, &memberships);

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.name == "shared"));
        assert_eq!(
            entries[0].full_path.as_deref(),
            Some(first_shared_path.as_str())
        );
        assert_eq!(
            entries[1].full_path.as_deref(),
            Some(duplicate_shared_path.as_str())
        );
    }

    #[test]
    fn list_effective_group_entries_excludes_overlay_and_source_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exact_repo = dir.path().join("exact-repo");
        let source = dir.path().join("source");
        let excluded_child = source.join("excluded-child");
        let kept_child = source.join("kept-child");
        std::fs::create_dir_all(&exact_repo).expect("exact repo");
        std::fs::create_dir_all(&excluded_child).expect("excluded child");
        std::fs::create_dir_all(&kept_child).expect("kept child");

        let group = OverlayDirGroup {
            name: "frontend".into(),
            paths: vec![exact_repo.clone()],
            dirs: vec![source],
        };
        let mut memberships = DirGroupMemberships::default();
        let delta = memberships.groups.entry("frontend".into()).or_default();
        delta
            .exclude_paths
            .insert(canonical_path_string(&exact_repo));
        delta
            .exclude_paths
            .insert(canonical_path_string(&excluded_child));

        let entries = list_effective_group_entries_sync(&group, &memberships);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "kept-child");
        assert_eq!(
            entries[0].full_path.as_deref(),
            Some(canonical_path_string(&kept_child).as_str())
        );
    }

    #[test]
    fn list_effective_group_entries_skips_hidden_includes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hidden = dir.path().join(".hidden-repo");
        let visible = dir.path().join("visible-repo");
        std::fs::create_dir_all(&hidden).expect("hidden");
        std::fs::create_dir_all(&visible).expect("visible");

        let group = OverlayDirGroup {
            name: "frontend".into(),
            paths: Vec::new(),
            dirs: Vec::new(),
        };
        let mut memberships = DirGroupMemberships::default();
        let delta = memberships.groups.entry("frontend".into()).or_default();
        delta.include_paths.insert(canonical_path_string(&hidden));
        delta.include_paths.insert(canonical_path_string(&visible));

        let entries = list_effective_group_entries_sync(&group, &memberships);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "visible-repo");
    }

    #[test]
    fn list_effective_group_entries_applies_user_include_and_exclude_deltas() {
        let dir = tempfile::tempdir().expect("tempdir");
        let overlay_repo = dir.path().join("overlay-repo");
        let user_repo = dir.path().join("user-repo");
        let source = dir.path().join("source");
        let source_child = source.join("source-child");
        std::fs::create_dir_all(&overlay_repo).expect("overlay repo");
        std::fs::create_dir_all(&user_repo).expect("user repo");
        std::fs::create_dir_all(&source_child).expect("source child");

        let group = OverlayDirGroup {
            name: "frontend".into(),
            paths: vec![overlay_repo.clone()],
            dirs: vec![source],
        };
        let mut memberships = DirGroupMemberships::default();
        let delta = memberships.groups.entry("frontend".into()).or_default();
        delta
            .exclude_paths
            .insert(canonical_path_string(&overlay_repo));
        delta
            .include_paths
            .insert(canonical_path_string(&user_repo));

        let entries = list_effective_group_entries_sync(&group, &memberships);
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["source-child", "user-repo"]);
        assert!(entries.iter().all(|entry| entry.full_path.is_some()));
    }
}
