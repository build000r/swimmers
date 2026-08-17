use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::types::{
    DependencyHealthSnapshot, FleetLensPreset, FleetLensPresetMatcher, LaunchPathMapping,
    LaunchTargetSummary,
};

/// Cached skillbox overlay, loaded once from disk.
pub fn default_overlay() -> Option<&'static SkillboxOverlay> {
    default_overlay_result().ok()
}

/// Loaded overlay, or the typed reason the Skillbox contract could not be read.
///
/// Callers that need to explain themselves — API errors, health details — must
/// use this instead of `default_overlay()`. The `None` from `default_overlay()`
/// exists only so long-standing call sites keep compiling.
pub fn default_overlay_result() -> Result<&'static SkillboxOverlay, &'static ContractUnavailable> {
    static OVERLAY: OnceLock<Result<SkillboxOverlay, ContractUnavailable>> = OnceLock::new();
    OVERLAY.get_or_init(SkillboxOverlay::load).as_ref()
}

pub fn default_overlay_health() -> DependencyHealthSnapshot {
    let now = Utc::now();
    match default_overlay_result() {
        Ok(overlay) => overlay.health_snapshot(),
        Err(unavailable) => DependencyHealthSnapshot::unavailable(now, unavailable.to_string())
            .with_detail("contract_status", unavailable.code()),
    }
}

pub fn remote_targets_health() -> DependencyHealthSnapshot {
    let now = Utc::now();
    match default_overlay_result() {
        Ok(overlay) => overlay.remote_targets_health_snapshot(),
        Err(unavailable) => DependencyHealthSnapshot::unknown(now)
            .with_detail("configured_targets", "unknown")
            .with_detail("probe", "contract_unavailable")
            .with_detail("contract_status", unavailable.code()),
    }
}

pub struct SkillboxOverlay {
    clients: Vec<ClientOverlay>,
    loaded_at: DateTime<Utc>,
    contract: ContractFacts,
}

/// Contract-level facts worth reporting even when every client parses cleanly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractFacts {
    /// `schema_version` the payload was produced under.
    pub schema_version: String,
    /// `"cache"`, `"build"` (CLI) or `"file"` (explicit override).
    pub source: String,
    /// True when the cached projection is past its TTL. Never blocks a read.
    pub stale: bool,
    /// True when the payload carries `observed` presence facts.
    pub observed: bool,
    /// Which machine Skillbox resolved this box to.
    pub machine_id: Option<String>,
    /// How that machine was identified (`explicit`, `env`, `hostname`, `none`).
    pub detection_source: Option<String>,
    /// `ready` / `degraded` / `unconfigured`.
    pub readiness: Option<String>,
}

/// Why the Skillbox `env-inventory` contract could not be consumed.
///
/// Every variant is a distinct operator action, which is the point: the old
/// loader collapsed all of these into `None` and left the message
/// "no skillbox-config overlay is available" as the only trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractUnavailable {
    /// No Skillbox checkout carrying `.env-manager/manage.py` was found.
    SkillboxRepoNotFound { searched: Vec<String> },
    /// The contract CLI could not be spawned at all.
    CommandUnavailable { command: String, detail: String },
    /// The CLI ran and failed in a way the contract does not define.
    CommandFailed {
        command: String,
        code: Option<i32>,
        detail: String,
    },
    /// The CLI answered, but not with the documented envelope.
    MalformedPayload { source: String, detail: String },
    /// `ok: false` — the cache has never been written under this schema.
    NotBuilt {
        reason: String,
        next_action: Option<String>,
    },
    /// The payload is some other contract, or a schema this build cannot read.
    SchemaUnsupported {
        contract: String,
        schema_version: String,
    },
    /// `supersedes.fields` no longer promises a fact Swimmers stopped parsing.
    SupersedesIncomplete {
        consumer: String,
        missing: Vec<String>,
    },
    /// The contract parsed but declares no clients.
    NoClients { schema_version: String },
}

