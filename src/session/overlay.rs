use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use serde::Deserialize;

/// Cached skillbox overlay, loaded once from disk.
pub fn default_overlay() -> Option<&'static SkillboxOverlay> {
    static OVERLAY: OnceLock<Option<SkillboxOverlay>> = OnceLock::new();
    OVERLAY.get_or_init(SkillboxOverlay::load).as_ref()
}

pub struct SkillboxOverlay {
    clients: Vec<ClientOverlay>,
}

/// A service entry declared in the overlay's `dev_sanity.services` section.
#[derive(Debug, Clone)]
pub struct OverlayServiceEntry {
    /// Service identifier (e.g. `"svc-alpha"`).
    pub name: String,
    /// Relative directory path from `base_path` (e.g. `"alpha"` or `"services/nested-app"`).
    pub dir: String,
    /// Optional HTTP URL for health checks.
    pub health_url: Option<String>,
    /// Optional shell command to restart the service.
    pub restart: Option<String>,
    /// Optional URL to open in a browser (e.g. local dev server).
    pub open_url: Option<String>,
}

/// A virtual directory group that aggregates entries from multiple source paths.
#[derive(Debug, Clone)]
pub struct OverlayDirGroup {
    /// Display name shown in the picker (e.g. "skills").
    pub name: String,
    /// Source directories whose immediate children become group entries.
    pub dirs: Vec<PathBuf>,
}

/// A domain plan discovered on disk under an overlay's `plans/{released,draft}` root.
#[derive(Debug, Clone)]
pub struct OverlayPlanEntry {
    /// Plan directory name, e.g. `"hybrid_booking_wrapper"`.
    pub slug: String,
    /// Display label for the overlay client that owns the plan (e.g. `"personal"`).
    pub client_label: String,
    /// Either `"released"` or `"draft"` — which plans subfolder the entry came from.
    pub kind: &'static str,
    /// Absolute path to the plan's `schema.mmd` file.
    pub schema_path: PathBuf,
    /// Most-recent mtime across the plan directory's files, for sort-by-recent.
    pub updated_at: Option<SystemTime>,
}

/// Directory browsing configuration derived from an overlay's `dev_sanity` section.
#[derive(Debug, Clone)]
pub struct OverlayDirConfig {
    /// Client label (e.g. "personal", "jeremy") for display in the TUI.
    pub label: String,
    /// Root directory for directory browsing.
    pub base_path: PathBuf,
    /// Services declared in the overlay.
    pub services: Vec<OverlayServiceEntry>,
    /// Virtual directory groups shown alongside managed entries.
    pub groups: Vec<OverlayDirGroup>,
}

struct ClientOverlay {
    #[allow(dead_code)]
    // FIXME(2026-04-21): Retained for overlay diagnostics/export metadata that is not surfaced yet.
    client_dir: PathBuf,
    label: String,
    cwd_patterns: Vec<String>,
    /// Number of explicit cwd_match entries (not repo_landscape paths).
    cwd_match_count: usize,
    plan_root: Option<PathBuf>,
    plan_draft: Option<PathBuf>,
    dir_config: Option<OverlayDirConfig>,
}

impl SkillboxOverlay {
    fn load() -> Option<Self> {
        let config_root = resolve_skillbox_config_root()?;
        let clients_dir = config_root.join("clients");
        if !clients_dir.is_dir() {
            return None;
        }

        let mut clients = Vec::new();
        let entries = std::fs::read_dir(&clients_dir).ok()?;
        for entry in entries.filter_map(Result::ok) {
            let client_dir = entry.path();
            if !client_dir.is_dir() {
                continue;
            }
            let overlay_path = client_dir.join("overlay.yaml");
            if !overlay_path.is_file() {
                continue;
            }
            if let Some(client) = parse_client_overlay(&client_dir, &overlay_path) {
                clients.push(client);
            }
        }

        if clients.is_empty() {
            None
        } else {
            Some(Self { clients })
        }
    }

