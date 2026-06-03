use chrono::Utc;

use crate::session::artifacts::{
    default_artifact_registry, extract_mmd_slice_name, list_plan_siblings, list_repo_docs,
    read_text_file_bounded, resolve_viewer_text_path, ArtifactDiscoveryContext, ArtifactKind,
    PLAN_SIBLING_FILENAMES, VIEWER_TEXT_FILENAMES, VIEWER_TEXT_MAX_BYTES,
};
use crate::types::{MermaidArtifactResponse, PlanFileResponse};

pub(crate) async fn build_mermaid_artifact_response(
    session_id: String,
    tmux_name: String,
    cwd: String,
    session_started_at: chrono::DateTime<Utc>,
) -> MermaidArtifactResponse {
    let fallback_session_id = session_id.clone();
    tokio::task::spawn_blocking(move || {
        build_mermaid_artifact_response_sync(session_id, tmux_name, cwd, session_started_at)
    })
    .await
    .unwrap_or_else(|err| MermaidArtifactResponse {
        session_id: fallback_session_id,
        available: false,
        path: None,
        updated_at: None,
        source: None,
        error: Some(format!("artifact scan task failed: {err}")),
        slice_name: None,
        plan_files: None,
    })
}

fn build_mermaid_artifact_response_sync(
    session_id: String,
    tmux_name: String,
    cwd: String,
    session_started_at: chrono::DateTime<Utc>,
) -> MermaidArtifactResponse {
    let context = ArtifactDiscoveryContext {
        session_id: session_id.clone(),
        tmux_name,
        cwd,
        session_started_at,
        pane_tail: String::new(),
    };
    default_artifact_registry()
        .discover(ArtifactKind::Mermaid, &context)
        .map(|artifact| {
            let slice_name = extract_mmd_slice_name(&artifact.path).map(str::to_owned);
            let mut plan_files = list_plan_siblings(&artifact.path);
            plan_files.extend(list_repo_docs(&context.cwd));
            plan_files.dedup();
            let plan_files = (!plan_files.is_empty()).then_some(plan_files);
            MermaidArtifactResponse {
                session_id: session_id.clone(),
                available: true,
                path: Some(artifact.path),
                updated_at: Some(artifact.updated_at),
                source: artifact.source,
                error: artifact.error,
                slice_name,
                plan_files,
            }
        })
        .unwrap_or(MermaidArtifactResponse {
            session_id,
            available: false,
            path: None,
            updated_at: None,
            source: None,
            error: None,
            slice_name: None,
            plan_files: None,
        })
}

pub(crate) async fn build_plan_file_response_async(
    session_id: String,
    cwd: String,
    session_started_at: chrono::DateTime<Utc>,
    name: String,
) -> PlanFileResponse {
    let fallback_session_id = session_id.clone();
    let fallback_name = name.clone();
    tokio::task::spawn_blocking(move || {
        build_plan_file_response(session_id, cwd, session_started_at, &name)
    })
    .await
    .unwrap_or_else(|err| PlanFileResponse {
        session_id: fallback_session_id,
        name: fallback_name,
        content: None,
        error: Some(format!("plan file task failed: {err}")),
    })
}

