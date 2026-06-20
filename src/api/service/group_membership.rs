use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;

use super::service_directory::{
    canonical_path_string, effective_dir_config_for_base, effective_groups_for_path,
    overlay_group_contains_path,
};
use super::{dir_groups, dirs_base_path, ApiServiceError};
use crate::api::AppState;
use crate::persistence::file_store::FileStore;
use crate::session::overlay::OverlayDirConfig;
use crate::types::{
    DirGroupMembershipUpdateRequest, DirGroupMembershipUpdateResponse, DirGroupMemberships,
};

pub(super) async fn load_dir_group_memberships(state: &Arc<AppState>) -> DirGroupMemberships {
    match state.current_file_store() {
        Some(store) => store.load_dir_group_memberships().await,
        None => DirGroupMemberships::default(),
    }
}

pub async fn update_dir_group_memberships(
    state: Arc<AppState>,
    body: DirGroupMembershipUpdateRequest,
) -> Result<DirGroupMembershipUpdateResponse, ApiServiceError> {
    let preflight = update_dir_group_memberships_preflight(
        state.current_file_store(),
        dirs_base_path(),
        effective_dir_config_for_base,
    )?;

    update_dir_group_memberships_with_config(
        preflight.store,
        &preflight.canonical_base,
        &preflight.dir_config,
        body,
    )
    .await
}

pub(super) struct DirGroupMembershipUpdatePreflight {
    pub(super) store: Arc<FileStore>,
    pub(super) canonical_base: PathBuf,
    pub(super) dir_config: OverlayDirConfig,
}

pub(super) fn update_dir_group_memberships_preflight(
    store: Option<Arc<FileStore>>,
    base: PathBuf,
    dir_config_for_base: impl FnOnce(&Path) -> Option<OverlayDirConfig>,
) -> Result<DirGroupMembershipUpdatePreflight, ApiServiceError> {
    let store = store.ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PERSISTENCE_UNAVAILABLE",
            "directory group edits require file persistence",
        )
    })?;

    let canonical_base = base.canonicalize().unwrap_or(base);
    let dir_config = dir_config_for_base(&canonical_base).ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OVERLAY_UNAVAILABLE",
            "directory group edits require a configured directory group source",
        )
    })?;
    if dir_config.groups.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "GROUPS_UNAVAILABLE",
            "no directory groups are configured",
        ));
    }

    Ok(DirGroupMembershipUpdatePreflight {
        store,
        canonical_base,
        dir_config,
    })
}

pub(super) async fn update_dir_group_memberships_with_config(
    store: Arc<FileStore>,
    canonical_base: &Path,
    dir_config: &OverlayDirConfig,
    body: DirGroupMembershipUpdateRequest,
) -> Result<DirGroupMembershipUpdateResponse, ApiServiceError> {
    let canonical_path = resolve_group_membership_path(canonical_base, &body.path, dir_config)?;
    let available_groups = dir_groups(Some(dir_config));
    let valid_groups = available_groups.iter().cloned().collect::<BTreeSet<_>>();
    let add = normalize_group_update_names(&body.add, &valid_groups)?;
    let remove = normalize_group_update_names(&body.remove, &valid_groups)?;
    if add.is_empty() && remove.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "GROUP_UPDATE_EMPTY",
            "at least one group must be added or removed",
        ));
    }

    let path = canonical_path_string(&canonical_path);
    let update_path = path.clone();
    let overlay_removals = overlay_group_membership_removals(dir_config, &canonical_path, &remove);
    let memberships = store
        .update_dir_group_memberships(move |memberships| {
            apply_group_membership_update_with_overlay_removals(
                memberships,
                &update_path,
                add,
                remove,
                &overlay_removals,
            );
        })
        .await
        .map_err(|error| {
            ApiServiceError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "GROUP_UPDATE_FAILED",
                format!("failed to persist directory group edits: {error}"),
            )
        })?;

    Ok(DirGroupMembershipUpdateResponse {
        groups: effective_groups_for_path(dir_config, &memberships, &canonical_path),
        available_groups,
        path,
    })
}

pub(super) fn resolve_group_membership_path(
    canonical_base: &Path,
    raw_path: &str,
    config: &OverlayDirConfig,
) -> Result<PathBuf, ApiServiceError> {
    let trimmed = require_group_membership_path(raw_path)?;
    let canonical = canonical_group_membership_dir(trimmed)?;
    if group_membership_path_allowed(canonical_base, &canonical, config) {
        return Ok(canonical);
    }

    Err(group_membership_outside_roots_error())
}

fn require_group_membership_path(raw_path: &str) -> Result<&str, ApiServiceError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "GROUP_PATH_REQUIRED",
            "path is required",
        ));
    }
    Ok(trimmed)
}

