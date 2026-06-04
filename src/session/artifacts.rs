use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::session::overlay::default_overlay;

pub const MERMAID_SOURCE_MAX_BYTES: u64 = 64 * 1024;
pub const VIEWER_TEXT_MAX_BYTES: u64 = 128 * 1024;
pub const MERMAID_SCAN_MAX_FILES: usize = 512;
const SCHEMA_MMD_FILENAME: &str = "schema.mmd";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    Mermaid,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ArtifactDiscoveryContext {
    pub session_id: String,
    pub tmux_name: String,
    pub cwd: String,
    pub session_started_at: DateTime<Utc>,
    pub pane_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredArtifact {
    pub kind: ArtifactKind,
    pub path: String,
    pub updated_at: DateTime<Utc>,
    pub source: Option<String>,
    pub error: Option<String>,
}

pub trait ArtifactDetector: Send + Sync {
    fn kind(&self) -> ArtifactKind;
    fn discover(&self, context: &ArtifactDiscoveryContext) -> Option<DiscoveredArtifact>;
}

pub struct ArtifactRegistry {
    detectors: Vec<Box<dyn ArtifactDetector>>,
}

impl Default for ArtifactRegistry {
    fn default() -> Self {
        Self {
            detectors: vec![Box::new(MermaidArtifactDetector)],
        }
    }
}

impl ArtifactRegistry {
    pub fn discover(
        &self,
        kind: ArtifactKind,
        context: &ArtifactDiscoveryContext,
    ) -> Option<DiscoveredArtifact> {
        self.detectors
            .iter()
            .find(|detector| detector.kind() == kind)
            .and_then(|detector| detector.discover(context))
    }
}

pub fn default_artifact_registry() -> &'static ArtifactRegistry {
    static REGISTRY: OnceLock<ArtifactRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ArtifactRegistry::default)
}

struct MermaidArtifactDetector;

#[derive(Debug, Clone)]
struct MermaidCandidate {
    display_path: PathBuf,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
struct MermaidScanResult {
    candidates: Vec<MermaidCandidate>,
    truncated: bool,
}

impl ArtifactDetector for MermaidArtifactDetector {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::Mermaid
    }

    fn discover(&self, context: &ArtifactDiscoveryContext) -> Option<DiscoveredArtifact> {
        let root = context.cwd.trim();
        if root.is_empty() {
            return None;
        }

        if let Some(artifact) = discover_overlay_mermaid_artifact(root) {
            return Some(artifact);
        }

        let scan = scan_mermaid_candidates(root);
        select_repo_mermaid_candidate(&scan, context.session_started_at)
            .map(|best| read_candidate_artifact(best, scan.truncated))
    }
}

fn discover_overlay_mermaid_artifact(root: &str) -> Option<DiscoveredArtifact> {
    let scan = scan_overlay_plan_dirs(root)?;
    select_best_mermaid_candidate(scan.candidates.iter())
        .map(|best| read_candidate_artifact(best, scan.truncated))
}

fn scan_overlay_plan_dirs(root: &str) -> Option<MermaidScanResult> {
    let overlay = default_overlay()?;
    let plan_dirs = overlay.find_plan_dirs(root)?;
    Some(scan_bounded_mermaid_dirs(&plan_dirs))
}

fn scan_bounded_mermaid_dirs(plan_dirs: &[PathBuf]) -> MermaidScanResult {
    let mut overlay_scan = MermaidScanResult::default();
    let _ = plan_dirs.iter().any(|dir| {
        let scan = scan_mermaid_candidates(&dir.to_string_lossy());
        append_bounded_mermaid_scan(&mut overlay_scan, scan)
    });
    overlay_scan
}

fn append_bounded_mermaid_scan(
    overlay_scan: &mut MermaidScanResult,
    scan: MermaidScanResult,
) -> bool {
    overlay_scan.truncated |= scan.truncated;
    overlay_scan.candidates.extend(scan.candidates);
    apply_mermaid_scan_file_cap(overlay_scan)
}