pub(crate) fn build_plan_file_response(
    session_id: String,
    cwd: String,
    session_started_at: chrono::DateTime<Utc>,
    name: &str,
) -> PlanFileResponse {
    if !VIEWER_TEXT_FILENAMES.contains(&name) {
        return PlanFileResponse {
            session_id,
            name: name.to_string(),
            content: None,
            error: Some(format!("artifact file name not allowed: {name}")),
        };
    }

    let schema_path = if PLAN_SIBLING_FILENAMES.contains(&name) {
        let context = ArtifactDiscoveryContext {
            session_id: session_id.clone(),
            tmux_name: String::new(),
            cwd: cwd.clone(),
            session_started_at,
            pane_tail: String::new(),
        };
        let Some(artifact) = default_artifact_registry().discover(ArtifactKind::Mermaid, &context)
        else {
            return PlanFileResponse {
                session_id,
                name: name.to_string(),
                content: None,
                error: Some("no mermaid artifact found".to_string()),
            };
        };
        Some(artifact.path)
    } else {
        None
    };
    let file_path = match resolve_viewer_text_path(&cwd, schema_path.as_deref(), name) {
        Some(path) => path,
        None => {
            return PlanFileResponse {
                session_id,
                name: name.to_string(),
                content: None,
                error: Some(format!("artifact file unavailable: {name}")),
            };
        }
    };
    match read_text_file_bounded(&file_path, VIEWER_TEXT_MAX_BYTES, "artifact file") {
        Ok(content) => PlanFileResponse {
            session_id,
            name: name.to_string(),
            content: Some(content),
            error: None,
        },
        Err(err) => PlanFileResponse {
            session_id,
            name: name.to_string(),
            content: None,
            error: Some(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::build_plan_file_response;
    use chrono::Utc;

    #[test]
    fn plan_file_response_rejects_path_traversal_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        let plan_dir = dir.path().join("plans").join("draft").join("slice");
        std::fs::create_dir_all(&plan_dir).expect("create plan dir");
        std::fs::write(plan_dir.join("schema.mmd"), "graph TD\nA-->B\n").expect("write schema");
        std::fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("write plan");

        let response = build_plan_file_response(
            "sess-path".to_string(),
            dir.path().to_string_lossy().into_owned(),
            Utc::now() - chrono::Duration::seconds(1),
            "../secret.txt",
        );

        assert!(response.content.is_none());
        assert_eq!(
            response.error.as_deref(),
            Some("artifact file name not allowed: ../secret.txt")
        );
    }

    #[test]
    fn plan_file_response_reads_repo_docs_without_mermaid_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        std::fs::write(dir.path().join("README.md"), "# Demo\n").expect("write readme");
        std::fs::create_dir_all(dir.path().join("docs")).expect("create docs");
        std::fs::write(dir.path().join("docs").join("VISION.md"), "# Vision\n")
            .expect("write vision");

        let readme = build_plan_file_response(
            "sess-docs".to_string(),
            dir.path().to_string_lossy().into_owned(),
            Utc::now(),
            "README.md",
        );
        let vision = build_plan_file_response(
            "sess-docs".to_string(),
            dir.path().to_string_lossy().into_owned(),
            Utc::now(),
            "VISION.md",
        );

        assert_eq!(readme.content.as_deref(), Some("# Demo\n"));
        assert!(readme.error.is_none());
        assert_eq!(vision.content.as_deref(), Some("# Vision\n"));
        assert!(vision.error.is_none());
    }

    #[test]
    fn plan_file_response_still_requires_mermaid_artifact_for_plan_siblings() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        std::fs::write(dir.path().join("plan.md"), "# Root plan\n").expect("write root plan");

        let response = build_plan_file_response(
            "sess-plan".to_string(),
            dir.path().to_string_lossy().into_owned(),
            Utc::now(),
            "plan.md",
        );

        assert!(response.content.is_none());
        assert_eq!(response.error.as_deref(), Some("no mermaid artifact found"));
    }

    #[test]
    fn plan_file_response_omits_oversized_content_with_clear_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .expect("write cargo");
        let plan_dir = dir.path().join("plans").join("draft").join("huge");
        std::fs::create_dir_all(&plan_dir).expect("create plan dir");
        std::fs::write(plan_dir.join("schema.mmd"), "graph TD\nA-->B\n").expect("write schema");
        std::fs::write(
            plan_dir.join("plan.md"),
            "x".repeat(crate::session::artifacts::VIEWER_TEXT_MAX_BYTES as usize + 1),
        )
        .expect("write oversized plan");

        let response = build_plan_file_response(
            "sess-plan".to_string(),
            dir.path().to_string_lossy().into_owned(),
            Utc::now() - chrono::Duration::seconds(1),
            "plan.md",
        );

        assert!(response.content.is_none());
        assert!(response
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("exceeds 128 KiB limit"));
    }
}