fn canonical_group_membership_dir(path: &str) -> Result<PathBuf, ApiServiceError> {
    let canonical = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| group_membership_dir_not_found_error(path))?;
    if !canonical.is_dir() {
        return Err(group_membership_dir_not_found_error(path));
    }
    Ok(canonical)
}

fn group_membership_dir_not_found_error(path: &str) -> ApiServiceError {
    ApiServiceError::new(
        StatusCode::NOT_FOUND,
        "DIR_NOT_FOUND",
        format!("directory not found: {path}"),
    )
}

fn group_membership_path_allowed(
    canonical_base: &Path,
    canonical: &Path,
    config: &OverlayDirConfig,
) -> bool {
    canonical.starts_with(canonical_base)
        || config
            .groups
            .iter()
            .any(|group| overlay_group_contains_path(group, canonical))
}

fn group_membership_outside_roots_error() -> ApiServiceError {
    ApiServiceError::new(
        StatusCode::FORBIDDEN,
        "DIR_OUTSIDE_BASE",
        "path is outside the allowed directory group roots",
    )
}

pub(super) fn normalize_group_update_names(
    groups: &[String],
    valid_groups: &BTreeSet<String>,
) -> Result<Vec<String>, ApiServiceError> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for raw in groups {
        let name = raw.trim();
        if name.is_empty() {
            return Err(ApiServiceError::new(
                StatusCode::BAD_REQUEST,
                "GROUP_NAME_REQUIRED",
                "group names must not be empty",
            ));
        }
        if !valid_groups.contains(name) {
            return Err(ApiServiceError::new(
                StatusCode::NOT_FOUND,
                "GROUP_NOT_FOUND",
                format!("no group named '{name}' in overlay"),
            ));
        }
        if seen.insert(name.to_string()) {
            normalized.push(name.to_string());
        }
    }
    Ok(normalized)
}

fn prune_empty_group_deltas(memberships: &mut DirGroupMemberships) {
    memberships
        .groups
        .retain(|_, delta| !delta.include_paths.is_empty() || !delta.exclude_paths.is_empty());
}

fn overlay_group_membership_removals(
    config: &OverlayDirConfig,
    canonical_path: &Path,
    groups: &[String],
) -> BTreeSet<String> {
    config
        .groups
        .iter()
        .filter(|group| groups.iter().any(|name| name == &group.name))
        .filter(|group| overlay_group_contains_path(group, canonical_path))
        .map(|group| group.name.clone())
        .collect()
}

#[cfg(test)]
pub(super) fn apply_group_membership_update(
    memberships: &mut DirGroupMemberships,
    path: &str,
    add: Vec<String>,
    remove: Vec<String>,
) {
    let overlay_removals = remove.iter().cloned().collect::<BTreeSet<_>>();
    apply_group_membership_update_with_overlay_removals(
        memberships,
        path,
        add,
        remove,
        &overlay_removals,
    );
}

fn apply_group_membership_update_with_overlay_removals(
    memberships: &mut DirGroupMemberships,
    path: &str,
    add: Vec<String>,
    remove: Vec<String>,
    overlay_removals: &BTreeSet<String>,
) {
    apply_group_membership_removes(memberships, path, remove, overlay_removals);
    apply_group_membership_adds(memberships, path, add);
    prune_empty_group_deltas(memberships);
}

fn apply_group_membership_removes(
    memberships: &mut DirGroupMemberships,
    path: &str,
    groups: Vec<String>,
    overlay_removals: &BTreeSet<String>,
) {
    for group in groups {
        let removes_overlay_membership = overlay_removals.contains(&group);
        apply_group_membership_remove(memberships, path, group, removes_overlay_membership);
    }
}

fn apply_group_membership_adds(
    memberships: &mut DirGroupMemberships,
    path: &str,
    groups: Vec<String>,
) {
    for group in groups {
        apply_group_membership_add(memberships, path, group);
    }
}

fn apply_group_membership_remove(
    memberships: &mut DirGroupMemberships,
    path: &str,
    group: String,
    removes_overlay_membership: bool,
) {
    let delta = memberships.groups.entry(group).or_default();
    delta.include_paths.remove(path);
    if removes_overlay_membership {
        delta.exclude_paths.insert(path.to_string());
    } else {
        delta.exclude_paths.remove(path);
    }
}

fn apply_group_membership_add(memberships: &mut DirGroupMemberships, path: &str, group: String) {
    let delta = memberships.groups.entry(group).or_default();
    delta.exclude_paths.remove(path);
    delta.include_paths.insert(path.to_string());
}
