use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;

use crate::types::{DirEntry, DirRepoSearchResponse};

use super::ApiServiceError;

const REPO_SEARCH_ROOTS_ENV: &str = "SWIMMERS_REPO_SEARCH_ROOTS";
const REPO_SEARCH_MAX_DEPTH_ENV: &str = "SWIMMERS_REPO_SEARCH_MAX_DEPTH";
pub(super) const REPO_SEARCH_DEFAULT_MAX_DEPTH: usize = 8;
const REPO_SEARCH_CACHE_TTL: Duration = Duration::from_secs(60);

pub(super) enum RepoSearchVisit {
    Repository,
    Descend,
    Skip,
}

#[derive(Clone)]
struct RepoSearchCacheEntry {
    roots: Vec<PathBuf>,
    max_depth: usize,
    generated_at: Instant,
    entries: Vec<DirEntry>,
}

static REPO_SEARCH_CACHE: OnceLock<Mutex<Option<RepoSearchCacheEntry>>> = OnceLock::new();

fn repo_search_cache() -> &'static Mutex<Option<RepoSearchCacheEntry>> {
    REPO_SEARCH_CACHE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
pub fn clear_repo_search_cache_for_tests() {
    if let Ok(mut cache) = repo_search_cache().lock() {
        *cache = None;
    }
}

fn repo_search_roots() -> Vec<PathBuf> {
    let configured = std::env::var_os(REPO_SEARCH_ROOTS_ENV)
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|home| vec![home.join("repos"), home.join("hard")])
                .unwrap_or_default()
        });

    let mut seen = BTreeSet::new();
    configured
        .into_iter()
        .map(expand_repo_search_root)
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let canonical = path.canonicalize().unwrap_or(path);
            seen.insert(canonical.clone()).then_some(canonical)
        })
        .collect()
}

fn expand_repo_search_root(path: PathBuf) -> PathBuf {
    let Some(raw) = path.to_str().map(|value| value.to_string()) else {
        return path;
    };
    let Some(home) = dirs::home_dir() else {
        return path;
    };
    if raw == "~" {
        return home;
    }
    raw.strip_prefix("~/")
        .map(|suffix| home.join(suffix))
        .unwrap_or(path)
}

fn repo_search_max_depth() -> usize {
    std::env::var(REPO_SEARCH_MAX_DEPTH_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|depth| *depth > 0)
        .unwrap_or(REPO_SEARCH_DEFAULT_MAX_DEPTH)
}

fn should_descend_for_repo_search(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }

    !matches!(
        name,
        "node_modules"
            | "target"
            | "dist"
            | "build"
            | "DerivedData"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
    )
}

fn compact_repo_search_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(suffix) = path.strip_prefix(&home) {
            let suffix = suffix.to_string_lossy();
            if suffix.is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", suffix.trim_start_matches('/'));
        }
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_repo_search_path_uses_tilde_for_home_paths() {
        let home = dirs::home_dir().expect("home dir");

        assert_eq!(compact_repo_search_path(&home), "~");
        assert_eq!(
            compact_repo_search_path(&home.join("repos").join("swimmers")),
            "~/repos/swimmers"
        );
    }
}

fn repo_search_entry(path: &Path) -> DirEntry {
    let basename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let compact = compact_repo_search_path(path);
    let name = if compact.ends_with(&basename) {
        format!("{basename}  {compact}")
    } else {
        basename
    };
    DirEntry {
        name,
        has_children: false,
        is_running: None,
        repo_dirty: None,
        repo_action: None,
        group: None,
        groups: Vec::new(),
        full_path: Some(path.to_string_lossy().into_owned()),
        has_restart: None,
        open_url: None,
    }
}

fn repo_search_queue(roots: &[PathBuf]) -> VecDeque<(PathBuf, usize)> {
    roots.iter().cloned().map(|root| (root, 0usize)).collect()
}

fn unseen_repo_search_path(path: PathBuf, seen: &mut BTreeSet<PathBuf>) -> Option<PathBuf> {
    let canonical = path.canonicalize().unwrap_or(path);
    seen.insert(canonical.clone()).then_some(canonical)
}