fn apply_mermaid_scan_file_cap(scan: &mut MermaidScanResult) -> bool {
    if scan.candidates.len() < MERMAID_SCAN_MAX_FILES {
        return false;
    }
    scan.candidates.truncate(MERMAID_SCAN_MAX_FILES);
    scan.truncated = true;
    true
}

fn scan_mermaid_candidates(root: &str) -> MermaidScanResult {
    let mut candidates = Vec::new();
    let mut truncated = false;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_visit_artifact_entry)
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("mmd"))
            .unwrap_or(false)
        {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        candidates.push(MermaidCandidate {
            display_path: path.to_path_buf(),
            updated_at: DateTime::<Utc>::from(modified),
        });
        if candidates.len() >= MERMAID_SCAN_MAX_FILES {
            truncated = true;
            break;
        }
    }
    MermaidScanResult {
        candidates,
        truncated,
    }
}

fn select_repo_mermaid_candidate(
    scan: &MermaidScanResult,
    session_started_at: DateTime<Utc>,
) -> Option<MermaidCandidate> {
    select_plan_mermaid_candidate(scan.candidates.iter()).or_else(|| {
        select_best_mermaid_candidate(
            scan.candidates
                .iter()
                .filter(|candidate| candidate.updated_at >= session_started_at),
        )
    })
}

fn select_plan_mermaid_candidate<'a>(
    candidates: impl Iterator<Item = &'a MermaidCandidate>,
) -> Option<MermaidCandidate> {
    select_best_mermaid_candidate(
        candidates.filter(|candidate| has_plan_siblings(&candidate.display_path)),
    )
}

fn select_best_mermaid_candidate<'a>(
    candidates: impl Iterator<Item = &'a MermaidCandidate>,
) -> Option<MermaidCandidate> {
    candidates
        .max_by(|left, right| compare_mermaid_candidates(left, right))
        .cloned()
}

fn read_candidate_artifact(
    candidate: MermaidCandidate,
    scan_truncated: bool,
) -> DiscoveredArtifact {
    let scan_error = scan_truncated.then(|| {
        format!(
            "artifact scan reached {MERMAID_SCAN_MAX_FILES} Mermaid files; showing newest file from bounded scan"
        )
    });
    let (source, read_error) = match read_text_file_bounded(
        &candidate.display_path,
        MERMAID_SOURCE_MAX_BYTES,
        "Mermaid artifact",
    ) {
        Ok(source) => (Some(source), None),
        Err(err) => (None, Some(err)),
    };
    let error = join_artifact_errors([scan_error, read_error]);
    DiscoveredArtifact {
        kind: ArtifactKind::Mermaid,
        path: candidate.display_path.to_string_lossy().into_owned(),
        updated_at: candidate.updated_at,
        source,
        error,
    }
}

pub fn read_text_file_bounded(path: &Path, max_bytes: u64, label: &str) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|err| format!("failed to inspect {label}: {err}"))?;
    if metadata.len() > max_bytes {
        return Err(format!(
            "{label} exceeds {} KiB limit ({} bytes); content omitted",
            max_bytes / 1024,
            metadata.len()
        ));
    }
    fs::read_to_string(path).map_err(|err| format!("failed to read {label}: {err}"))
}

fn join_artifact_errors(errors: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let messages = errors.into_iter().flatten().collect::<Vec<_>>();
    (!messages.is_empty()).then(|| messages.join("; "))
}

/// Returns true if the .mmd file's parent directory contains known plan sibling files.
pub fn has_plan_siblings(mmd_path: &Path) -> bool {
    let Some(dir) = mmd_path.parent() else {
        return false;
    };
    PLAN_SIBLING_FILENAMES
        .iter()
        .any(|name| dir.join(name).is_file())
}

fn compare_mermaid_candidates(left: &MermaidCandidate, right: &MermaidCandidate) -> Ordering {
    left.updated_at
        .cmp(&right.updated_at)
        .then_with(|| left.display_path.cmp(&right.display_path))
}

