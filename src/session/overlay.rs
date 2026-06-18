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
    pub default_target_explicit: bool,
    pub targets: Vec<LaunchTargetSummary>,
    pub group_defaults: BTreeMap<String, String>,
}

impl OverlayLaunchConfig {
    pub fn local_only() -> Self {
        Self {
            default_target: "local".to_string(),
            default_target_explicit: true,
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

    pub fn default_for_group_or_path(&self, group: Option<&str>, path: &Path) -> String {
        if let Some(target) = group.and_then(|name| self.group_defaults.get(name)) {
            return target.clone();
        }
        if self.default_target_explicit {
            return self.default_target.clone();
        }
        best_mapped_launch_target(path, &self.targets)
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

        self.clients
            .iter()
            .find_map(|client| dir_config_matching_base_path(client, &cwd_normalized))
            .or_else(|| {
                self.clients
                    .iter()
                    .find(|client| client_matches_cwd_patterns(client, &cwd_normalized))
                    .and_then(|client| client.dir_config.as_ref())
            })
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
        unique_launch_targets(
            self.clients
                .iter()
                .filter_map(|client| client.dir_config.as_ref())
                .flat_map(|config| config.launch.targets.iter()),
        )
    }

    /// Given a session CWD, find the matching client's plan directories.
    ///
    /// Multi-repo clients (more than one `cwd_match` entry) are skipped —
    /// their plan directories span multiple repos and the caller should
    /// fall back to the in-repo scan instead.
    pub fn find_plan_dirs(&self, cwd: &str) -> Option<Vec<PathBuf>> {
        let cwd_normalized = normalize_path(cwd);
        let client = first_client_matching_cwd(&self.clients, &cwd_normalized)?;
        let client = single_repo_plan_client(client)?;
        existing_plan_dirs(client)
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

fn first_client_matching_cwd<'a>(
    clients: &'a [ClientOverlay],
    cwd_normalized: &str,
) -> Option<&'a ClientOverlay> {
    clients
        .iter()
        .find(|client| client_matches_cwd_patterns(client, cwd_normalized))
}

fn single_repo_plan_client(client: &ClientOverlay) -> Option<&ClientOverlay> {
    (client.cwd_match_count <= 1).then_some(client)
}

fn existing_plan_dirs(client: &ClientOverlay) -> Option<Vec<PathBuf>> {
    let dirs: Vec<_> = [client.plan_root.as_deref(), client.plan_draft.as_deref()]
        .into_iter()
        .flatten()
        .filter(|dir| dir.is_dir())
        .map(Path::to_path_buf)
        .collect();
    (!dirs.is_empty()).then_some(dirs)
}

fn dir_config_matching_base_path<'a>(
    client: &'a ClientOverlay,
    cwd_normalized: &str,
) -> Option<&'a OverlayDirConfig> {
    let config = client.dir_config.as_ref()?;
    dir_config_base_path_contains_cwd(config, cwd_normalized).then_some(config)
}

fn dir_config_base_path_contains_cwd(config: &OverlayDirConfig, cwd_normalized: &str) -> bool {
    let base = config
        .base_path
        .canonicalize()
        .unwrap_or_else(|_| config.base_path.clone());
    let cwd = canonical_or_original(Path::new(cwd_normalized));
    let base_str = base.to_string_lossy();
    let cwd_str = cwd.to_string_lossy();
    cwd_starts_with(cwd_str.as_ref(), base_str.as_ref())
}

fn client_matches_cwd_patterns(client: &ClientOverlay, cwd_normalized: &str) -> bool {
    client
        .cwd_patterns
        .iter()
        .any(|pattern| cwd_starts_with(cwd_normalized, pattern))
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
        if let Some(plan) = overlay_plan_entry_from_dir_entry(entry, client_label, kind) {
            out.push(plan);
        }
    }
}

fn overlay_plan_entry_from_dir_entry(
    entry: std::fs::DirEntry,
    client_label: &str,
    kind: &'static str,
) -> Option<OverlayPlanEntry> {
    if !entry.file_type().ok()?.is_dir() {
        return None;
    }
    overlay_plan_entry_from_dir(entry.path(), client_label, kind)
}

