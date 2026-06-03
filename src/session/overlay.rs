use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::types::{DependencyHealthSnapshot, LaunchPathMapping, LaunchTargetSummary};

/// Cached skillbox overlay, loaded once from disk.
pub fn default_overlay() -> Option<&'static SkillboxOverlay> {
    static OVERLAY: OnceLock<Option<SkillboxOverlay>> = OnceLock::new();
    OVERLAY.get_or_init(SkillboxOverlay::load).as_ref()
}

pub fn default_overlay_health() -> DependencyHealthSnapshot {
    let now = Utc::now();
    default_overlay().map_or_else(
        || DependencyHealthSnapshot::unavailable(now, "skillbox overlay unavailable"),
        SkillboxOverlay::health_snapshot,
    )
}

pub fn remote_targets_health() -> DependencyHealthSnapshot {
    let now = Utc::now();
    default_overlay().map_or_else(
        || {
            DependencyHealthSnapshot::unknown(now)
                .with_detail("configured_targets", "unknown")
                .with_detail("probe", "overlay_unavailable")
        },
        SkillboxOverlay::remote_targets_health_snapshot,
    )
}

pub struct SkillboxOverlay {
    clients: Vec<ClientOverlay>,
    loaded_at: DateTime<Utc>,
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
    /// Exact directories that become group entries.
    pub paths: Vec<PathBuf>,
    /// Source directories whose immediate children become group entries.
    pub dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct OverlayLaunchConfig {
    pub default_target: String,
    pub targets: Vec<LaunchTargetSummary>,
    pub group_defaults: BTreeMap<String, String>,
}

impl OverlayLaunchConfig {
    pub fn local_only() -> Self {
        Self {
            default_target: "local".to_string(),
            targets: vec![LaunchTargetSummary::local()],
            group_defaults: BTreeMap::new(),
        }
    }

    pub fn default_for_group(&self, group: Option<&str>) -> String {
        group
            .and_then(|name| self.group_defaults.get(name))
            .cloned()
            .unwrap_or_else(|| self.default_target.clone())
    }
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
    /// Agent launch targets/defaults declared by the overlay.
    pub launch: OverlayLaunchConfig,
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
        let clients = load_client_overlays(&config_root)?;
        if clients.is_empty() {
            None
        } else {
            Some(Self {
                clients,
                loaded_at: Utc::now(),
            })
        }
    }

    pub fn health_snapshot(&self) -> DependencyHealthSnapshot {
        let now = Utc::now();
        DependencyHealthSnapshot::healthy(now)
            .with_last_seen(self.loaded_at)
            .with_detail("client_count", self.clients.len().to_string())
            .with_detail(
                "launch_target_count",
                self.all_launch_targets().len().to_string(),
            )
    }