impl ContractUnavailable {
    /// Stable machine-readable discriminator for API errors and health details.
    pub fn code(&self) -> &'static str {
        match self {
            Self::SkillboxRepoNotFound { .. } => "SKILLBOX_REPO_NOT_FOUND",
            Self::CommandUnavailable { .. } => "CONTRACT_COMMAND_UNAVAILABLE",
            Self::CommandFailed { .. } => "CONTRACT_COMMAND_FAILED",
            Self::MalformedPayload { .. } => "CONTRACT_MALFORMED",
            Self::NotBuilt { .. } => "CONTRACT_NOT_BUILT",
            Self::SchemaUnsupported { .. } => "CONTRACT_SCHEMA_UNSUPPORTED",
            Self::SupersedesIncomplete { .. } => "CONTRACT_SUPERSEDES_INCOMPLETE",
            Self::NoClients { .. } => "CONTRACT_NO_CLIENTS",
        }
    }

    /// The one command that most plausibly clears this state.
    pub fn remedy(&self) -> Option<String> {
        match self {
            Self::SkillboxRepoNotFound { .. } => {
                Some("set SWIMMERS_SKILLBOX_REPO to a skillbox checkout".to_string())
            }
            Self::CommandUnavailable { .. } => {
                Some("set SWIMMERS_SKILLBOX_PYTHON to a python3 interpreter".to_string())
            }
            Self::NotBuilt { next_action, .. } => next_action.clone().or_else(|| {
                Some(format!(
                    "{INVENTORY_CLI_SCRIPT} env-inventory refresh --format json"
                ))
            }),
            Self::CommandFailed { .. }
            | Self::MalformedPayload { .. }
            | Self::SchemaUnsupported { .. }
            | Self::SupersedesIncomplete { .. }
            | Self::NoClients { .. } => None,
        }
    }
}

impl std::fmt::Display for ContractUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "skillbox {INVENTORY_CONTRACT} unavailable: ")?;
        match self {
            Self::SkillboxRepoNotFound { searched } => write!(
                f,
                "no skillbox checkout with {INVENTORY_CLI_SCRIPT} (looked in {})",
                searched.join(", ")
            ),
            Self::CommandUnavailable { command, detail } => {
                write!(f, "could not run `{command}` ({detail})")
            }
            Self::CommandFailed {
                command,
                code,
                detail,
            } => {
                let code = code.map_or_else(|| "signal".to_string(), |code| code.to_string());
                write!(f, "`{command}` exited {code} ({detail})")
            }
            Self::MalformedPayload { source, detail } => {
                write!(f, "{source} is not the documented envelope ({detail})")
            }
            Self::NotBuilt { reason, .. } => write!(f, "cache not built yet ({reason})"),
            Self::SchemaUnsupported {
                contract,
                schema_version,
            } => write!(
                f,
                "got contract '{contract}' schema '{schema_version}', expected \
                 '{INVENTORY_CONTRACT}' schema '{INVENTORY_SCHEMA_VERSION}'"
            ),
            Self::SupersedesIncomplete { consumer, missing } => write!(
                f,
                "supersedes.fields for consumer '{consumer}' no longer covers: {}",
                missing.join(", ")
            ),
            Self::NoClients { schema_version } => {
                write!(f, "schema '{schema_version}' declares no clients")
            }
        }
    }
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
    fleet_presets: Vec<FleetLensPreset>,
}

impl SkillboxOverlay {
    fn load() -> Result<Self, ContractUnavailable> {
        let envelope = load_inventory_envelope()?;
        let payload = envelope.into_payload()?;
        payload.verify_contract_version()?;
        payload.verify_supersedes()?;

        let clients = payload.build_client_overlays();
        if clients.is_empty() {
            return Err(ContractUnavailable::NoClients {
                schema_version: payload.inventory.schema_version.clone(),
            });
        }
        Ok(Self {
            clients,
            loaded_at: Utc::now(),
            contract: payload.facts(),
        })
    }

    /// Contract provenance for this load — schema, source, staleness, machine.
    pub fn contract(&self) -> &ContractFacts {
        &self.contract
    }

    pub fn health_snapshot(&self) -> DependencyHealthSnapshot {
        let now = Utc::now();
        let snapshot = DependencyHealthSnapshot::healthy(now)
            .with_last_seen(self.loaded_at)
            .with_detail("client_count", self.clients.len().to_string())
            .with_detail(
                "launch_target_count",
                self.all_launch_targets().len().to_string(),
            )
            .with_detail(
                "contract_schema_version",
                self.contract.schema_version.clone(),
            )
            .with_detail("contract_source", self.contract.source.clone())
            .with_detail("contract_stale", self.contract.stale.to_string())
            .with_detail("contract_observed", self.contract.observed.to_string());
        with_optional_details(
            snapshot,
            [
                ("machine_id", self.contract.machine_id.as_deref()),
                (
                    "machine_detection",
                    self.contract.detection_source.as_deref(),
                ),
                ("contract_readiness", self.contract.readiness.as_deref()),
            ],
        )
    }