pub fn extract_mmd_slice_name(path: &str) -> Option<&str> {
    let parts: Vec<&str> = path.split('/').collect();
    extract_schema_slice_from_plans_dir(&parts)
        .or_else(|| extract_schema_slice_from_status_dir(&parts))
        .or_else(|| extract_schema_slice_from_plan_siblings(path))
}

fn extract_schema_slice_from_plans_dir<'a>(parts: &[&'a str]) -> Option<&'a str> {
    extract_slice_from_schema_windows(parts, |window| {
        window[0] == "plans" && matches!(window[1], "released" | "draft")
    })
}

fn extract_schema_slice_from_status_dir<'a>(parts: &[&'a str]) -> Option<&'a str> {
    extract_slice_from_schema_windows(parts, |window| {
        matches!(window[1], "released" | "draft" | "planned")
    })
}

fn extract_slice_from_schema_windows<'a>(
    parts: &[&'a str],
    matches_window: impl Fn(&[&'a str]) -> bool,
) -> Option<&'a str> {
    parts
        .windows(4)
        .find(|window| window[3] == SCHEMA_MMD_FILENAME && matches_window(window))
        .map(|window| window[2])
}

fn extract_schema_slice_from_plan_siblings(path: &str) -> Option<&str> {
    let path = Path::new(path);
    if path.file_name()?.to_str()? != SCHEMA_MMD_FILENAME || !has_plan_siblings(path) {
        return None;
    }
    path.parent()?.file_name()?.to_str()
}

pub const PLAN_SIBLING_FILENAMES: &[&str] = &[
    "plan.md",
    "shared.md",
    "backend.md",
    "frontend.md",
    "flows.md",
    "WORKGRAPH.md",
];

pub const VIEWER_TEXT_FILENAMES: &[&str] = &[
    "plan.md",
    "shared.md",
    "backend.md",
    "frontend.md",
    "flows.md",
    "WORKGRAPH.md",
    "README.md",
    "VISION.md",
];

/// Given the absolute path to a `schema.mmd` inside a plan directory, returns the
/// filenames of sibling plan files that exist on disk.
pub fn list_plan_siblings(schema_path: &str) -> Vec<String> {
    let path = std::path::Path::new(schema_path);
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    PLAN_SIBLING_FILENAMES
        .iter()
        .filter(|name| dir.join(name).is_file())
        .map(|name| (*name).to_string())
        .collect()
}

pub fn resolve_repo_root(cwd: &str) -> Option<PathBuf> {
    repo_root_search_start(Path::new(cwd)).and_then(nearest_repo_root)
}

fn repo_root_search_start(path: &Path) -> Option<&Path> {
    path.is_dir().then_some(path).or_else(|| path.parent())
}

fn nearest_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| looks_like_repo_root(candidate))
        .map(Path::to_path_buf)
}

fn looks_like_repo_root(path: &Path) -> bool {
    REPO_ROOT_MARKERS
        .iter()
        .any(|(name, matches)| matches(&path.join(name)))
}

type RepoRootMarkerPredicate = fn(&Path) -> bool;

const REPO_ROOT_MARKERS: &[(&str, RepoRootMarkerPredicate)] = &[
    (".git", Path::exists),
    ("Cargo.toml", Path::is_file),
    ("package.json", Path::is_file),
    (".swimmers", Path::is_dir),
    (".throngterm", Path::is_dir),
];

pub fn list_repo_docs(cwd: &str) -> Vec<String> {
    let Some(root) = resolve_repo_root(cwd) else {
        return Vec::new();
    };

    let mut docs = Vec::new();
    if root.join("README.md").is_file() {
        docs.push("README.md".to_string());
    }
    if root.join("docs").join("VISION.md").is_file() || root.join("VISION.md").is_file() {
        docs.push("VISION.md".to_string());
    }
    docs
}

