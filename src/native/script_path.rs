use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};

use crate::types::NativeDesktopApp;

pub(super) fn script_path_for_app(app: NativeDesktopApp) -> Result<PathBuf> {
    let override_root = std::env::var_os(super::NATIVE_SCRIPT_ROOT_ENV).map(PathBuf::from);
    let current_exe = std::env::current_exe().ok();
    let current_dir = std::env::current_dir().ok();
    resolve_script_path(
        app.script_relative_path(),
        override_root.as_deref(),
        current_exe.as_deref(),
        current_dir.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &bundled_script_root(),
        app.bundled_script_source(),
    )
}

pub(super) fn script_path_for_app_without_materializing(app: NativeDesktopApp) -> Result<PathBuf> {
    let override_root = std::env::var_os(super::NATIVE_SCRIPT_ROOT_ENV).map(PathBuf::from);
    let current_exe = std::env::current_exe().ok();
    let current_dir = std::env::current_dir().ok();
    resolve_script_path_without_materializing(
        app.script_relative_path(),
        override_root.as_deref(),
        current_exe.as_deref(),
        current_dir.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &bundled_script_root(),
    )
}

pub(super) fn resolve_script_path(
    script_relative_path: &str,
    override_root: Option<&Path>,
    current_exe: Option<&Path>,
    current_dir: Option<&Path>,
    manifest_dir: &Path,
    bundled_root: &Path,
    bundled_source: &str,
) -> Result<PathBuf> {
    if let Some(path) = override_script_path(script_relative_path, override_root) {
        return Ok(path);
    }

    if let Some(path) = find_existing_script_path(
        script_relative_path,
        checkout_script_roots(current_dir, current_exe, manifest_dir),
    ) {
        return Ok(path);
    }

    materialize_bundled_script(script_relative_path, bundled_root, bundled_source)
}

fn resolve_script_path_without_materializing(
    script_relative_path: &str,
    override_root: Option<&Path>,
    current_exe: Option<&Path>,
    current_dir: Option<&Path>,
    manifest_dir: &Path,
    bundled_root: &Path,
) -> Result<PathBuf> {
    if let Some(path) = override_script_path(script_relative_path, override_root) {
        return Ok(path);
    }

    Ok(find_existing_script_path(
        script_relative_path,
        health_check_script_roots(current_dir, current_exe, manifest_dir, bundled_root),
    )
    .unwrap_or_else(|| bundled_root.join(script_relative_path)))
}

fn override_script_path(
    script_relative_path: &str,
    override_root: Option<&Path>,
) -> Option<PathBuf> {
    override_root.map(|root| root.join(script_relative_path))
}

fn checkout_script_roots(
    current_dir: Option<&Path>,
    current_exe: Option<&Path>,
    manifest_dir: &Path,
) -> Vec<PathBuf> {
    candidate_script_roots(current_dir, current_exe, manifest_dir, None)
}

fn health_check_script_roots(
    current_dir: Option<&Path>,
    current_exe: Option<&Path>,
    manifest_dir: &Path,
    bundled_root: &Path,
) -> Vec<PathBuf> {
    candidate_script_roots(current_dir, current_exe, manifest_dir, Some(bundled_root))
}

fn candidate_script_roots(
    current_dir: Option<&Path>,
    current_exe: Option<&Path>,
    manifest_dir: &Path,
    extra_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = current_dir {
        push_ancestor_roots(&mut roots, dir);
    }
    if let Some(exe_dir) = current_exe.and_then(Path::parent) {
        push_ancestor_roots(&mut roots, exe_dir);
    }
    push_unique_root(&mut roots, manifest_dir);
    if let Some(root) = extra_root {
        push_unique_root(&mut roots, root);
    }
    roots
}

fn find_existing_script_path(script_relative_path: &str, roots: Vec<PathBuf>) -> Option<PathBuf> {
    roots
        .into_iter()
        .map(|root| root.join(script_relative_path))
        .find(|candidate| candidate.is_file())
}

fn bundled_script_root() -> PathBuf {
    let data_dir = match std::env::var_os("SWIMMERS_DATA_DIR") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => dirs::data_dir()
            .map(|base| base.join("swimmers"))
            .unwrap_or_else(|| PathBuf::from("./data/swimmers/")),
    };
    data_dir
        .join("native-scripts")
        .join(env!("CARGO_PKG_VERSION"))
}

/// Builds a per-call unique suffix for atomic temp-file writes so that two
/// concurrent in-process callers (reachable from the unsynchronized
/// native-status endpoint) never share a tmp path and race on the same file.
pub(super) fn unique_tmp_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{:?}.{counter}.{nanos}", std::thread::current().id())
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '.', "")
}

pub(super) fn materialize_bundled_script(
    script_relative_path: &str,
    bundled_root: &Path,
    bundled_source: &str,
) -> Result<PathBuf> {
    let target = bundled_root.join(script_relative_path);
    if let Ok(existing) = std::fs::read_to_string(&target) {
        if existing == bundled_source {
            return Ok(target);
        }
    }

    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("native script path has no parent: {}", target.display()))?;
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create native script directory {}",
            parent.display()
        )
    })?;

    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("native script path has no file name: {}", target.display()))?;
    let tmp_path = target.with_file_name(format!(
        "{file_name}.{}.{}.tmp",
        std::process::id(),
        unique_tmp_suffix()
    ));
    std::fs::write(&tmp_path, bundled_source).with_context(|| {
        format!(
            "failed to write bundled native script {}",
            tmp_path.display()
        )
    })?;
    std::fs::rename(&tmp_path, &target).with_context(|| {
        let _ = std::fs::remove_file(&tmp_path);
        format!(
            "failed to install bundled native script at {}",
            target.display()
        )
    })?;
    Ok(target)
}

fn push_ancestor_roots(roots: &mut Vec<PathBuf>, start: &Path) {
    for ancestor in start.ancestors() {
        push_unique_root(roots, ancestor);
    }
}

fn push_unique_root(roots: &mut Vec<PathBuf>, candidate: &Path) {
    if candidate.as_os_str().is_empty() {
        return;
    }
    if roots.iter().any(|existing| existing == candidate) {
        return;
    }
    roots.push(candidate.to_path_buf());
}