    pub fn remote_targets_health_snapshot(&self) -> DependencyHealthSnapshot {
        let now = Utc::now();
        let configured_targets = self
            .all_launch_targets()
            .into_iter()
            .filter(is_swimmers_api_launch_target)
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

    pub fn all_fleet_presets(&self) -> Vec<FleetLensPreset> {
        let mut seen = BTreeSet::new();
        let mut presets = Vec::new();
        for preset in self
            .clients
            .iter()
            .flat_map(|client| client.fleet_presets.iter())
        {
            if seen.insert(preset.id.clone()) {
                presets.push(preset.clone());
            }
        }
        presets
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
            (Some(lhs), Some(rhs)) => rhs
                .cmp(&lhs)
                .then_with(|| compare_overlay_plan_entries(a, b)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => compare_overlay_plan_entries(a, b),
        });
        entries
    }
}

fn with_optional_details<'a, I>(
    snapshot: DependencyHealthSnapshot,
    details: I,
) -> DependencyHealthSnapshot
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    details
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .fold(snapshot, |snapshot, (key, value)| {
            snapshot.with_detail(key, value)
        })
}

fn compare_overlay_plan_entries(a: &OverlayPlanEntry, b: &OverlayPlanEntry) -> std::cmp::Ordering {
    a.client_label
        .cmp(&b.client_label)
        .then_with(|| a.kind.cmp(b.kind))
        .then_with(|| a.slug.cmp(&b.slug))
}

