use std::cmp::Ordering;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use walkdir::WalkDir;

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

        let candidates = scan_mermaid_candidates(root);
        let best = candidates
            .iter()
            .filter(|candidate| candidate.updated_at >= context.session_started_at)
            .max_by(|left, right| compare_mermaid_candidates(left, right))
            .cloned()?;

        let (source, error) = match fs::read_to_string(&best.display_path) {
            Ok(source) => (Some(source), None),
            Err(err) => (
                None,
                Some(format!("failed to read Mermaid artifact: {err}")),
            ),
        };

        Some(DiscoveredArtifact {
            kind: ArtifactKind::Mermaid,
            path: best.display_path.to_string_lossy().into_owned(),
            updated_at: best.updated_at,
            source,
            error,
        })
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

fn compare_mermaid_candidates(left: &MermaidCandidate, right: &MermaidCandidate) -> Ordering {
    left.updated_at
        .cmp(&right.updated_at)
        .then_with(|| left.display_path.cmp(&right.display_path))
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
        default_artifact_registry, ArtifactDiscoveryContext, ArtifactKind, DiscoveredArtifact,
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
}