    pub fn remote_targets_health_snapshot(&self) -> DependencyHealthSnapshot {
        let now = Utc::now();
        let configured_targets = self
            .all_launch_targets()
            .into_iter()
            .filter(|target| target.kind == "swimmers_api")
            .count();

        if configured_targets == 0 {
            return DependencyHealthSnapshot::not_configured(now)
                .with_last_seen(self.loaded_at)
                .with_detail("configured_targets", "0");
        }

        DependencyHealthSnapshot::unknown(now)
            .with_last_seen(self.loaded_at)
            .with_detail("configured_targets", configured_targets.to_string())
            .with_detail("probe", "not_run_by_health")
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

    pub fn launch_target_by_id(&self, id: &str) -> Option<LaunchTargetSummary> {
        self.clients
            .iter()
            .filter_map(|client| client.dir_config.as_ref())
            .flat_map(|config| config.launch.targets.iter())
            .find(|target| target.id == id)
            .cloned()
    }

    pub fn launch_target_for_cwd(&self, cwd: &str, id: &str) -> Option<LaunchTargetSummary> {
        self.find_dir_config(cwd)?
            .launch
            .targets
            .iter()
            .find(|target| target.id == id)
            .cloned()
    }

    pub fn all_launch_targets(&self) -> Vec<LaunchTargetSummary> {
        let mut targets = Vec::new();
        for target in self
            .clients
            .iter()
            .filter_map(|client| client.dir_config.as_ref())
            .flat_map(|config| config.launch.targets.iter())
        {
            if !targets
                .iter()
                .any(|existing: &LaunchTargetSummary| existing.id == target.id)
            {
                targets.push(target.clone());
            }
        }
        targets
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

fn load_client_overlays(config_root: &Path) -> Option<Vec<ClientOverlay>> {
    let clients_dir = config_root.join("clients");
    if !clients_dir.is_dir() {
        return None;
    }

    client_overlay_paths(&clients_dir).map(|paths| {
        paths
            .into_iter()
            .filter_map(|(client_dir, overlay_path)| {
                parse_client_overlay(&client_dir, &overlay_path)
            })
            .collect()
    })
}

fn client_overlay_paths(clients_dir: &Path) -> Option<Vec<(PathBuf, PathBuf)>> {
    let entries = std::fs::read_dir(clients_dir).ok()?;
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let client_dir = entry.path();
        if let Some(overlay_path) = overlay_file_in_client_dir(&client_dir) {
            paths.push((client_dir, overlay_path));
        }
    }
    Some(paths)
}

fn overlay_file_in_client_dir(client_dir: &Path) -> Option<PathBuf> {
    if !client_dir.is_dir() {
        return None;
    }
    let overlay_path = client_dir.join("overlay.yaml");
    overlay_path.is_file().then_some(overlay_path)
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
    #[serde(default)]
    repos: Vec<ClientRepoEntry>,
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
    scan_roots: Vec<String>,
    #[serde(default)]
    repos: Vec<RepoEntry>,
}

#[derive(Deserialize, Default)]
struct RepoEntry {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize, Default)]
struct ClientRepoEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    repo_path: Option<String>,
}

#[derive(Deserialize, Default)]
struct DevSanitySection {
    #[serde(default)]
    agent_launch: Option<DevSanityAgentLaunch>,
    #[serde(default)]
    services: Option<DevSanityServices>,
    #[serde(default)]
    groups: Vec<DevSanityGroup>,
}