    /// Find the overlay client whose `dev_sanity.services.base_path` is an
    /// ancestor of `cwd`, or whose `cwd_match` patterns match `cwd`.
    ///
    /// Prefers base_path containment (the overlay that "owns" the browsing
    /// root) over generic CWD matching, so the personal overlay's service
    /// definitions are found even when the CWD matches a single-repo overlay.
    pub fn find_dir_config(&self, cwd: &str) -> Option<&OverlayDirConfig> {
        let cwd_normalized = normalize_path(cwd);

        // First pass: find an overlay whose base_path contains the CWD.
        let by_base_path = self.clients.iter().find_map(|c| {
            let config = c.dir_config.as_ref()?;
            let base = config
                .base_path
                .canonicalize()
                .unwrap_or(config.base_path.clone());
            let base_str = base.to_string_lossy();
            if cwd_starts_with(&cwd_normalized, base_str.as_ref()) {
                Some(config)
            } else {
                None
            }
        });
        if by_base_path.is_some() {
            return by_base_path;
        }

        // Fallback: CWD-match the overlay and return its dir_config if present.
        self.clients
            .iter()
            .find(|c| {
                c.cwd_patterns
                    .iter()
                    .any(|pattern| cwd_starts_with(&cwd_normalized, pattern))
            })
            .and_then(|c| c.dir_config.as_ref())
    }

    /// Given a session CWD, find the matching client's plan directories.
    ///
    /// Multi-repo clients (more than one `cwd_match` entry) are skipped —
    /// their plan directories span multiple repos and the caller should
    /// fall back to the in-repo scan instead.
    pub fn find_plan_dirs(&self, cwd: &str) -> Option<Vec<PathBuf>> {
        let cwd_normalized = normalize_path(cwd);
        let client = self.clients.iter().find(|c| {
            c.cwd_patterns
                .iter()
                .any(|pattern| cwd_starts_with(&cwd_normalized, pattern))
        })?;

        if client.cwd_match_count > 1 {
            return None;
        }

        let mut dirs = Vec::new();
        if let Some(root) = &client.plan_root {
            if root.is_dir() {
                dirs.push(root.clone());
            }
        }
        if let Some(draft) = &client.plan_draft {
            if draft.is_dir() {
                dirs.push(draft.clone());
            }
        }
        if dirs.is_empty() {
            None
        } else {
            Some(dirs)
        }
    }

    /// Enumerate every domain plan across every overlay client.
    ///
    /// Walks each client's `plans/released` and `plans/draft` directories (if
    /// configured) and returns one entry per plan directory that contains a
    /// `schema.mmd` file. Entries are sorted by `updated_at` descending so the
    /// most-recently-touched plans come first. Paths whose components contain
    /// `archived` are skipped; `sessions` subfolders are never scanned.
    pub fn list_all_plans(&self) -> Vec<OverlayPlanEntry> {
        let mut entries = Vec::new();
        for client in &self.clients {
            let label = client_display_label(client);
            if let Some(root) = &client.plan_root {
                collect_plans_from_root(root, &label, "released", &mut entries);
            }
            if let Some(draft) = &client.plan_draft {
                collect_plans_from_root(draft, &label, "draft", &mut entries);
            }
        }
        entries.sort_by(|a, b| match (a.updated_at, b.updated_at) {
            (Some(lhs), Some(rhs)) => rhs.cmp(&lhs),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.slug.cmp(&b.slug),
        });
        entries
    }
}

fn client_display_label(client: &ClientOverlay) -> String {
    client.label.clone()
}

fn collect_plans_from_root(
    root: &Path,
    client_label: &str,
    kind: &'static str,
    out: &mut Vec<OverlayPlanEntry>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let plan_dir = entry.path();
        let slug = match plan_dir.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        // Skip any plan dir whose path mentions "archived" anywhere — the user
        // wants live plans only.
        if plan_dir
            .components()
            .any(|c| c.as_os_str().to_string_lossy().contains("archived"))
        {
            continue;
        }
        let schema_path = plan_dir.join("schema.mmd");
        if !schema_path.is_file() {
            continue;
        }
        let updated_at = plan_dir_latest_mtime(&plan_dir);
        out.push(OverlayPlanEntry {
            slug,
            client_label: client_label.to_string(),
            kind,
            schema_path,
            updated_at,
        });
    }
}

fn plan_dir_latest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut latest: Option<SystemTime> = None;
    let walk = std::fs::read_dir(dir).ok()?;
    for entry in walk.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        latest = Some(match latest {
            Some(prev) if prev >= mtime => prev,
            _ => mtime,
        });
    }
    latest
}

// ---------------------------------------------------------------------------
// Overlay YAML parsing (minimal — only extract what we need)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct OverlayFile {
    #[serde(default)]
    client: Option<OverlayClient>,
    #[serde(default)]
    dev_sanity: Option<DevSanitySection>,
}

#[derive(Deserialize, Default)]
struct OverlayClient {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    context: Option<OverlayContext>,
}