fn is_swimmers_api_launch_target(target: &LaunchTargetSummary) -> bool {
    target.kind.trim().eq_ignore_ascii_case("swimmers_api")
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
// Skillbox environment-inventory contract
//
// Skillbox owns machine/client/repo intent and the root translation that turns
// `~/repos/foo` into wherever that repo actually lives on this box. Everything
// this section reads is a field `supersedes.fields` names explicitly; Swimmers
// no longer globs `clients/*/overlay.yaml` nor parses `client.*` / `context.*`.
// The one thing still parsed privately is `dev_sanity`, which is launch config
// (and credential-adjacent) rather than environment inventory.
// ---------------------------------------------------------------------------

const INVENTORY_CONTRACT: &str = "skillbox.environment_inventory";
const INVENTORY_SCHEMA_VERSION: &str = "2026-07-25+environment_inventory.v1";
const INVENTORY_CONSUMER: &str = "swimmers";
const INVENTORY_CLI_SCRIPT: &str = ".env-manager/manage.py";
/// `show --cached` reports a cache miss as exit 4 (drift), which is a normal
/// answer carrying a payload — not a failure.
const INVENTORY_CLI_DRIFT_EXIT: i32 = 4;
const INVENTORY_CLI_TIMEOUT: Duration = Duration::from_secs(20);

/// Every `supersedes.fields` key Swimmers relies on. Each one names a fact this
/// module used to derive by parsing skillbox-config directly; if the contract
/// ever stops promising one, we must fail loudly rather than silently read a
/// field that no longer means what it meant.
const REQUIRED_SUPERSEDED_FIELDS: &[&str] = &[
    "<derived> repo exists on this box",
    "<derived> repo root translation",
    "<derived> which machine am I",
    "<discovery> clients/*/overlay.yaml glob",
    "client.id",
    "client.label",
    "client.repos[].id",
    "client.repos[].kind",
    "client.repos[].repo_path",
    "context.cwd_match",
    "context.plans.plan_draft",
    "context.plans.plan_root",
    "context.repo_landscape.repos[].path",
    "context.repo_landscape.scan_roots",
];

#[derive(Debug, Deserialize)]
struct InventoryEnvelope {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    stale: bool,
    #[serde(default)]
    cache_path: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    inventory: Option<Inventory>,
    #[serde(default)]
    next_actions: Vec<String>,
}

/// An envelope whose `inventory` is known to be present.
#[derive(Debug)]
struct LoadedInventory {
    inventory: Inventory,
    source: String,
    stale: bool,
}

impl InventoryEnvelope {
    fn into_payload(self) -> Result<LoadedInventory, ContractUnavailable> {
        let source = self.source.unwrap_or_else(|| "unknown".to_string());
        match self.inventory.filter(|_| self.ok) {
            Some(inventory) => Ok(LoadedInventory {
                inventory,
                source,
                stale: self.stale,
            }),
            None => Err(ContractUnavailable::NotBuilt {
                reason: self.reason.unwrap_or_else(|| {
                    self.cache_path.map_or_else(
                        || "no inventory in payload".to_string(),
                        |path| format!("no cache at {path}"),
                    )
                }),
                next_action: self.next_actions.into_iter().next(),
            }),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct Inventory {
    #[serde(default)]
    contract: String,
    #[serde(default)]
    schema_version: String,
    #[serde(default)]
    machine: InventoryMachine,
    #[serde(default)]
    clients: Vec<InventoryClient>,
    #[serde(default)]
    repos: Vec<InventoryRepo>,
    #[serde(default)]
    sources: Vec<InventorySource>,
    #[serde(default)]
    supersedes: InventorySupersedes,
    #[serde(default)]
    readiness: InventoryReadiness,
    #[serde(default)]
    freshness: InventoryFreshness,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryMachine {
    #[serde(default)]
    declared: InventoryMachineDeclared,
    #[serde(default)]
    detection_source: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryMachineDeclared {
    #[serde(default)]
    machine_id: Option<String>,
    #[serde(default)]
    repo_roots: Vec<String>,
    #[serde(default)]
    projects_roots: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryClient {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    declared: InventoryClientDeclared,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryClientDeclared {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    cwd_match: Vec<String>,
    #[serde(default)]
    plan_root: Option<String>,
    #[serde(default)]
    plan_draft: Option<String>,
    #[serde(default)]
    scan_roots: Vec<String>,
    #[serde(default)]
    repo_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryRepo {
    #[serde(default)]
    repo_id: String,
    #[serde(default)]
    declared: InventoryRepoDeclared,
    #[serde(default)]
    observed: Option<InventoryRepoObserved>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryRepoDeclared {
    #[serde(default)]
    registry_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    path_declared: Option<String>,
    #[serde(default)]
    path_relative: Option<String>,
    #[serde(default)]
    root_category: Option<String>,
    #[serde(default)]
    declared_by: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryRepoObserved {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    present: bool,
}

#[derive(Debug, Deserialize, Default)]
struct InventorySource {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    present: bool,
}

#[derive(Debug, Deserialize, Default)]
struct InventorySupersedes {
    #[serde(default)]
    consumer: Option<String>,
    #[serde(default)]
    fields: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryReadiness {
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct InventoryFreshness {
    #[serde(default)]
    observed: bool,
}

impl LoadedInventory {
    fn facts(&self) -> ContractFacts {
        ContractFacts {
            schema_version: self.inventory.schema_version.clone(),
            source: self.source.clone(),
            stale: self.stale,
            observed: self.inventory.freshness.observed,
            machine_id: nonempty(self.inventory.machine.declared.machine_id.clone()),
            detection_source: nonempty(self.inventory.machine.detection_source.clone()),
            readiness: nonempty(self.inventory.readiness.status.clone()),
        }
    }

    fn verify_contract_version(&self) -> Result<(), ContractUnavailable> {
        let matches = self.inventory.contract == INVENTORY_CONTRACT
            && self.inventory.schema_version == INVENTORY_SCHEMA_VERSION;
        if matches {
            return Ok(());
        }
        Err(ContractUnavailable::SchemaUnsupported {
            contract: self.inventory.contract.clone(),
            schema_version: self.inventory.schema_version.clone(),
        })
    }

    /// Assert the contract still promises every mapping Swimmers deleted its
    /// private parser for. A key present with an empty list counts as missing:
    /// the promise is the list of contract paths that now own the fact.
    fn verify_supersedes(&self) -> Result<(), ContractUnavailable> {
        let supersedes = &self.inventory.supersedes;
        let missing: Vec<String> = REQUIRED_SUPERSEDED_FIELDS
            .iter()
            .filter(|field| {
                supersedes
                    .fields
                    .get(**field)
                    .is_none_or(|paths| paths.is_empty())
            })
            .map(|field| (*field).to_string())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(ContractUnavailable::SupersedesIncomplete {
            consumer: supersedes
                .consumer
                .clone()
                .unwrap_or_else(|| INVENTORY_CONSUMER.to_string()),
            missing,
        })
    }

    fn build_client_overlays(&self) -> Vec<ClientOverlay> {
        let index = ContractIndex::new(&self.inventory);
        self.inventory
            .clients
            .iter()
            .map(|client| build_client_overlay(client, &index))
            .collect()
    }
}

/// Declared repo roots for this machine, used to translate a repo's declared
/// spelling onto wherever it actually lives here.
#[derive(Default)]
struct MachineRoots {
    repos: Vec<String>,
    projects: Vec<String>,
}

impl MachineRoots {
    fn for_category(&self, category: Option<&str>) -> &[String] {
        match category {
            Some("repos") => &self.repos,
            Some("projects") => &self.projects,
            _ => &[],
        }
    }
}

struct ContractIndex<'a> {
    repos: BTreeMap<&'a str, &'a InventoryRepo>,
    /// client_id -> the overlay.yaml the contract read that client from.
    overlay_paths: BTreeMap<&'a str, PathBuf>,
    machine_roots: MachineRoots,
}

impl<'a> ContractIndex<'a> {
    fn new(inventory: &'a Inventory) -> Self {
        Self {
            repos: inventory
                .repos
                .iter()
                .map(|repo| (repo.repo_id.as_str(), repo))
                .collect(),
            overlay_paths: client_overlay_paths(&inventory.sources),
            machine_roots: MachineRoots {
                repos: inventory.machine.declared.repo_roots.clone(),
                projects: inventory.machine.declared.projects_roots.clone(),
            },
        }
    }

    /// Repos this client declared under `declared_by` marker `suffix`, in the
    /// contract's stable `repo_ids` order.
    fn client_repos(&self, client: &InventoryClient, suffix: &str) -> Vec<&'a InventoryRepo> {
        let marker = format!("client:{}.{suffix}", contract_client_id(client));
        client
            .declared
            .repo_ids
            .iter()
            .filter_map(|id| self.repos.get(id.as_str()).copied())
            .filter(|repo| repo.declared.declared_by.contains(&marker))
            .collect()
    }
}

fn client_overlay_paths(sources: &[InventorySource]) -> BTreeMap<&str, PathBuf> {
    sources
        .iter()
        .filter(|source| source.kind == "client_overlay" && source.present)
        .filter_map(|source| {
            let client_id = source.client_id.as_deref()?;
            let path = nonempty(source.path.clone())?;
            Some((client_id, PathBuf::from(expand_path(&path))))
        })
        .collect()
}

fn contract_client_id(client: &InventoryClient) -> &str {
    client
        .declared
        .client_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(&client.client_id)
}

fn build_client_overlay(client: &InventoryClient, index: &ContractIndex) -> ClientOverlay {
    let client_id = contract_client_id(client);
    let label = nonempty(client.declared.label.clone()).unwrap_or_else(|| client_id.to_string());
    let overlay_path = index.overlay_paths.get(client_id);
    let client_dir = overlay_path.and_then(|path| path.parent());

    let landscape_repos = index.client_repos(client, "repo_landscape");
    let cwd_patterns = client_cwd_patterns(client, &landscape_repos, &index.machine_roots);
    let scan_roots: Vec<PathBuf> = client
        .declared
        .scan_roots
        .iter()
        .map(|root| PathBuf::from(expand_path(root)))
        .collect();

    let dev_sanity = overlay_path.and_then(|path| read_dev_sanity_section(path));
    let fleet_presets = dev_sanity
        .as_ref()
        .map(|section| parse_fleet_lenses(&section.fleet_lenses))
        .unwrap_or_default();
    let dir_config = dev_sanity.and_then(|section| {
        let client_repos = index.client_repos(client, "repos");
        parse_dir_config(section, &label, &client_repos, &scan_roots)
    });

    ClientOverlay {
        label,
        cwd_patterns,
        cwd_match_count: client.declared.cwd_match.len(),
        plan_root: contract_plan_dir(client_dir, client.declared.plan_root.as_deref()),
        plan_draft: contract_plan_dir(client_dir, client.declared.plan_draft.as_deref()),
        dir_config,
        fleet_presets,
    }
}

fn contract_plan_dir(client_dir: Option<&Path>, relative: Option<&str>) -> Option<PathBuf> {
    let relative = nonempty(relative.map(str::to_string))?;
    Some(client_dir?.join(relative))
}

/// `cwd_match`, then `scan_roots`, then every declared landscape repo — the
/// same precedence the private parser used, now sourced from the contract.
fn client_cwd_patterns(
    client: &InventoryClient,
    landscape_repos: &[&InventoryRepo],
    roots: &MachineRoots,
) -> Vec<String> {
    let mut patterns: Vec<String> = client
        .declared
        .cwd_match
        .iter()
        .chain(client.declared.scan_roots.iter())
        .map(|path| expand_path(path))
        .collect();
    for repo in landscape_repos {
        patterns.extend(repo_match_paths(repo, roots));
    }
    dedupe_preserving_order(patterns)
}

/// Every place this repo could be on this box, most-authoritative first.
///
/// The observed path is Skillbox's own answer. Failing that we translate
/// `path_relative` onto each declared root for its category — the root
/// translation this module used to skip entirely — and only then fall back to
/// the declared spelling.
fn repo_match_paths(repo: &InventoryRepo, roots: &MachineRoots) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(observed) = observed_repo_path(repo) {
        paths.push(observed);
    }
    if let Some(relative) = nonempty(repo.declared.path_relative.clone()) {
        for root in roots.for_category(repo.declared.root_category.as_deref()) {
            let root = expand_path(root);
            paths.push(format!("{}/{relative}", root.trim_end_matches('/')));
        }
    }
    if let Some(declared) = nonempty(repo.declared.path_declared.clone()) {
        paths.push(expand_path(&declared));
    }
    dedupe_preserving_order(paths)
}

fn observed_repo_path(repo: &InventoryRepo) -> Option<String> {
    let observed = repo.observed.as_ref()?;
    observed
        .present
        .then(|| nonempty(observed.path.clone()))
        .flatten()
        .map(|path| expand_path(&path))
}

fn dedupe_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(nonempty_launch_metadata)
}

// ---------------------------------------------------------------------------
// Contract transport: locate a Skillbox checkout, run the CLI, read the payload
// ---------------------------------------------------------------------------

fn load_inventory_envelope() -> Result<InventoryEnvelope, ContractUnavailable> {
    if let Some(path) = inventory_override_path() {
        return read_inventory_file(&path);
    }
    run_inventory_cli(&skillbox_repo_root()?)
}

/// Explicit payload override: an already-captured `env-inventory show` envelope
/// (or a bare inventory document). Keeps the contract readable on a box with no
/// Skillbox checkout, and gives tests a seam that never shells out.
fn inventory_override_path() -> Option<PathBuf> {
    let raw = std::env::var("SWIMMERS_SKILLBOX_INVENTORY").ok()?;
    nonempty(Some(raw)).map(|raw| PathBuf::from(expand_path(&raw)))
}

fn read_inventory_file(path: &Path) -> Result<InventoryEnvelope, ContractUnavailable> {
    let source = path.display().to_string();
    let contents =
        std::fs::read_to_string(path).map_err(|err| ContractUnavailable::MalformedPayload {
            source: source.clone(),
            detail: err.to_string(),
        })?;
    parse_inventory_json(&contents, &source)
}

/// Accepts either the CLI envelope or a bare inventory document (the shape
/// written to the cache file), so an operator can point at either one.
fn parse_inventory_json(raw: &str, source: &str) -> Result<InventoryEnvelope, ContractUnavailable> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|err| ContractUnavailable::MalformedPayload {
            source: source.to_string(),
            detail: err.to_string(),
        })?;
    let is_envelope = value.get("inventory").is_some();
    let malformed = |err: serde_json::Error| ContractUnavailable::MalformedPayload {
        source: source.to_string(),
        detail: err.to_string(),
    };
    if is_envelope {
        return serde_json::from_value(value).map_err(malformed);
    }
    let inventory: Inventory = serde_json::from_value(value).map_err(malformed)?;
    Ok(InventoryEnvelope {
        ok: true,
        source: Some("file".to_string()),
        stale: false,
        cache_path: None,
        reason: None,
        inventory: Some(inventory),
        next_actions: Vec::new(),
    })
}

fn skillbox_repo_root() -> Result<PathBuf, ContractUnavailable> {
    let candidates = skillbox_repo_candidates();
    candidates
        .iter()
        .find(|root| root.join(INVENTORY_CLI_SCRIPT).is_file())
        .cloned()
        .ok_or_else(|| ContractUnavailable::SkillboxRepoNotFound {
            searched: candidates
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
        })
}

fn skillbox_repo_candidates() -> Vec<PathBuf> {
    if let Some(explicit) = std::env::var("SWIMMERS_SKILLBOX_REPO")
        .ok()
        .and_then(|raw| nonempty(Some(raw)))
    {
        return vec![PathBuf::from(expand_path(&explicit))];
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        home.join("repos").join("opensource").join("skillbox"),
        home.join("repos").join("skillbox"),
    ]
}

fn run_inventory_cli(repo_root: &Path) -> Result<InventoryEnvelope, ContractUnavailable> {
    let python = std::env::var("SWIMMERS_SKILLBOX_PYTHON")
        .ok()
        .and_then(|raw| nonempty(Some(raw)))
        .unwrap_or_else(|| "python3".to_string());
    // `--cached` is the only hot-path-safe read: one file, no YAML, no probing.
    // The subcommand flags are not mirrored onto the parent parser, so
    // `--format json` has to follow `show`.
    let args = [
        INVENTORY_CLI_SCRIPT,
        "env-inventory",
        "show",
        "--cached",
        "--format",
        "json",
    ];
    let display = format!("{python} {}", args.join(" "));

    let mut command = Command::new(&python);
    command
        .args(args)
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let captured = capture_command(command, INVENTORY_CLI_TIMEOUT).map_err(|err| {
        ContractUnavailable::CommandUnavailable {
            command: display.clone(),
            detail: err.to_string(),
        }
    })?;
    let Some(captured) = captured else {
        return Err(ContractUnavailable::CommandFailed {
            command: display,
            code: None,
            detail: format!("timed out after {}s", INVENTORY_CLI_TIMEOUT.as_secs()),
        });
    };

    // Exit 4 is the contract's "cache miss" answer and still carries a payload.
    if !matches!(captured.code, Some(0) | Some(INVENTORY_CLI_DRIFT_EXIT)) {
        return Err(ContractUnavailable::CommandFailed {
            command: display,
            code: captured.code,
            detail: first_line(&captured.stderr),
        });
    }
    parse_inventory_json(&captured.stdout, &display)
}

struct CapturedCommand {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run `command`, draining both pipes on their own threads so a payload larger
/// than the pipe buffer cannot deadlock, and give up after `timeout`.
fn capture_command(
    mut command: Command,
    timeout: Duration,
) -> std::io::Result<Option<CapturedCommand>> {
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().map(drain_pipe);
    let stderr = child.stderr.take().map(drain_pipe);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    Ok(Some(CapturedCommand {
        code: status.code(),
        stdout: stdout.map(join_pipe).unwrap_or_default(),
        stderr: stderr.map(join_pipe).unwrap_or_default(),
    }))
}

fn drain_pipe<R>(mut pipe: R) -> std::thread::JoinHandle<String>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        String::from_utf8_lossy(&buffer).into_owned()
    })
}

fn join_pipe(handle: std::thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_string()
}

// ---------------------------------------------------------------------------
// Retained private parsing: `dev_sanity` only.
//
// This section is launch config, not environment inventory — `auth_token_env`,
// `ssh_alias` and `base_url` are credential-adjacent, and Skillbox deliberately
// left them out of the contract. Everything else that used to live here now
// comes from `supersedes.fields`.
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct OverlayFile {
    #[serde(default)]
    dev_sanity: Option<DevSanitySection>,
}

#[derive(Deserialize, Default)]
struct DevSanitySection {
    #[serde(default)]
    agent_launch: Option<DevSanityAgentLaunch>,
    #[serde(default)]
    services: Option<DevSanityServices>,
    #[serde(default)]
    groups: Vec<DevSanityGroup>,
    #[serde(default)]
    fleet_lenses: Vec<DevSanityFleetLens>,
}

#[derive(Deserialize, Default)]
struct DevSanityFleetLens {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    matchers: Vec<FleetLensPresetMatcher>,
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
    ssh_alias: Option<String>,
    #[serde(default)]
    remote_attach_command_template: Option<String>,
    #[serde(default)]
    bootstrap_hint: Option<String>,
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

fn read_dev_sanity_section(overlay_path: &Path) -> Option<DevSanitySection> {
    let content = std::fs::read_to_string(overlay_path).ok()?;
    let file: OverlayFile = serde_yaml::from_str(&content).ok()?;
    file.dev_sanity
}

fn parse_dir_config(
    section: DevSanitySection,
    client_label: &str,
    client_repos: &[&InventoryRepo],
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

fn parse_fleet_lenses(entries: &[DevSanityFleetLens]) -> Vec<FleetLensPreset> {
    entries
        .iter()
        .filter_map(|entry| {
            let id = nonempty_launch_metadata(entry.id.clone()?)?;
            let label = entry
                .label
                .clone()
                .and_then(nonempty_launch_metadata)
                .unwrap_or_else(|| id.clone());
            (!entry.matchers.is_empty()).then(|| FleetLensPreset {
                id,
                label,
                source: "overlay".to_string(),
                matchers: entry.matchers.clone(),
            })
        })
        .collect()
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
    client_repos: &[&InventoryRepo],
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
    client_repos: &[&InventoryRepo],
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
    let id = nonempty_launch_metadata(target.id?)?;
    let label = target
        .label
        .and_then(nonempty_launch_metadata)
        .unwrap_or_else(|| id.clone());
    let kind = target
        .kind
        .and_then(nonempty_launch_metadata)
        .unwrap_or_else(|| "local".to_string());
    Some(LaunchTargetSummary {
        label,
        kind,
        id,
        base_url: target.base_url,
        auth_token_env: target.auth_token_env,
        ssh_alias: target.ssh_alias.and_then(nonempty_launch_metadata),
        remote_attach_command_template: target
            .remote_attach_command_template
            .and_then(nonempty_launch_metadata),
        bootstrap_hint: target.bootstrap_hint.and_then(nonempty_launch_metadata),
        path_mappings: parse_launch_path_mappings(target.path_mappings),
    })
}

fn nonempty_launch_metadata(raw: String) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_launch_path_mappings(mappings: Vec<DevSanityLaunchPathMapping>) -> Vec<LaunchPathMapping> {
    mappings
        .into_iter()
        .filter_map(parse_launch_path_mapping)
        .collect()
}

fn parse_launch_path_mapping(mapping: DevSanityLaunchPathMapping) -> Option<LaunchPathMapping> {
    let local_prefix = expanded_nonempty_launch_prefix(mapping.local_prefix?)?;
    let remote_prefix = expanded_nonempty_launch_prefix(mapping.remote_prefix?)?;
    Some(LaunchPathMapping {
        local_prefix,
        remote_prefix,
    })
}

fn expanded_nonempty_launch_prefix(raw: String) -> Option<String> {
    let expanded = expand_path(&raw);
    (!expanded.trim().is_empty()).then_some(expanded)
}

fn ensure_local_launch_target(targets: &mut Vec<LaunchTargetSummary>) {
    if !target_exists(targets, "local") {
        targets.insert(0, LaunchTargetSummary::local());
    }
}

fn valid_default_target(default_target: Option<String>, targets: &[LaunchTargetSummary]) -> String {
    default_target
        .and_then(nonempty_launch_metadata)
        .filter(|target| target_exists(targets, target))
        .unwrap_or_else(|| "local".to_string())
}

fn valid_group_defaults(
    group_defaults: BTreeMap<String, String>,
    targets: &[LaunchTargetSummary],
) -> BTreeMap<String, String> {
    group_defaults
        .into_iter()
        .filter_map(|(group, target)| {
            let target = nonempty_launch_metadata(target)?;
            target_exists(targets, &target).then_some((group, target))
        })
        .collect()
}

fn target_exists(targets: &[LaunchTargetSummary], id: &str) -> bool {
    let id = id.trim();
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
        .fold(
            None,
            |best: Option<(usize, String)>, candidate| match best {
                Some((best_score, best_id)) if best_score >= candidate.0 => {
                    Some((best_score, best_id))
                }
                _ => Some(candidate),
            },
        )
        .map(|(_, id)| id)
}

fn launch_mapping_score(path: &Path, mapping: &LaunchPathMapping) -> Option<usize> {
    if mapping.local_prefix.trim().is_empty() || mapping.remote_prefix.trim().is_empty() {
        return None;
    }
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

/// Turn a contract repo the client declared under `client.repos[]` into a
/// browsable service entry.
///
/// `registry_id` is `client.repos[].id`, `kind` is `client.repos[].kind`, and
/// the path prefers Skillbox's observed answer before falling back to the
/// declared spelling.
fn service_entry_from_client_repo(
    repo: &InventoryRepo,
    base_path: &Path,
) -> Option<OverlayServiceEntry> {
    if repo
        .declared
        .kind
        .as_deref()
        .is_some_and(|kind| kind != "repo")
    {
        return None;
    }

    let repo_path = match observed_repo_path(repo) {
        Some(observed) => PathBuf::from(observed),
        None => expand_repo_path(&nonempty(repo.declared.path_declared.clone())?, base_path),
    };
    service_entry_from_repo_path(
        nonempty(repo.declared.registry_id.clone()),
        repo_path,
        base_path,
    )
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

    if result == "~" {
        // A bare `~` (no trailing slash) must also expand to home; otherwise an
        // overlay path written as `~` is left literal and silently dropped by
        // the downstream is_dir()/canonicalize() filters with no diagnostic.
        if let Some(home) = dirs::home_dir() {
            result = home.display().to_string();
        }
    } else if result.starts_with("~/") {
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