#[derive(Deserialize, Default)]
struct DevSanityAgentLaunch {
    #[serde(default)]
    default_target: Option<String>,
    #[serde(default)]
    targets: Vec<DevSanityLaunchTarget>,
    #[serde(default)]
    group_defaults: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct DevSanityLaunchTarget {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    auth_token_env: Option<String>,
    #[serde(default)]
    path_mappings: Vec<DevSanityLaunchPathMapping>,
}

#[derive(Deserialize, Default)]
struct DevSanityLaunchPathMapping {
    #[serde(default)]
    local_prefix: Option<String>,
    #[serde(default)]
    remote_prefix: Option<String>,
}

#[derive(Deserialize, Default)]
struct DevSanityGroup {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
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
    let file = read_overlay_file(overlay_path)?;
    let client = file.client?;
    let client_label = resolve_client_label(&client, client_dir);
    let client_repos = client.repos;
    let context = client.context?;
    let cwd_match_count = context.cwd_match.len();
    let cwd_patterns = cwd_patterns_from_context(&context);
    let scan_roots = scan_roots_from_context(&context);
    let (plan_root, plan_draft) = plan_dirs_from_context(client_dir, context.plans);
    let dir_config = file
        .dev_sanity
        .and_then(|ds| parse_dir_config(ds, &client_label, client_repos, &scan_roots));

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

fn read_overlay_file(overlay_path: &Path) -> Option<OverlayFile> {
    let content = std::fs::read_to_string(overlay_path).ok()?;
    serde_yaml::from_str(&content).ok()
}

fn resolve_client_label(client: &OverlayClient, client_dir: &Path) -> String {
    client
        .label
        .clone()
        .or_else(|| client.id.clone())
        .unwrap_or_else(|| fallback_client_label(client_dir))
}

fn fallback_client_label(client_dir: &Path) -> String {
    client_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "overlay".to_string())
}

fn cwd_patterns_from_context(context: &OverlayContext) -> Vec<String> {
    let mut patterns: Vec<String> = context.cwd_match.iter().map(|p| expand_path(p)).collect();
    if let Some(landscape) = &context.repo_landscape {
        patterns.extend(landscape.scan_roots.iter().map(|root| expand_path(root)));
        patterns.extend(
            landscape
                .repos
                .iter()
                .filter_map(|repo| repo.path.as_deref())
                .map(expand_path),
        );
    }
    patterns
}

fn scan_roots_from_context(context: &OverlayContext) -> Vec<PathBuf> {
    context
        .repo_landscape
        .as_ref()
        .map(|landscape| {
            landscape
                .scan_roots
                .iter()
                .map(|root| PathBuf::from(expand_path(root)))
                .collect()
        })
        .unwrap_or_default()
}

fn plan_dirs_from_context(
    client_dir: &Path,
    plans: Option<OverlayPlans>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let plan_root = plans
        .as_ref()
        .and_then(|p| p.plan_root.as_deref())
        .map(|rel| client_dir.join(rel));
    let plan_draft = plans
        .as_ref()
        .and_then(|p| p.plan_draft.as_deref())
        .map(|rel| client_dir.join(rel));
    (plan_root, plan_draft)
}

fn parse_dir_config(
    section: DevSanitySection,
    client_label: &str,
    client_repos: Vec<ClientRepoEntry>,
    scan_roots: &[PathBuf],
) -> Option<OverlayDirConfig> {
    let launch = parse_agent_launch(section.agent_launch);
    let groups = parse_dir_groups(section.groups);
    let services = section.services?;
    let base_path = services
        .base_path
        .as_deref()
        .map(|p| PathBuf::from(expand_path(p)))?;
    let services = parse_services(services.entries, client_repos, scan_roots, &base_path);

    Some(OverlayDirConfig {
        label: client_label.to_string(),
        base_path,
        services,
        groups,
        launch,
    })
}

fn parse_dir_groups(groups: Vec<DevSanityGroup>) -> Vec<OverlayDirGroup> {
    groups.into_iter().filter_map(parse_dir_group).collect()
}

fn parse_dir_group(group: DevSanityGroup) -> Option<OverlayDirGroup> {
    let name = group.name?;
    let mut paths = Vec::new();
    let mut dirs = Vec::new();
    let mut seen = BTreeSet::new();

    extend_unique_paths(
        &mut paths,
        &mut seen,
        group.paths.iter(),
        expand_exact_group_path,
    );
    extend_unique_paths(&mut dirs, &mut seen, group.dirs.iter(), expand_group_dir);

    if paths.is_empty() && dirs.is_empty() {
        return None;
    }
    Some(OverlayDirGroup { name, paths, dirs })
}

fn extend_unique_paths<'a, I, F>(
    output: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    raw_paths: I,
    expand: F,
) where
    I: IntoIterator<Item = &'a String>,
    F: Fn(&str) -> Vec<PathBuf>,
{
    for raw in raw_paths {
        for path in expand(raw) {
            if seen.insert(path.clone()) {
                output.push(path);
            }
        }
    }
}

fn parse_services(
    entries: Vec<DevSanityServiceEntry>,
    client_repos: Vec<ClientRepoEntry>,
    scan_roots: &[PathBuf],
    base_path: &Path,
) -> Vec<OverlayServiceEntry> {
    let mut seen_dirs = BTreeSet::new();
    let mut services: Vec<OverlayServiceEntry> = entries
        .into_iter()
        .filter_map(parse_service_entry)
        .inspect(|entry| {
            seen_dirs.insert(entry.dir.clone());
        })
        .collect();

    append_client_repo_services(&mut services, &mut seen_dirs, client_repos, base_path);
    append_scan_root_services(&mut services, &mut seen_dirs, scan_roots, base_path);
    services
}

fn parse_service_entry(entry: DevSanityServiceEntry) -> Option<OverlayServiceEntry> {
    Some(OverlayServiceEntry {
        name: entry.name?,
        dir: expand_path(&entry.dir?),
        health_url: entry.health_url,
        restart: entry.restart,
        open_url: entry.open_url,
    })
}

fn append_client_repo_services(
    services: &mut Vec<OverlayServiceEntry>,
    seen_dirs: &mut BTreeSet<String>,
    client_repos: Vec<ClientRepoEntry>,
    base_path: &Path,
) {
    for repo in client_repos {
        append_service_if_new(
            services,
            seen_dirs,
            service_entry_from_client_repo(repo, base_path),
        );
    }
}