pub fn resolve_viewer_text_path(
    cwd: &str,
    schema_path: Option<&str>,
    name: &str,
) -> Option<PathBuf> {
    if PLAN_SIBLING_FILENAMES.contains(&name) {
        let dir = Path::new(schema_path?).parent()?;
        let path = dir.join(name);
        return path.is_file().then_some(path);
    }

    let root = resolve_repo_root(cwd)?;
    match name {
        "README.md" => {
            let path = root.join("README.md");
            path.is_file().then_some(path)
        }
        "VISION.md" => {
            let docs_path = root.join("docs").join("VISION.md");
            if docs_path.is_file() {
                Some(docs_path)
            } else {
                let root_path = root.join("VISION.md");
                root_path.is_file().then_some(root_path)
            }
        }
        _ => None,
    }
}

fn should_visit_artifact_entry(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }

    let Some(name) = entry.file_name().to_str() else {
        return false;
    };
    !matches!(
        name,
        ".git" | "node_modules" | "target" | ".next" | ".turbo" | ".venv" | "venv"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        default_artifact_registry, resolve_repo_root, resolve_viewer_text_path,
        ArtifactDiscoveryContext, ArtifactKind, DiscoveredArtifact,
    };
    use chrono::{Duration, Utc};
    use std::fs;

    fn discover_mermaid(context: &ArtifactDiscoveryContext) -> Option<DiscoveredArtifact> {
        default_artifact_registry().discover(ArtifactKind::Mermaid, context)
    }

    #[test]
    fn mermaid_discovery_uses_latest_post_start_file_and_ignores_skipped_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("before.mmd"), "graph TD\nOld-->Node\n").expect("write before");

        let session_started_at = Utc::now();
        std::thread::sleep(std::time::Duration::from_millis(25));

        let docs_dir = root.join("docs");
        fs::create_dir_all(&docs_dir).expect("create docs");
        let expected_path = docs_dir.join("chosen.mmd");
        fs::write(&expected_path, "graph TD\nA-->B\n").expect("write chosen");

        std::thread::sleep(std::time::Duration::from_millis(25));

        let skipped_dir = root.join("node_modules");
        fs::create_dir_all(&skipped_dir).expect("create node_modules");
        fs::write(
            skipped_dir.join("ignored.mmd"),
            "graph TD\nIgnored-->Node\n",
        )
        .expect("write ignored");

        let artifact = discover_mermaid(&ArtifactDiscoveryContext {
            session_id: "sess-mermaid".to_string(),
            tmux_name: "29".to_string(),
            cwd: root.to_string_lossy().into_owned(),
            session_started_at,
            pane_tail: String::new(),
        })
        .expect("artifact");

        assert_eq!(artifact.path, expected_path.to_string_lossy());
        assert_eq!(artifact.source.as_deref(), Some("graph TD\nA-->B\n"));
        assert!(artifact.error.is_none());
    }

    #[test]
    fn mermaid_discovery_ignores_pre_session_files_even_when_pane_tail_mentions_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let docs_dir = root.join("docs");
        fs::create_dir_all(&docs_dir).expect("create docs");
        let existing = docs_dir.join("existing.mmd");
        fs::write(&existing, "graph TD\nExisting-->Node\n").expect("write existing");

        let artifact = discover_mermaid(&ArtifactDiscoveryContext {
            session_id: "sess-mermaid".to_string(),
            tmux_name: "29".to_string(),
            cwd: root.to_string_lossy().into_owned(),
            session_started_at: Utc::now() + Duration::seconds(1),
            pane_tail: "Added docs/existing.mmd\n".to_string(),
        });

        assert!(artifact.is_none());
    }

    #[test]
    fn mermaid_discovery_returns_none_when_no_candidates_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = discover_mermaid(&ArtifactDiscoveryContext {
            session_id: "sess-mermaid".to_string(),
            tmux_name: "29".to_string(),
            cwd: dir.path().to_string_lossy().into_owned(),
            session_started_at: Utc::now(),
            pane_tail: "working on docs/notes.md".to_string(),
        });

        assert!(artifact.is_none());
    }

    #[test]
    fn extract_slice_name_matches_schema_path_patterns() {
        let cases = [
            (
                "plans released",
                "/home/user/skillbox-config/clients/personal/plans/released/journal_to_cm/schema.mmd",
                Some("journal_to_cm"),
            ),
            (
                "plans draft",
                "/home/user/skillbox-config/clients/personal/plans/draft/persistence_topology/schema.mmd",
                Some("persistence_topology"),
            ),
            (
                "arbitrary released parent",
                "/home/user/repos/project/releases/released/customer_sync/schema.mmd",
                Some("customer_sync"),
            ),
            (
                "arbitrary draft parent",
                "/home/user/repos/project/planning/draft/checkout_flow/schema.mmd",
                Some("checkout_flow"),
            ),
            (
                "arbitrary planned parent",
                "/home/user/repos/project/src/data/db-schemas/planned/agent_billing/schema.mmd",
                Some("agent_billing"),
            ),
            (
                "non-schema filename under plans",
                "/home/user/repos/project/plans/released/customer_sync/diagram.mmd",
                None,
            ),
            (
                "non-schema filename under arbitrary parent",
                "/home/user/repos/project/planning/draft/checkout_flow/diagram.mmd",
                None,
            ),
            ("short plans path", "plans/released/schema.mmd", None),
            ("short status path", "planned/agent_billing/schema.mmd", None),
            (
                "schema outside known paths",
                "clients/personal/skills/domain-planner/assets/templates/schema.mmd",
                None,
            ),
            ("no-match mmd", "/some/repo/docs/architecture.mmd", None),
        ];

        for (label, path, expected) in cases {
            assert_eq!(super::extract_mmd_slice_name(path), expected, "{label}");
        }
    }

    #[test]
    fn extract_slice_name_prefers_plans_pattern_before_status_pattern() {
        let path =
            "/repo/released/status_first/schema.mmd/archive/plans/draft/plans_first/schema.mmd";
        assert_eq!(super::extract_mmd_slice_name(path), Some("plans_first"));
    }

    #[test]
    fn extract_slice_name_returns_slice_borrowed_from_input_path() {
        let path = String::from(
            "/home/user/skillbox-config/clients/personal/plans/released/owned_slice/schema.mmd",
        );
        let slice = super::extract_mmd_slice_name(&path).expect("slice name");
        let base = path.as_ptr() as usize;
        let slice_start = slice.as_ptr() as usize;

        assert_eq!(slice, "owned_slice");
        assert_eq!(
            slice_start.checked_sub(base),
            Some(path.find("owned_slice").expect("slice offset"))
        );
    }

    #[test]
    fn has_plan_siblings_detects_plan_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_dir = dir.path().join("my_slice");
        fs::create_dir_all(&plan_dir).expect("create plan dir");
        fs::write(plan_dir.join("schema.mmd"), "erDiagram\n").expect("write schema");
        fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("write plan");

        assert!(super::has_plan_siblings(&plan_dir.join("schema.mmd")));
    }

    #[test]
    fn has_plan_siblings_returns_false_without_siblings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lone_dir = dir.path().join("lonely");
        fs::create_dir_all(&lone_dir).expect("create dir");
        fs::write(lone_dir.join("diagram.mmd"), "graph TD\n").expect("write mmd");

        assert!(!super::has_plan_siblings(&lone_dir.join("diagram.mmd")));
    }

    #[test]
    fn plan_siblings_bypass_time_filter_in_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_dir = dir
            .path()
            .join("db-schemas")
            .join("planned")
            .join("test_slice");
        fs::create_dir_all(&plan_dir).expect("create plan dir");
        fs::write(plan_dir.join("schema.mmd"), "erDiagram\n").expect("write schema");
        fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("write plan");
        fs::write(plan_dir.join("shared.md"), "# Shared\n").expect("write shared");

        // Session starts AFTER the files were written — normally they'd be filtered out
        let session_started_at = chrono::Utc::now() + Duration::seconds(60);

        let artifact = discover_mermaid(&ArtifactDiscoveryContext {
            session_id: "sess-plan".to_string(),
            tmux_name: "99".to_string(),
            cwd: dir.path().to_string_lossy().into_owned(),
            session_started_at,
            pane_tail: String::new(),
        })
        .expect("artifact should be found despite time filter");

        assert!(artifact.path.ends_with("schema.mmd"));
        assert_eq!(
            super::extract_mmd_slice_name(&artifact.path),
            Some("test_slice")
        );
    }

    #[test]
    fn list_plan_siblings_finds_existing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_dir = dir.path().join("plans").join("draft").join("test_slice");
        fs::create_dir_all(&plan_dir).expect("create plan dir");
        fs::write(plan_dir.join("schema.mmd"), "erDiagram\n").expect("write schema");
        fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("write plan");
        fs::write(plan_dir.join("shared.md"), "# Shared\n").expect("write shared");
        fs::write(plan_dir.join("unrelated.txt"), "nope\n").expect("write unrelated");

        let schema_path = plan_dir.join("schema.mmd");
        let siblings = super::list_plan_siblings(&schema_path.to_string_lossy());
        assert_eq!(siblings, vec!["plan.md", "shared.md"]);
    }

    #[test]
    fn list_plan_siblings_returns_empty_for_no_siblings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_dir = dir.path().join("plans").join("draft").join("lonely");
        fs::create_dir_all(&plan_dir).expect("create plan dir");
        fs::write(plan_dir.join("schema.mmd"), "erDiagram\n").expect("write schema");

        let schema_path = plan_dir.join("schema.mmd");
        let siblings = super::list_plan_siblings(&schema_path.to_string_lossy());
        assert!(siblings.is_empty());
    }

    #[test]
    fn mermaid_discovery_omits_oversized_source_with_clear_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_dir = dir.path().join("plans").join("draft").join("huge");
        fs::create_dir_all(&plan_dir).expect("create plan dir");
        let schema_path = plan_dir.join("schema.mmd");
        fs::write(
            &schema_path,
            format!("graph TD\n{}", "A-->B\n".repeat(12_000)),
        )
        .expect("write large schema");
        fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("write plan sibling");

        let artifact = discover_mermaid(&ArtifactDiscoveryContext {
            session_id: "sess-huge".to_string(),
            tmux_name: "12".to_string(),
            cwd: dir.path().to_string_lossy().into_owned(),
            session_started_at: Utc::now() + Duration::seconds(60),
            pane_tail: String::new(),
        })
        .expect("artifact metadata should still be available");

        assert_eq!(artifact.path, schema_path.to_string_lossy());
        assert!(artifact.source.is_none());
        assert!(artifact
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("exceeds 64 KiB limit"));
    }

    #[test]
    fn mermaid_discovery_reports_bounded_scan_when_many_artifacts_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let docs_dir = dir.path().join("docs");
        fs::create_dir_all(&docs_dir).expect("create docs dir");
        let session_started_at = Utc::now();
        std::thread::sleep(std::time::Duration::from_millis(25));
        for index in 0..(super::MERMAID_SCAN_MAX_FILES + 8) {
            fs::write(
                docs_dir.join(format!("diagram-{index:04}.mmd")),
                format!("graph TD\nA{index}-->B{index}\n"),
            )
            .expect("write mmd");
        }

        let artifact = discover_mermaid(&ArtifactDiscoveryContext {
            session_id: "sess-many".to_string(),
            tmux_name: "18".to_string(),
            cwd: dir.path().to_string_lossy().into_owned(),
            session_started_at,
            pane_tail: String::new(),
        })
        .expect("bounded scan still returns an artifact");

        assert!(artifact.source.is_some());
        assert!(artifact
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("scan reached"));
    }

    #[test]
    fn scan_bounded_mermaid_dirs_merges_plan_dirs_in_input_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first_dir = dir.path().join("first");
        let second_dir = dir.path().join("second");
        fs::create_dir_all(&first_dir).expect("create first dir");
        fs::create_dir_all(&second_dir).expect("create second dir");
        let first_path = first_dir.join("first.mmd");
        let second_path = second_dir.join("second.mmd");
        fs::write(&first_path, "graph TD\nA-->B\n").expect("write first mmd");
        fs::write(&second_path, "graph TD\nB-->C\n").expect("write second mmd");

        let scan = super::scan_bounded_mermaid_dirs(&[first_dir, second_dir]);
        let paths = scan
            .candidates
            .iter()
            .map(|candidate| candidate.display_path.clone())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec![first_path, second_path]);
        assert!(!scan.truncated);
    }

    #[test]
    fn scan_bounded_mermaid_dirs_stops_after_aggregate_cap_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capped_dir = dir.path().join("capped");
        let skipped_dir = dir.path().join("skipped");
        fs::create_dir_all(&capped_dir).expect("create capped dir");
        fs::create_dir_all(&skipped_dir).expect("create skipped dir");
        for index in 0..super::MERMAID_SCAN_MAX_FILES {
            fs::write(
                capped_dir.join(format!("diagram-{index:04}.mmd")),
                format!("graph TD\nA{index}-->B{index}\n"),
            )
            .expect("write capped mmd");
        }
        let skipped_path = skipped_dir.join("skipped.mmd");
        fs::write(&skipped_path, "graph TD\nSkipped-->Node\n").expect("write skipped mmd");

        let scan = super::scan_bounded_mermaid_dirs(&[capped_dir, skipped_dir]);

        assert_eq!(scan.candidates.len(), super::MERMAID_SCAN_MAX_FILES);
        assert!(scan.truncated);
        assert!(!scan
            .candidates
            .iter()
            .any(|candidate| candidate.display_path == skipped_path));
    }

    #[test]
    fn append_bounded_mermaid_scan_ors_child_truncation_without_aggregate_cap() {
        let mut aggregate = super::MermaidScanResult {
            candidates: vec![mermaid_candidate("first.mmd")],
            truncated: false,
        };
        let child = super::MermaidScanResult {
            candidates: vec![mermaid_candidate("second.mmd")],
            truncated: true,
        };

        let reached_cap = super::append_bounded_mermaid_scan(&mut aggregate, child);

        assert!(!reached_cap);
        assert!(aggregate.truncated);
        assert_eq!(
            aggregate
                .candidates
                .iter()
                .map(|candidate| candidate.display_path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["first.mmd", "second.mmd"]
        );
    }

    #[test]
    fn append_bounded_mermaid_scan_truncates_to_aggregate_cap() {
        let mut aggregate = super::MermaidScanResult {
            candidates: (0..(super::MERMAID_SCAN_MAX_FILES - 1))
                .map(|index| mermaid_candidate(&format!("kept-{index:04}.mmd")))
                .collect(),
            truncated: false,
        };
        let child = super::MermaidScanResult {
            candidates: vec![
                mermaid_candidate("kept-last.mmd"),
                mermaid_candidate("dropped.mmd"),
            ],
            truncated: false,
        };

        let reached_cap = super::append_bounded_mermaid_scan(&mut aggregate, child);

        assert!(reached_cap);
        assert!(aggregate.truncated);
        assert_eq!(aggregate.candidates.len(), super::MERMAID_SCAN_MAX_FILES);
        assert_eq!(
            aggregate
                .candidates
                .last()
                .expect("last candidate")
                .display_path,
            std::path::PathBuf::from("kept-last.mmd")
        );
    }

    #[test]
    fn resolve_repo_root_falls_back_from_file_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        let nested_dir = dir.path().join("src").join("bin");
        fs::create_dir_all(&nested_dir).expect("create nested dir");
        let file_cwd = nested_dir.join("main.rs");
        fs::write(&file_cwd, "fn main() {}\n").expect("write file cwd");

        assert_eq!(
            resolve_repo_root(&file_cwd.to_string_lossy()).as_deref(),
            Some(dir.path())
        );
    }

    #[test]
    fn resolve_repo_root_recognizes_supported_markers() {
        enum MarkerKind {
            Exists,
            File,
            Dir,
        }

        let cases = [
            (".git", MarkerKind::Exists),
            ("Cargo.toml", MarkerKind::File),
            ("package.json", MarkerKind::File),
            (".swimmers", MarkerKind::Dir),
            (".throngterm", MarkerKind::Dir),
        ];

        for (name, kind) in cases {
            let dir = tempfile::tempdir().expect("tempdir");
            let marker_path = dir.path().join(name);
            match kind {
                MarkerKind::Exists | MarkerKind::File => {
                    fs::write(&marker_path, "").expect("write marker");
                }
                MarkerKind::Dir => {
                    fs::create_dir(&marker_path).expect("create marker dir");
                }
            }

            assert_eq!(
                resolve_repo_root(&dir.path().to_string_lossy()).as_deref(),
                Some(dir.path()),
                "{name} should mark a repo root"
            );
        }
    }

    #[test]
    fn resolve_repo_root_returns_nearest_marked_ancestor() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"outer\"\n",
        )
        .expect("write outer cargo");
        let inner = dir.path().join("workspace").join("app");
        fs::create_dir_all(inner.join("src").join("nested")).expect("create nested");
        fs::write(inner.join("package.json"), "{}\n").expect("write inner package");

        assert_eq!(
            resolve_repo_root(&inner.join("src").join("nested").to_string_lossy()).as_deref(),
            Some(inner.as_path())
        );
    }

    #[test]
    fn resolve_repo_root_returns_none_without_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("src").join("nested");
        fs::create_dir_all(&nested).expect("create nested");

        assert!(resolve_repo_root(&nested.to_string_lossy()).is_none());
        assert!(resolve_repo_root(&nested.join("missing.rs").to_string_lossy()).is_none());
    }

    #[test]
    fn list_repo_docs_prefers_repo_root_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        fs::write(dir.path().join("README.md"), "# Demo\n").expect("write readme");
        fs::create_dir_all(dir.path().join("docs")).expect("create docs");
        fs::write(dir.path().join("docs").join("VISION.md"), "# Vision\n").expect("write vision");
        fs::create_dir_all(dir.path().join("src").join("nested")).expect("create nested");

        let docs = super::list_repo_docs(&dir.path().join("src").join("nested").to_string_lossy());
        assert_eq!(docs, vec!["README.md", "VISION.md"]);
    }

    #[test]
    fn resolve_viewer_text_path_finds_repo_docs() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        fs::write(dir.path().join("README.md"), "# Demo\n").expect("write readme");
        fs::create_dir_all(dir.path().join("docs")).expect("create docs");
        fs::write(dir.path().join("docs").join("VISION.md"), "# Vision\n").expect("write vision");

        let readme = resolve_viewer_text_path(&dir.path().to_string_lossy(), None, "README.md")
            .expect("readme path");
        let vision = resolve_viewer_text_path(&dir.path().to_string_lossy(), None, "VISION.md")
            .expect("vision path");

        assert_eq!(readme, dir.path().join("README.md"));
        assert_eq!(vision, dir.path().join("docs").join("VISION.md"));
        assert_eq!(
            resolve_repo_root(&dir.path().to_string_lossy()).as_deref(),
            Some(dir.path())
        );
    }

    fn mermaid_candidate(path: &str) -> super::MermaidCandidate {
        super::MermaidCandidate {
            display_path: std::path::PathBuf::from(path),
            updated_at: Utc::now(),
        }
    }
}
