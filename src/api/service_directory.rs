use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::http::StatusCode;

use super::ApiServiceError;
use crate::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayLaunchConfig, OverlayServiceEntry,
};
use crate::types::{DirEntry, DirGroupMemberships};

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
        .filter(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())))
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
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect::<BTreeSet<_>>();
    for path in additions {
        let key = path.canonicalize().unwrap_or_else(|_| path.clone());
        if seen.insert(key) {
            target.push(path);
        }
    }
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
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let canonical_base = context
        .base_path
        .canonicalize()
        .unwrap_or_else(|_| context.base_path.clone());
    if target == canonical_base {
        return Vec::new();
    };

    let mut services = BTreeSet::new();
    for service in &context.services {
        let service_path = service_dir_path(&canonical_base, &service.dir);
        let canonical_service = service_path
            .canonicalize()
            .unwrap_or_else(|_| service_path.clone());
        if canonical_service == target
            || canonical_service.starts_with(&target)
            || target.starts_with(&canonical_service)
        {
            services.insert(service.name.clone());
        }
    }

    services.into_iter().collect()
}

pub fn resolve_target_path(
    base: PathBuf,
    target: PathBuf,
) -> Result<(PathBuf, PathBuf), ApiServiceError> {
    let canonical = target.canonicalize().map_err(|_| {
        ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            format!("directory not found: {}", target.display()),
        )
    })?;

    let canonical_base = base.canonicalize().unwrap_or(base);
    if !canonical.starts_with(&canonical_base) {
        return Err(ApiServiceError::new(
            StatusCode::FORBIDDEN,
            "DIR_OUTSIDE_BASE",
            "path is outside the allowed base directory",
        ));
    }

    Ok((canonical_base, canonical))
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
    let mut seen = BTreeSet::new();
    let mut entries: Vec<(DirEntry, u64)> = Vec::new();

    for entry_path in &group.paths {
        let Some(name) = entry_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name.starts_with('.') || !seen.insert(name.clone()) {
            continue;
        }

        let full_path = entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.clone())
            .to_string_lossy()
            .into_owned();

        entries.push((
            DirEntry {
                name,
                has_children: false,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: Some(full_path),
                has_restart: None,
                open_url: None,
            },
            modified_secs(entry_path),
        ));
    }

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

            let modified_at = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);

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
                    groups: Vec::new(),
                    full_path: Some(full_path),
                    has_restart: None,
                    open_url: None,
                },
                modified_at,
            ));
        }
    }

    entries.sort_by(|(a, _), (b, _)| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.into_iter().map(|(entry, _)| entry).collect()
}

pub fn list_effective_group_entries_sync(
    group: &OverlayDirGroup,
    memberships: &DirGroupMemberships,
) -> Vec<DirEntry> {
    let Some(delta) = memberships.groups.get(&group.name) else {
        return list_group_entries_sync(group);
    };

    let mut seen_names = BTreeSet::new();
    let mut entries: Vec<(DirEntry, u64)> = list_group_entries_sync(group)
        .into_iter()
        .filter(|entry| {
            entry
                .full_path
                .as_deref()
                .map(|path| !delta.exclude_paths.contains(path))
                .unwrap_or(true)
        })
        .map(|entry| {
            seen_names.insert(entry.name.clone());
            let modified_at = entry
                .full_path
                .as_deref()
                .map(|path| modified_secs(Path::new(path)))
                .unwrap_or(0);
            (entry, modified_at)
        })
        .collect();

    for raw_path in &delta.include_paths {
        if delta.exclude_paths.contains(raw_path) {
            continue;
        }
        let path = PathBuf::from(raw_path);
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name.starts_with('.') || !seen_names.insert(name.clone()) {
            continue;
        }

        entries.push((
            DirEntry {
                name,
                has_children: false,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: Some(canonical_path_string(&path)),
                has_restart: None,
                open_url: None,
            },
            modified_secs(&path),
        ));
    }

    entries.sort_by(|(a, _), (b, _)| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.into_iter().map(|(entry, _)| entry).collect()
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
