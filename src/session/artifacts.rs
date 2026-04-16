use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::session::overlay::default_overlay;

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

impl ArtifactDetector for MermaidArtifactDetector {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::Mermaid
    }

    fn discover(&self, context: &ArtifactDiscoveryContext) -> Option<DiscoveredArtifact> {
        let root = context.cwd.trim();
        if root.is_empty() {
            return None;
        }

        // Phase 1: Try skillbox overlay plan directories (no time filter)
        if let Some(overlay) = default_overlay() {
            if let Some(plan_dirs) = overlay.find_plan_dirs(root) {
                let mut overlay_candidates = Vec::new();
                for dir in &plan_dirs {
                    overlay_candidates.extend(scan_mermaid_candidates(&dir.to_string_lossy()));
                }
                if let Some(best) = overlay_candidates
                    .iter()
                    .max_by(|l, r| compare_mermaid_candidates(l, r))
                    .cloned()
                {
                    return Some(read_candidate_artifact(best));
                }
            }
        }

        // Phase 2: In-repo scan with relaxed time filter for plan directories
        let candidates = scan_mermaid_candidates(root);

        // Prefer: plan-directory candidates (no time filter), then time-filtered others
        let plan_best = candidates
            .iter()
            .filter(|c| has_plan_siblings(&c.display_path))
            .max_by(|l, r| compare_mermaid_candidates(l, r))
            .cloned();
        if let Some(best) = plan_best {
            return Some(read_candidate_artifact(best));
        }

        // Original: time-filtered non-plan candidates
        let best = candidates
            .iter()
            .filter(|candidate| candidate.updated_at >= context.session_started_at)
            .max_by(|left, right| compare_mermaid_candidates(left, right))
            .cloned()?;

        Some(read_candidate_artifact(best))
    }
}

fn scan_mermaid_candidates(root: &str) -> Vec<MermaidCandidate> {
    let mut candidates = Vec::new();
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
    }
    candidates
}

fn read_candidate_artifact(candidate: MermaidCandidate) -> DiscoveredArtifact {
    let (source, error) = match fs::read_to_string(&candidate.display_path) {
        Ok(source) => (Some(source), None),
        Err(err) => (
            None,
            Some(format!("failed to read Mermaid artifact: {err}")),
        ),
    };
    DiscoveredArtifact {
        kind: ArtifactKind::Mermaid,
        path: candidate.display_path.to_string_lossy().into_owned(),
        updated_at: candidate.updated_at,
        source,
        error,
    }
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
    // Pattern 1: plans/{released|draft}/{slice}/schema.mmd (original)
    let parts: Vec<&str> = path.split('/').collect();
    for window in parts.windows(4) {
        if window[0] == "plans"
            && (window[1] == "released" || window[1] == "draft")
            && window[3] == "schema.mmd"
        {
            return Some(window[2]);
        }
    }

    // Pattern 2: any {parent}/{released|draft|planned}/{slice}/schema.mmd
    for window in parts.windows(4) {
        if matches!(window[1], "released" | "draft" | "planned") && window[3] == "schema.mmd" {
            return Some(window[2]);
        }
    }

    // Pattern 3: schema.mmd with plan siblings — use parent dir name
    let p = Path::new(path);
    if p.file_name()?.to_str()? == "schema.mmd" && has_plan_siblings(p) {
        return p.parent()?.file_name()?.to_str();
    }

    None
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
    let mut current = Path::new(cwd);
    if !current.is_dir() {
        current = current.parent()?;
    }

    loop {
        if looks_like_repo_root(current) {
            return Some(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return None,
        }
    }
}

fn looks_like_repo_root(path: &Path) -> bool {
    path.join(".git").exists()
        || path.join("Cargo.toml").is_file()
        || path.join("package.json").is_file()
        || path.join(".swimmers").is_dir()
        || path.join(".throngterm").is_dir()
}

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
    fn extract_slice_name_from_released_plan_path() {
        let path =
            "/home/user/skillbox-config/clients/personal/plans/released/journal_to_cm/schema.mmd";
        assert_eq!(super::extract_mmd_slice_name(path), Some("journal_to_cm"));
    }

    #[test]
    fn extract_slice_name_from_draft_plan_path() {
        let path = "/home/user/skillbox-config/clients/personal/plans/draft/persistence_topology/schema.mmd";
        assert_eq!(
            super::extract_mmd_slice_name(path),
            Some("persistence_topology")
        );
    }

    #[test]
    fn extract_slice_name_returns_none_for_non_plan_mmd() {
        assert_eq!(
            super::extract_mmd_slice_name("/some/repo/docs/architecture.mmd"),
            None
        );
    }

    #[test]
    fn extract_slice_name_returns_none_for_template_schema() {
        let path = "clients/personal/skills/domain-planner/assets/templates/schema.mmd";
        assert_eq!(super::extract_mmd_slice_name(path), None);
    }

    #[test]
    fn extract_slice_name_from_planned_path() {
        let path = "/home/user/repos/project/src/data/db-schemas/planned/agent_billing/schema.mmd";
        assert_eq!(super::extract_mmd_slice_name(path), Some("agent_billing"));
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
}
