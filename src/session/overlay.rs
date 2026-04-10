use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
}

struct ClientOverlay {
    #[allow(dead_code)] // TODO: re-evaluate when overlay debug/export paths are wired up
    client_dir: PathBuf,
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

    let dir_config = file
        .dev_sanity
        .and_then(|ds| ds.services)
        .and_then(|svc| {
            let base_path = svc.base_path.as_deref().map(|p| PathBuf::from(expand_path(p)))?;
            let services = svc
                .entries
                .into_iter()
                .filter_map(|entry| {
                    Some(OverlayServiceEntry {
                        name: entry.name?,
                        dir: entry.dir?,
                        health_url: entry.health_url,
                        restart: entry.restart,
                    })
                })
                .collect();
            Some(OverlayDirConfig {
                label: client_label.clone(),
                base_path,
                services,
            })
        });

    Some(ClientOverlay {
        client_dir: client_dir.to_path_buf(),
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
}