fn overlay_plan_entry_from_dir(
    plan_dir: PathBuf,
    client_label: &str,
    kind: &'static str,
) -> Option<OverlayPlanEntry> {
    let slug = overlay_plan_slug(&plan_dir)?;
    if is_archived_overlay_plan_path(&plan_dir) {
        return None;
    }
    let schema_path = overlay_plan_schema_path(&plan_dir)?;
    let updated_at = plan_dir_latest_mtime(&plan_dir);
    Some(OverlayPlanEntry {
        slug,
        client_label: client_label.to_string(),
        kind,
        schema_path,
        updated_at,
    })
}

fn overlay_plan_slug(plan_dir: &Path) -> Option<String> {
    plan_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn is_archived_overlay_plan_path(plan_dir: &Path) -> bool {
    // Skip any plan dir whose path mentions "archived" anywhere; callers only
    // want live plans.
    plan_dir
        .components()
        .any(|component| component.as_os_str().to_string_lossy().contains("archived"))
}

fn overlay_plan_schema_path(plan_dir: &Path) -> Option<PathBuf> {
    let schema_path = plan_dir.join("schema.mmd");
    schema_path.is_file().then_some(schema_path)
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
            mark_service_dir_seen(&mut seen_dirs, &entry.dir);
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
    append_services_if_new(
        services,
        seen_dirs,
        scan_roots
            .iter()
            .flat_map(|root| service_entries_from_scan_root(root, base_path)),
    );
}

fn append_service_if_new(
    services: &mut Vec<OverlayServiceEntry>,
    seen_dirs: &mut BTreeSet<String>,
    entry: Option<OverlayServiceEntry>,
) {
    let Some(entry) = entry else {
        return;
    };
    if service_dir_seen(seen_dirs, &entry.dir) {
        return;
    }
    mark_service_dir_seen(seen_dirs, &entry.dir);
    services.push(entry);
}

fn append_services_if_new<I>(
    services: &mut Vec<OverlayServiceEntry>,
    seen_dirs: &mut BTreeSet<String>,
    entries: I,
) where
    I: IntoIterator<Item = OverlayServiceEntry>,
{
    for entry in entries {
        append_service_if_new(services, seen_dirs, Some(entry));
    }
}

fn service_entries_from_scan_root(root: &Path, base_path: &Path) -> Vec<OverlayServiceEntry> {
    let canonical = canonical_scan_root_paths(root, base_path);
    if !scan_root_is_outside_base(&canonical.root, &canonical.base) {
        return Vec::new();
    }

    collect_sorted_service_entries(repo_dirs_in_scan_root(root), base_path)
}

fn service_dir_seen(seen_dirs: &BTreeSet<String>, dir: &str) -> bool {
    service_dir_seen_keys(dir)
        .iter()
        .any(|key| seen_dirs.contains(key))
}

fn mark_service_dir_seen(seen_dirs: &mut BTreeSet<String>, dir: &str) {
    for key in service_dir_seen_keys(dir) {
        seen_dirs.insert(key);
    }
}

fn service_dir_seen_keys(dir: &str) -> Vec<String> {
    let mut keys = vec![dir.to_string()];
    let path = Path::new(dir);
    if path.is_absolute() {
        let canonical = canonical_or_original(path).to_string_lossy().into_owned();
        if canonical != dir {
            keys.push(canonical);
        }
    }
    keys
}

struct CanonicalScanRootPaths {
    root: PathBuf,
    base: PathBuf,
}

fn canonical_scan_root_paths(root: &Path, base_path: &Path) -> CanonicalScanRootPaths {
    CanonicalScanRootPaths {
        root: canonical_or_original(root),
        base: canonical_or_original(base_path),
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn scan_root_is_outside_base(root: &Path, base: &Path) -> bool {
    root != base && !root.starts_with(base)
}

fn repo_dirs_in_scan_root(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    entries.flatten().filter_map(repo_dir_from_entry).collect()
}

fn repo_dir_from_entry(entry: std::fs::DirEntry) -> Option<PathBuf> {
    if !entry.file_type().ok()?.is_dir() {
        return None;
    }

    let path = entry.path();
    visible_git_repo_dir(&path).then_some(path)
}

fn visible_git_repo_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|name| name.to_string_lossy()) else {
        return false;
    };

    !name.starts_with('.') && path.join(".git").is_dir()
}

fn collect_sorted_service_entries<I>(repo_dirs: I, base_path: &Path) -> Vec<OverlayServiceEntry>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut services: Vec<OverlayServiceEntry> = repo_dirs
        .into_iter()
        .filter_map(|path| service_entry_from_repo_path(None, path, base_path))
        .collect();
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
    let default_target_explicit = section
        .default_target
        .as_deref()
        .is_some_and(|target| target_exists(&targets, target));
    let default_target = valid_default_target(section.default_target, &targets);
    let group_defaults = valid_group_defaults(section.group_defaults, &targets);

    OverlayLaunchConfig {
        default_target,
        default_target_explicit,
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

fn best_mapped_launch_target(path: &Path, targets: &[LaunchTargetSummary]) -> Option<String> {
    targets
        .iter()
        .filter(|target| target.id != "local" && !target.path_mappings.is_empty())
        .flat_map(|target| {
            target.path_mappings.iter().filter_map(move |mapping| {
                launch_mapping_score(path, mapping).map(|score| (score, target.id.clone()))
            })
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, id)| id)
}

fn launch_mapping_score(path: &Path, mapping: &LaunchPathMapping) -> Option<usize> {
    let local_prefix = lexical_path_buf(&mapping.local_prefix);
    lexical_path_buf(path.to_string_lossy().as_ref())
        .strip_prefix(&local_prefix)
        .ok()?;
    Some(local_prefix.components().count())
}

fn lexical_path_buf(path: &str) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn unique_launch_targets<'a, I>(targets: I) -> Vec<LaunchTargetSummary>
where
    I: IntoIterator<Item = &'a LaunchTargetSummary>,
{
    let mut unique_targets = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for target in targets {
        if seen_ids.insert(target.id.clone()) {
            unique_targets.push(target.clone());
        }
    }
    unique_targets
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
    let Some(pattern) = ExistingDirPattern::parse(&expanded) else {
        return Vec::new();
    };

    pattern.expand()
}

enum ExistingDirPattern {
    Literal(PathBuf),
    Wildcard(WildcardDirPattern),
}

impl ExistingDirPattern {
    fn parse(expanded: &str) -> Option<Self> {
        match expanded.find('*') {
            Some(star_idx) => WildcardDirPattern::parse(expanded, star_idx).map(Self::Wildcard),
            None => Some(Self::Literal(PathBuf::from(expanded))),
        }
    }

    fn expand(self) -> Vec<PathBuf> {
        match self {
            Self::Literal(path) => existing_literal_dir(path),
            Self::Wildcard(pattern) => pattern.expand(),
        }
    }
}

struct WildcardDirPattern {
    root: PathBuf,
    suffix: String,
}

impl WildcardDirPattern {
    fn parse(expanded: &str, star_idx: usize) -> Option<Self> {
        let before = &expanded[..star_idx];
        let after = &expanded[star_idx + 1..];

        is_single_component_wildcard(before, after).then(|| Self {
            root: PathBuf::from(before.trim_end_matches('/')),
            suffix: after.trim_start_matches('/').to_string(),
        })
    }

    fn expand(self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };

        let mut results: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|entry| wildcard_candidate_dir(entry, &self.suffix))
            .collect();
        results.sort();
        results
    }
}

fn existing_literal_dir(path: PathBuf) -> Vec<PathBuf> {
    if path.is_dir() {
        vec![path]
    } else {
        Vec::new()
    }
}

fn is_single_component_wildcard(before: &str, after: &str) -> bool {
    let starts_on_boundary = before.is_empty() || before.ends_with('/');
    let ends_on_boundary = after.is_empty() || after.starts_with('/');
    starts_on_boundary && ends_on_boundary && !after.contains('*')
}

fn wildcard_candidate_dir(entry: std::fs::DirEntry, suffix: &str) -> Option<PathBuf> {
    if !entry.file_type().ok()?.is_dir() {
        return None;
    }

    let candidate = if suffix.is_empty() {
        entry.path()
    } else {
        entry.path().join(suffix)
    };
    candidate.is_dir().then_some(candidate)
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
mod tests;