#[derive(Deserialize, Default)]
struct OverlayContext {
    #[serde(default)]
    cwd_match: Vec<String>,
    #[serde(default)]
    plans: Option<OverlayPlans>,
    #[serde(default)]
    repo_landscape: Option<RepoLandscape>,
}

#[derive(Deserialize, Default)]
struct OverlayPlans {
    #[serde(default)]
    plan_root: Option<String>,
    #[serde(default)]
    plan_draft: Option<String>,
}

#[derive(Deserialize, Default)]
struct RepoLandscape {
    #[serde(default)]
    repos: Vec<RepoEntry>,
}

#[derive(Deserialize, Default)]
struct RepoEntry {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize, Default)]
struct DevSanitySection {
    #[serde(default)]
    services: Option<DevSanityServices>,
    #[serde(default)]
    groups: Vec<DevSanityGroup>,
}

#[derive(Deserialize, Default)]
struct DevSanityGroup {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    dirs: Vec<String>,
}

#[derive(Deserialize, Default)]
struct DevSanityServices {
    #[serde(default)]
    base_path: Option<String>,
    #[serde(default)]
    entries: Vec<DevSanityServiceEntry>,
}

#[derive(Deserialize, Default)]
struct DevSanityServiceEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    health_url: Option<String>,
    #[serde(default)]
    restart: Option<String>,
    #[serde(default)]
    open_url: Option<String>,
}

fn parse_client_overlay(client_dir: &Path, overlay_path: &Path) -> Option<ClientOverlay> {
    let content = std::fs::read_to_string(overlay_path).ok()?;
    let file: OverlayFile = serde_yaml::from_str(&content).ok()?;
    let client = file.client?;
    let client_label = client
        .label
        .clone()
        .or_else(|| client.id.clone())
        .unwrap_or_else(|| {
            client_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "overlay".to_string())
        });
    let context = client.context?;

    let cwd_match_count = context.cwd_match.len();
    let mut cwd_patterns: Vec<String> = context.cwd_match.iter().map(|p| expand_path(p)).collect();

    if let Some(landscape) = &context.repo_landscape {
        for repo in &landscape.repos {
            if let Some(path) = &repo.path {
                cwd_patterns.push(expand_path(path));
            }
        }
    }

    let plans = context.plans;
    let plan_root = plans
        .as_ref()
        .and_then(|p| p.plan_root.as_deref())
        .map(|rel| client_dir.join(rel));
    let plan_draft = plans
        .as_ref()
        .and_then(|p| p.plan_draft.as_deref())
        .map(|rel| client_dir.join(rel));

    let dir_config = file.dev_sanity.and_then(|ds| {
        let groups: Vec<OverlayDirGroup> = ds
            .groups
            .into_iter()
            .filter_map(|g| {
                let name = g.name?;
                let mut dirs: Vec<PathBuf> = Vec::new();
                let mut seen: std::collections::BTreeSet<PathBuf> =
                    std::collections::BTreeSet::new();
                for raw in &g.dirs {
                    for path in expand_group_dir(raw) {
                        if seen.insert(path.clone()) {
                            dirs.push(path);
                        }
                    }
                }
                if dirs.is_empty() {
                    return None;
                }
                Some(OverlayDirGroup { name, dirs })
            })
            .collect();

        let svc = ds.services?;
        let base_path = svc
            .base_path
            .as_deref()
            .map(|p| PathBuf::from(expand_path(p)))?;
        let services = svc
            .entries
            .into_iter()
            .filter_map(|entry| {
                Some(OverlayServiceEntry {
                    name: entry.name?,
                    dir: entry.dir?,
                    health_url: entry.health_url,
                    restart: entry.restart,
                    open_url: entry.open_url,
                })
            })
            .collect();
        Some(OverlayDirConfig {
            label: client_label.clone(),
            base_path,
            services,
            groups,
        })
    });

    Some(ClientOverlay {
        client_dir: client_dir.to_path_buf(),
        label: client_label,
        cwd_patterns,
        cwd_match_count,
        plan_root,
        plan_draft,
        dir_config,
    })
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn resolve_skillbox_config_root() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("SWIMMERS_SKILLBOX_CONFIG") {
        let path = PathBuf::from(expand_path(&val));
        if path.is_dir() {
            return Some(path);
        }
    }
    // Default: ~/repos/skillbox-config
    let home = dirs::home_dir()?;
    let default = home.join("repos").join("skillbox-config");
    if default.is_dir() {
        Some(default)
    } else {
        None
    }
}