fn append_scan_root_services(
    services: &mut Vec<OverlayServiceEntry>,
    seen_dirs: &mut BTreeSet<String>,
    scan_roots: &[PathBuf],
    base_path: &Path,
) {
    for root in scan_roots {
        for entry in service_entries_from_scan_root(root, base_path) {
            append_service_if_new(services, seen_dirs, Some(entry));
        }
    }
}

fn append_service_if_new(
    services: &mut Vec<OverlayServiceEntry>,
    seen_dirs: &mut BTreeSet<String>,
    entry: Option<OverlayServiceEntry>,
) {
    let Some(entry) = entry else {
        return;
    };
    if seen_dirs.insert(entry.dir.clone()) {
        services.push(entry);
    }
}

fn service_entries_from_scan_root(root: &Path, base_path: &Path) -> Vec<OverlayServiceEntry> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let base = base_path
        .canonicalize()
        .unwrap_or_else(|_| base_path.to_path_buf());
    if root == base || root.starts_with(&base) {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut services = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name.starts_with('.') || !path.join(".git").is_dir() {
            continue;
        }
        if let Some(service) = service_entry_from_repo_path(None, path, base_path) {
            services.push(service);
        }
    }
    services.sort_by(|a, b| a.name.cmp(&b.name));
    services
}

fn parse_agent_launch(section: Option<DevSanityAgentLaunch>) -> OverlayLaunchConfig {
    let Some(section) = section else {
        return OverlayLaunchConfig::local_only();
    };

    let mut targets: Vec<LaunchTargetSummary> = section
        .targets
        .into_iter()
        .filter_map(parse_launch_target)
        .collect();
    ensure_local_launch_target(&mut targets);
    let default_target = valid_default_target(section.default_target, &targets);
    let group_defaults = valid_group_defaults(section.group_defaults, &targets);

    OverlayLaunchConfig {
        default_target,
        targets,
        group_defaults,
    }
}

fn parse_launch_target(target: DevSanityLaunchTarget) -> Option<LaunchTargetSummary> {
    let id = target.id?;
    Some(LaunchTargetSummary {
        label: target.label.unwrap_or_else(|| id.clone()),
        kind: target.kind.unwrap_or_else(|| "local".to_string()),
        id,
        base_url: target.base_url,
        auth_token_env: target.auth_token_env,
        path_mappings: parse_launch_path_mappings(target.path_mappings),
    })
}

fn parse_launch_path_mappings(mappings: Vec<DevSanityLaunchPathMapping>) -> Vec<LaunchPathMapping> {
    mappings
        .into_iter()
        .filter_map(parse_launch_path_mapping)
        .collect()
}

fn parse_launch_path_mapping(mapping: DevSanityLaunchPathMapping) -> Option<LaunchPathMapping> {
    Some(LaunchPathMapping {
        local_prefix: expand_path(&mapping.local_prefix?),
        remote_prefix: expand_path(&mapping.remote_prefix?),
    })
}

fn ensure_local_launch_target(targets: &mut Vec<LaunchTargetSummary>) {
    if !target_exists(targets, "local") {
        targets.insert(0, LaunchTargetSummary::local());
    }
}

fn valid_default_target(default_target: Option<String>, targets: &[LaunchTargetSummary]) -> String {
    default_target
        .filter(|target| target_exists(targets, target))
        .unwrap_or_else(|| "local".to_string())
}

fn valid_group_defaults(
    group_defaults: BTreeMap<String, String>,
    targets: &[LaunchTargetSummary],
) -> BTreeMap<String, String> {
    group_defaults
        .into_iter()
        .filter(|(_, target)| target_exists(targets, target))
        .collect()
}

fn target_exists(targets: &[LaunchTargetSummary], id: &str) -> bool {
    targets.iter().any(|target| target.id == id)
}