pub(super) fn repo_search_visit(path: &Path, depth: usize, max_depth: usize) -> RepoSearchVisit {
    if path.join(".git").exists() {
        return RepoSearchVisit::Repository;
    }
    if depth >= max_depth {
        return RepoSearchVisit::Skip;
    }
    RepoSearchVisit::Descend
}

fn repo_search_child_dir_path(child: std::fs::DirEntry) -> Option<PathBuf> {
    let file_type = child.file_type().ok()?;
    file_type.is_dir().then_some(())?;
    let name = child.file_name().to_string_lossy().into_owned();
    should_descend_for_repo_search(&name).then(|| child.path())
}

pub(super) fn repo_search_child_dirs(path: &Path) -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    read_dir
        .flatten()
        .filter_map(repo_search_child_dir_path)
        .collect()
}

fn enqueue_repo_search_children(
    queue: &mut VecDeque<(PathBuf, usize)>,
    parent: &Path,
    next_depth: usize,
) {
    queue.extend(
        repo_search_child_dirs(parent)
            .into_iter()
            .map(|child| (child, next_depth)),
    );
}

fn sort_repo_search_entries(repos: &mut [DirEntry]) {
    repos.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.full_path.cmp(&right.full_path))
    });
}

pub(super) fn scan_repo_search_roots_sync(roots: &[PathBuf], max_depth: usize) -> Vec<DirEntry> {
    let mut queue = repo_search_queue(roots);
    let mut seen = BTreeSet::new();
    let mut repos = Vec::new();
    while let Some((path, depth)) = queue.pop_front() {
        let Some(canonical) = unseen_repo_search_path(path, &mut seen) else {
            continue;
        };

        match repo_search_visit(&canonical, depth, max_depth) {
            RepoSearchVisit::Repository => repos.push(repo_search_entry(&canonical)),
            RepoSearchVisit::Descend => {
                enqueue_repo_search_children(&mut queue, &canonical, depth + 1);
            }
            RepoSearchVisit::Skip => {}
        }
    }

    sort_repo_search_entries(&mut repos);
    repos
}

fn cached_repo_search_entries(roots: &[PathBuf], max_depth: usize) -> Option<Vec<DirEntry>> {
    let cache = repo_search_cache().lock().ok()?;
    let cache = cache.as_ref()?;
    (cache.roots == roots
        && cache.max_depth == max_depth
        && cache.generated_at.elapsed() < REPO_SEARCH_CACHE_TTL)
        .then(|| cache.entries.clone())
}

fn write_repo_search_cache(roots: &[PathBuf], max_depth: usize, entries: &[DirEntry]) {
    if let Ok(mut cache) = repo_search_cache().lock() {
        *cache = Some(RepoSearchCacheEntry {
            roots: roots.to_vec(),
            max_depth,
            generated_at: Instant::now(),
            entries: entries.to_vec(),
        });
    }
}

pub async fn list_repo_search_entries() -> Result<DirRepoSearchResponse, ApiServiceError> {
    let roots = repo_search_roots();
    let max_depth = repo_search_max_depth();
    list_repo_search_entries_inner(roots, max_depth).await
}

pub(super) async fn list_repo_search_entries_inner(
    roots: Vec<PathBuf>,
    max_depth: usize,
) -> Result<DirRepoSearchResponse, ApiServiceError> {
    let root_labels = roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Ok(DirRepoSearchResponse {
            roots: root_labels,
            entries: Vec::new(),
        });
    }

    if let Some(entries) = cached_repo_search_entries(&roots, max_depth) {
        return Ok(DirRepoSearchResponse {
            roots: root_labels,
            entries,
        });
    }

    let scan_roots = roots.clone();
    let entries =
        tokio::task::spawn_blocking(move || scan_repo_search_roots_sync(&scan_roots, max_depth))
            .await
            .map_err(|err| {
                ApiServiceError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "REPO_SEARCH_FAILED",
                    format!("repository search task failed: {err}"),
                )
            })?;
    write_repo_search_cache(&roots, max_depth, &entries);

    Ok(DirRepoSearchResponse {
        roots: root_labels,
        entries,
    })
}