/// Expand a group dir entry into concrete filesystem paths.
///
/// Supports a single `*` wildcard matching one path component (e.g.
/// `~/repos/*/.claude/skills` or `~/projects/*/skills`). Literal paths
/// (no `*`) are returned as-is if they exist as directories.
fn expand_group_dir(raw: &str) -> Vec<PathBuf> {
    let expanded = expand_path(raw);
    if !expanded.contains('*') {
        let path = PathBuf::from(expanded);
        return if path.is_dir() {
            vec![path]
        } else {
            Vec::new()
        };
    }

    let Some(star_idx) = expanded.find('*') else {
        return Vec::new();
    };
    let before = &expanded[..star_idx];
    let after = &expanded[star_idx + 1..];

    // Only support full-component wildcards: the `*` must be bounded by `/`
    // on both sides (or be at the start/end of the string).
    if !before.is_empty() && !before.ends_with('/') {
        return Vec::new();
    }
    if !after.is_empty() && !after.starts_with('/') {
        return Vec::new();
    }
    // Reject multi-star patterns for now.
    if after.contains('*') {
        return Vec::new();
    }

    let root = PathBuf::from(before.trim_end_matches('/'));
    let suffix = after.trim_start_matches('/').to_string();

    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut results: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let candidate = if suffix.is_empty() {
            entry.path()
        } else {
            entry.path().join(&suffix)
        };
        if candidate.is_dir() {
            results.push(candidate);
        }
    }
    results.sort();
    results
}

/// Expand `~` and `${VAR}` in path strings.
fn expand_path(path: &str) -> String {
    let mut result = path.to_string();

    // Expand ~ at start
    if result.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            result = format!("{}{}", home.display(), &result[1..]);
        }
    }

    // Expand ${VAR} patterns
    while let Some(start) = result.find("${") {
        let Some(end) = result[start..].find('}') else {
            break;
        };
        let var_name = &result[start + 2..start + end];
        let replacement = std::env::var(var_name).unwrap_or_default();
        result = format!(
            "{}{}{}",
            &result[..start],
            replacement,
            &result[start + end + 1..]
        );
    }

    result
}

fn normalize_path(path: &str) -> String {
    expand_path(path)
}