fn service_entry_from_client_repo(
    repo: ClientRepoEntry,
    base_path: &Path,
) -> Option<OverlayServiceEntry> {
    if repo.kind.as_deref().is_some_and(|kind| kind != "repo") {
        return None;
    }

    let repo_path = expand_repo_path(&repo.repo_path?, base_path);
    service_entry_from_repo_path(repo.id, repo_path, base_path)
}

fn service_entry_from_repo_path(
    id: Option<String>,
    repo_path: PathBuf,
    base_path: &Path,
) -> Option<OverlayServiceEntry> {
    let dir = relative_dir_from_base(base_path, &repo_path)
        .unwrap_or_else(|| repo_path.to_string_lossy().into_owned());
    let name = id.or_else(|| {
        repo_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    })?;

    Some(OverlayServiceEntry {
        name,
        dir,
        health_url: None,
        restart: None,
        open_url: None,
    })
}

fn expand_repo_path(raw: &str, base_path: &Path) -> PathBuf {
    if let Some(suffix) = raw.strip_prefix("${SKILLBOX_MONOSERVER_ROOT}/") {
        if std::env::var_os("SKILLBOX_MONOSERVER_ROOT").is_none() {
            return base_path.join(suffix);
        }
    }

    PathBuf::from(expand_path(raw))
}

fn relative_dir_from_base(base_path: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(base_path).ok()?;
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    (!components.is_empty()).then(|| components.join("/"))
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
    expand_existing_dirs(raw)
}

/// Expand an exact group entry into concrete filesystem paths.
///
/// This currently shares the same path expansion rules as `dirs`; the
/// difference is semantic: `paths` become entries themselves, while `dirs`
/// contribute their immediate children.
fn expand_exact_group_path(raw: &str) -> Vec<PathBuf> {
    expand_existing_dirs(raw)
}

fn expand_existing_dirs(raw: &str) -> Vec<PathBuf> {
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
///
/// Substitution is forward-only: each `${VAR}` slot is resolved exactly once,
/// and its replacement is not rescanned. That keeps the function terminating
/// even when an env var resolves to text containing another `${VAR}` (or even
/// itself).
fn expand_path(path: &str) -> String {
    let mut result = path.to_string();

    if result.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            result = format!("{}{}", home.display(), &result[1..]);
        }
    }

    let mut search_from = 0usize;
    while let Some(rel_start) = result[search_from..].find("${") {
        let start = search_from + rel_start;
        let Some(rel_end) = result[start..].find('}') else {
            break;
        };
        let end = start + rel_end;
        let var_name = &result[start + 2..end];
        let replacement = std::env::var(var_name).unwrap_or_default();
        let suffix = result[end + 1..].to_string();
        result.truncate(start);
        result.push_str(&replacement);
        // Advance past the inserted text so it is not re-scanned.
        search_from = result.len();
        result.push_str(&suffix);
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
        assert!(cwd_starts_with("/tmp/repos/example", "/tmp/repos/example"));
    }

    #[test]
    fn cwd_starts_with_child_dir() {
        assert!(cwd_starts_with(
            "/tmp/repos/example/src/data",
            "/tmp/repos/example"
        ));
    }

    #[test]
    fn cwd_starts_with_rejects_partial_name() {
        assert!(!cwd_starts_with(
            "/tmp/repos/example_server",
            "/tmp/repos/example"
        ));
    }

    #[test]
    fn expand_tilde() {
        let expanded = expand_path("~/repos/foo");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("/repos/foo"));
    }

    #[test]
    fn expand_path_terminates_when_env_var_resolves_to_self_referential_text() {
        // Regression: the previous implementation re-scanned from offset 0
        // after each substitution, so an env var that expanded to text
        // containing the same `${VAR}` reference would loop forever.
        let key = "SWIMMERS_EXPAND_PATH_RECURSIVE_TEST";
        let prior = std::env::var(key).ok();
        std::env::set_var(key, format!("${{{key}}}/x"));

        let expanded = expand_path(&format!("${{{key}}}/y"));

        match prior {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }

        // The first expansion is the only one performed; the inserted
        // `${VAR}` is treated as literal text, not re-resolved.
        assert_eq!(expanded, format!("${{{key}}}/x/y"));
    }

    #[test]
    fn expand_repo_path_falls_back_to_base_for_unset_monoserver_root() {
        let key = "SKILLBOX_MONOSERVER_ROOT";
        let prior = std::env::var(key).ok();
        std::env::remove_var(key);

        let base = PathBuf::from("/tmp/repos");
        let expanded = expand_repo_path("${SKILLBOX_MONOSERVER_ROOT}/voice-to-text", &base);

        match prior {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }

        assert_eq!(expanded, base.join("voice-to-text"));
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
    fn load_client_overlays_returns_none_when_clients_dir_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(load_client_overlays(tmp.path()).is_none());
    }

    #[test]
    fn load_client_overlays_returns_empty_when_clients_dir_has_no_overlay_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let clients_dir = tmp.path().join("clients");
        std::fs::create_dir_all(clients_dir.join("empty-client")).expect("client dir");
        std::fs::write(clients_dir.join("not-a-client"), "x").expect("file");

        let clients = load_client_overlays(tmp.path()).expect("scan clients");

        assert!(clients.is_empty());
    }

    #[test]
    fn parse_agent_launch_injects_local_and_filters_unknown_defaults() {
        let mut group_defaults = BTreeMap::new();
        group_defaults.insert("known".to_string(), "remote".to_string());
        group_defaults.insert("unknown".to_string(), "missing".to_string());

        let launch = parse_agent_launch(Some(DevSanityAgentLaunch {
            default_target: Some("missing".to_string()),
            targets: vec![DevSanityLaunchTarget {
                id: Some("remote".to_string()),
                label: None,
                kind: Some("swimmers_api".to_string()),
                base_url: Some("http://remote.test:3210".to_string()),
                auth_token_env: Some("REMOTE_TOKEN".to_string()),
                path_mappings: vec![DevSanityLaunchPathMapping {
                    local_prefix: Some("/local".to_string()),
                    remote_prefix: Some("/remote".to_string()),
                }],
            }],
            group_defaults,
        }));

        assert_eq!(launch.default_target, "local");
        assert_eq!(
            launch
                .targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local", "remote"]
        );
        assert_eq!(launch.default_for_group(Some("known")), "remote");
        assert_eq!(launch.default_for_group(Some("unknown")), "local");

        let remote = launch
            .targets
            .iter()
            .find(|target| target.id == "remote")
            .expect("remote target");
        assert_eq!(remote.label, "remote");
        assert_eq!(remote.kind, "swimmers_api");
        assert_eq!(remote.path_mappings[0].local_prefix, "/local");
        assert_eq!(remote.path_mappings[0].remote_prefix, "/remote");
    }

    #[test]
    fn parse_client_overlay_adds_client_repos_to_dir_config_services() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client_dir = tmp.path().join("clients").join("personal");
        std::fs::create_dir_all(&client_dir).expect("client dir");
        let repo_base = tmp.path().join("repos");
        std::fs::create_dir_all(&repo_base).expect("repo base");
        std::fs::create_dir_all(repo_base.join("finalreceipts")).expect("finalreceipts repo");
        std::fs::create_dir_all(repo_base.join("sweet-potato")).expect("sweet-potato repo");
        let hard_root = tmp.path().join("hard");
        let hard_repo = hard_root.join("mmd-pcb");
        std::fs::create_dir_all(&hard_repo).expect("hard repo");
        let scanned_hard_repo = hard_root.join("pcbcd");
        std::fs::create_dir_all(scanned_hard_repo.join(".git")).expect("scanned hard repo");
        let overlay_path = client_dir.join("overlay.yaml");
        std::fs::write(
            &overlay_path,
            format!(
                r#"
version: 1
client:
  id: personal
  context:
    cwd_match:
      - {repo_base}
    repo_landscape:
      scan_roots:
        - {hard_root}
  repos:
    - id: finalreceipts
      kind: repo
      repo_path: {repo_base}/finalreceipts
    - id: sweet-potato-dupe
      kind: repo
      repo_path: {repo_base}/sweet-potato
    - id: mmd-pcb
      kind: repo
      repo_path: {hard_repo}
dev_sanity:
  services:
    base_path: {repo_base}
    entries:
      - name: spaps
        dir: sweet-potato
        health_url: http://localhost:3301
  groups:
    - name: frontend
      paths:
        - {repo_base}/finalreceipts
"#,
                hard_repo = hard_repo.display(),
                hard_root = hard_root.display(),
                repo_base = repo_base.display()
            ),
        )
        .expect("write overlay");

        let client = parse_client_overlay(&client_dir, &overlay_path).expect("parse overlay");
        let config = client.dir_config.expect("dir config");
        let service_dirs: Vec<&str> = config
            .services
            .iter()
            .map(|service| service.dir.as_str())
            .collect();

        let scanned_hard_path = scanned_hard_repo
            .canonicalize()
            .expect("canonical scanned hard path")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            service_dirs,
            vec![
                "sweet-potato",
                "finalreceipts",
                hard_repo.to_str().expect("hard path"),
                scanned_hard_path.as_str()
            ]
        );
        assert!(config
            .services
            .iter()
            .any(|service| service.name == "finalreceipts"));
        assert_eq!(config.groups[0].name, "frontend");
        assert!(config.groups[0]
            .paths
            .iter()
            .any(|path| path.ends_with("finalreceipts")));
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
            client_dir,
            label: "personal".to_string(),
            cwd_patterns: Vec::new(),
            cwd_match_count: 0,
            plan_root: Some(released),
            plan_draft: Some(draft),
            dir_config: None,
        };
        let overlay = SkillboxOverlay {
            clients: vec![client],
            loaded_at: Utc::now(),
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
            loaded_at: Utc::now(),
        };
        let plans = overlay.list_all_plans();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].slug, "live_plan");
    }

    #[test]
    fn overlay_health_reports_load_age_and_remote_target_count_without_probe() {
        let remote = LaunchTargetSummary {
            id: "remote-skillbox".to_string(),
            label: "Remote".to_string(),
            kind: "swimmers_api".to_string(),
            base_url: Some("http://example.test:3210".to_string()),
            auth_token_env: Some("REMOTE_TOKEN".to_string()),
            path_mappings: Vec::new(),
        };
        let client = ClientOverlay {
            client_dir: PathBuf::from("/tmp/overlay_health"),
            label: "health".to_string(),
            cwd_patterns: Vec::new(),
            cwd_match_count: 0,
            plan_root: None,
            plan_draft: None,
            dir_config: Some(OverlayDirConfig {
                label: "health".to_string(),
                base_path: PathBuf::from("/tmp"),
                services: Vec::new(),
                groups: Vec::new(),
                launch: OverlayLaunchConfig {
                    default_target: "local".to_string(),
                    targets: vec![LaunchTargetSummary::local(), remote],
                    group_defaults: BTreeMap::new(),
                },
            }),
        };
        let overlay = SkillboxOverlay {
            clients: vec![client],
            loaded_at: Utc::now() - chrono::Duration::seconds(1),
        };

        let health = overlay.health_snapshot();
        assert_eq!(health.status, crate::types::DependencyHealthStatus::Healthy);
        assert_eq!(health.details["client_count"], "1");
        assert!(health.freshness_ms.is_some());

        let remote = overlay.remote_targets_health_snapshot();
        assert_eq!(remote.status, crate::types::DependencyHealthStatus::Unknown);
        assert_eq!(remote.details["configured_targets"], "1");
        assert_eq!(remote.details["probe"], "not_run_by_health");
        assert!(
            !remote
                .details
                .values()
                .any(|value| value.contains("REMOTE_TOKEN")),
            "health details must not leak token env names or values"
        );
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

    fn find_plan_dirs_overlay(client: ClientOverlay) -> SkillboxOverlay {
        SkillboxOverlay {
            clients: vec![client],
            loaded_at: Utc::now(),
        }
    }

    fn make_plan_client(
        cwd_patterns: Vec<String>,
        cwd_match_count: usize,
        plan_root: Option<PathBuf>,
        plan_draft: Option<PathBuf>,
    ) -> ClientOverlay {
        ClientOverlay {
            client_dir: PathBuf::from("/tmp/find_plan_dirs_test"),
            label: "test".to_string(),
            cwd_patterns,
            cwd_match_count,
            plan_root,
            plan_draft,
            dir_config: None,
        }
    }

    #[test]
    fn find_plan_dirs_returns_none_when_no_client_matches_cwd() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_root = tmp.path().join("plans").join("released");
        std::fs::create_dir_all(&plan_root).unwrap();
        let client = make_plan_client(
            vec!["/some/other/repo".to_string()],
            1,
            Some(plan_root),
            None,
        );
        let overlay = find_plan_dirs_overlay(client);
        assert!(overlay.find_plan_dirs("/unrelated/path").is_none());
    }

    #[test]
    fn find_plan_dirs_skips_multi_repo_clients() {
        // Multi-repo clients (cwd_match_count > 1) span multiple repos so the
        // overlay can't pick a single plan dir set; caller falls back to the
        // in-repo scan.
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_string_lossy().to_string();
        let plan_root = tmp.path().join("plans").join("released");
        std::fs::create_dir_all(&plan_root).unwrap();
        let client = make_plan_client(vec![cwd.clone()], 2, Some(plan_root), None);
        let overlay = find_plan_dirs_overlay(client);
        assert!(overlay.find_plan_dirs(&cwd).is_none());
    }

    #[test]
    fn find_plan_dirs_returns_both_root_and_draft_when_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_string_lossy().to_string();
        let plan_root = tmp.path().join("plans").join("released");
        let plan_draft = tmp.path().join("plans").join("draft");
        std::fs::create_dir_all(&plan_root).unwrap();
        std::fs::create_dir_all(&plan_draft).unwrap();
        let client = make_plan_client(
            vec![cwd.clone()],
            1,
            Some(plan_root.clone()),
            Some(plan_draft.clone()),
        );
        let overlay = find_plan_dirs_overlay(client);
        let dirs = overlay.find_plan_dirs(&cwd).expect("dirs");
        assert_eq!(dirs, vec![plan_root, plan_draft]);
    }

    #[test]
    fn find_plan_dirs_skips_directories_that_do_not_exist_on_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_string_lossy().to_string();
        let real_root = tmp.path().join("plans").join("released");
        std::fs::create_dir_all(&real_root).unwrap();
        // plan_draft points to a path that was never created.
        let missing_draft = tmp.path().join("plans").join("draft");
        let client = make_plan_client(
            vec![cwd.clone()],
            1,
            Some(real_root.clone()),
            Some(missing_draft),
        );
        let overlay = find_plan_dirs_overlay(client);
        let dirs = overlay.find_plan_dirs(&cwd).expect("dirs");
        assert_eq!(dirs, vec![real_root]);
    }

    #[test]
    fn find_plan_dirs_returns_none_when_neither_dir_exists_on_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_string_lossy().to_string();
        let missing_root = tmp.path().join("plans").join("released");
        let missing_draft = tmp.path().join("plans").join("draft");
        let client = make_plan_client(
            vec![cwd.clone()],
            1,
            Some(missing_root),
            Some(missing_draft),
        );
        let overlay = find_plan_dirs_overlay(client);
        assert!(overlay.find_plan_dirs(&cwd).is_none());
    }

    #[test]
    fn find_plan_dirs_matches_cwd_inside_pattern_dir() {
        // cwd_starts_with allows nested directories under the pattern.
        let tmp = tempfile::tempdir().expect("tempdir");
        let pattern = tmp.path().to_string_lossy().to_string();
        let nested = tmp.path().join("nested").join("crate");
        std::fs::create_dir_all(&nested).unwrap();
        let plan_root = tmp.path().join("plans").join("released");
        std::fs::create_dir_all(&plan_root).unwrap();
        let client = make_plan_client(vec![pattern], 1, Some(plan_root.clone()), None);
        let overlay = find_plan_dirs_overlay(client);
        let dirs = overlay
            .find_plan_dirs(&nested.to_string_lossy())
            .expect("dirs");
        assert_eq!(dirs, vec![plan_root]);
    }

    #[test]
    fn find_plan_dirs_returns_none_when_no_plan_paths_configured() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path().to_string_lossy().to_string();
        let client = make_plan_client(vec![cwd.clone()], 1, None, None);
        let overlay = find_plan_dirs_overlay(client);
        assert!(overlay.find_plan_dirs(&cwd).is_none());
    }
}