/// Check if CWD starts with a pattern path (prefix match on path components).
fn cwd_starts_with(cwd: &str, pattern: &str) -> bool {
    let cwd = cwd.trim_end_matches('/');
    let pattern = pattern.trim_end_matches('/');
    // Reject empty or unresolved patterns
    if pattern.is_empty() || pattern.contains("${") {
        return false;
    }
    cwd == pattern || cwd.starts_with(&format!("{pattern}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    fn set_mtime(path: &Path, when: SystemTime) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for mtime");
        file.set_modified(when).expect("set_modified");
    }

    #[test]
    fn cwd_starts_with_exact_match() {
        assert!(cwd_starts_with(
            "/Users/b/repos/htma",
            "/Users/b/repos/htma"
        ));
    }

    #[test]
    fn cwd_starts_with_child_dir() {
        assert!(cwd_starts_with(
            "/Users/b/repos/htma/src/data",
            "/Users/b/repos/htma"
        ));
    }

    #[test]
    fn cwd_starts_with_rejects_partial_name() {
        assert!(!cwd_starts_with(
            "/Users/b/repos/htma_server",
            "/Users/b/repos/htma"
        ));
    }

    #[test]
    fn expand_tilde() {
        let expanded = expand_path("~/repos/foo");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("/repos/foo"));
    }

    #[test]
    fn expand_group_dir_literal_passthrough() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let literal = tmp.path().join("alpha");
        std::fs::create_dir_all(&literal).expect("alpha");
        let results = expand_group_dir(literal.to_str().unwrap());
        assert_eq!(results, vec![literal]);
    }

    #[test]
    fn expand_group_dir_literal_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("does-not-exist");
        let results = expand_group_dir(missing.to_str().unwrap());
        assert!(results.is_empty());
    }

    #[test]
    fn expand_group_dir_single_star_with_suffix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("repo-a").join(".claude").join("skills"))
            .expect("repo-a skills");
        std::fs::create_dir_all(tmp.path().join("repo-b").join(".claude").join("skills"))
            .expect("repo-b skills");
        // A sibling without the suffix should be ignored.
        std::fs::create_dir_all(tmp.path().join("repo-c")).expect("repo-c");
        // A file (not a dir) at the wildcard level should be ignored.
        std::fs::write(tmp.path().join("not-a-dir"), "x").expect("file");

        let pattern = format!("{}/*/.claude/skills", tmp.path().display());
        let results = expand_group_dir(&pattern);

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|p| p.ends_with("repo-a/.claude/skills")));
        assert!(results.iter().any(|p| p.ends_with("repo-b/.claude/skills")));
    }

    #[test]
    fn expand_group_dir_single_star_projects_skills() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("alpha").join("skills")).expect("alpha skills");
        std::fs::create_dir_all(tmp.path().join("beta").join("skills")).expect("beta skills");
        std::fs::create_dir_all(tmp.path().join("gamma")).expect("gamma no-skills");

        let pattern = format!("{}/*/skills", tmp.path().display());
        let results = expand_group_dir(&pattern);

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|p| p.ends_with("alpha/skills")));
        assert!(results.iter().any(|p| p.ends_with("beta/skills")));
    }

    #[test]
    fn list_all_plans_sorts_by_mtime_desc() {
        use std::time::Duration;
        let tmp = tempfile::tempdir().expect("tempdir");
        let client_dir = tmp.path().join("clients").join("personal");
        let released = client_dir.join("plans").join("released");
        let draft = client_dir.join("plans").join("draft");
        std::fs::create_dir_all(released.join("older_plan")).unwrap();
        std::fs::create_dir_all(released.join("newest_plan")).unwrap();
        std::fs::create_dir_all(draft.join("draft_plan")).unwrap();
        let older_schema = released.join("older_plan").join("schema.mmd");
        let newest_schema = released.join("newest_plan").join("schema.mmd");
        let draft_schema = draft.join("draft_plan").join("schema.mmd");
        std::fs::write(&older_schema, "older").unwrap();
        std::fs::write(&newest_schema, "newest").unwrap();
        std::fs::write(&draft_schema, "draft").unwrap();
        // Stamp mtimes so the sort order is deterministic without relying on
        // fs precision or write-order side-effects.
        let now = SystemTime::now();
        let earlier = now - Duration::from_secs(3600);
        let oldest = earlier - Duration::from_secs(3600);
        set_mtime(&older_schema, oldest);
        set_mtime(&newest_schema, now);
        set_mtime(&draft_schema, earlier);

        let client = ClientOverlay {
            client_dir: client_dir.clone(),
            label: "personal".to_string(),
            cwd_patterns: Vec::new(),
            cwd_match_count: 0,
            plan_root: Some(released.clone()),
            plan_draft: Some(draft.clone()),
            dir_config: None,
        };
        let overlay = SkillboxOverlay {
            clients: vec![client],
        };
        let plans = overlay.list_all_plans();
        assert_eq!(
            plans.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(),
            vec!["newest_plan", "draft_plan", "older_plan"]
        );
        assert_eq!(plans[0].kind, "released");
        assert_eq!(plans[1].kind, "draft");
        assert!(plans.iter().all(|p| p.client_label == "personal"));
    }

    #[test]
    fn list_all_plans_skips_archived_and_missing_schema() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client_dir = tmp.path().join("clients").join("personal");
        let released = client_dir.join("plans").join("released");
        std::fs::create_dir_all(released.join("live_plan")).unwrap();
        std::fs::write(released.join("live_plan").join("schema.mmd"), "ok").unwrap();
        // No schema.mmd → skipped.
        std::fs::create_dir_all(released.join("no_schema")).unwrap();
        // "archived" in path → skipped even with schema.mmd.
        let archived = client_dir.join("plans").join("archived").join("stale_plan");
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::write(archived.join("schema.mmd"), "stale").unwrap();

        let client = ClientOverlay {
            client_dir: client_dir.clone(),
            label: "personal".to_string(),
            cwd_patterns: Vec::new(),
            cwd_match_count: 0,
            plan_root: Some(released),
            plan_draft: Some(client_dir.join("plans").join("archived")),
            dir_config: None,
        };
        let overlay = SkillboxOverlay {
            clients: vec![client],
        };
        let plans = overlay.list_all_plans();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].slug, "live_plan");
    }

    #[test]
    fn expand_group_dir_rejects_partial_component_wildcard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("alpha-one")).expect("alpha-one");
        let pattern = format!("{}/alpha-*", tmp.path().display());
        let results = expand_group_dir(&pattern);
        assert!(
            results.is_empty(),
            "partial-component wildcards are not supported: {:?}",
            results
        );
    }

    #[test]
    fn expand_group_dir_rejects_multi_star() {
        let pattern = "/tmp/*/*/skills";
        let results = expand_group_dir(pattern);
        assert!(results.is_empty());
    }
}
