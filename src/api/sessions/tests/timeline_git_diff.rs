use super::*;

#[tokio::test]
async fn get_timeline_returns_ordered_events_and_pinned_summaries() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempdir().expect("tempdir");
    let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
    let repo = tempdir().expect("repo tempdir");
    let init = std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["init", "-q"])
        .status()
        .expect("git init");
    assert!(init.success(), "git init should succeed");
    std::fs::write(repo.path().join("app.txt"), "before\n").expect("write app");
    let add = std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["add", "app.txt"])
        .status()
        .expect("git add");
    assert!(add.success(), "git add should succeed");
    std::fs::write(repo.path().join("app.txt"), "before\nafter\n").expect("modify app");

    let cwd = repo.path().to_string_lossy().into_owned();
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("05")
        .join("08");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let jsonl = [
            json!({"type": "session_meta", "payload": {"cwd": cwd}}).to_string(),
            json!({"type": "response_item", "payload": {"role": "user", "content": [{"type": "input_text", "text": "build the workbench"}]}}).to_string(),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec", "arguments": "{\"command\":\"cargo test timeline\"}"}}).to_string(),
        ]
        .join("\n");
    std::fs::write(
        sessions_dir.join("rollout-timeline-target.jsonl"),
        format!("{jsonl}\n"),
    )
    .expect("timeline jsonl");

    let state = test_state();
    let mut session = summary("sess-timeline", SessionState::Idle);
    session.cwd = cwd.clone();
    insert_timeline_test_handle(
        &state,
        session,
        "cargo test\nfinished green\n".to_string(),
        MermaidArtifactResponse {
            session_id: "sess-timeline".to_string(),
            available: true,
            path: Some("/tmp/project/docs/plan.mmd".to_string()),
            updated_at: Some(Utc::now()),
            source: Some("flowchart TD; A-->B".to_string()),
            error: None,
            slice_name: None,
            plan_files: Some(vec!["plan.md".to_string(), "WORKGRAPH.md".to_string()]),
        },
    )
    .await;

    let response = get_timeline(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-timeline".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-timeline");
    assert_eq!(json["available"], true);
    assert_eq!(json["cwd"], cwd);
    assert_eq!(json["pinned"]["task"]["summary"], "build the workbench");
    assert_eq!(json["pinned"]["current_action"]["title"], "exec");
    assert_eq!(json["pinned"]["diff"]["summary"], "dirty");
    assert_eq!(json["pinned"]["pane_tail"]["summary"], "2 lines");
    assert_eq!(json["pinned"]["artifact"]["summary"], "2 plan files");
    let events = json["events"].as_array().expect("timeline events");
    assert!(events.iter().any(|event| event["kind"] == "task"));
    assert!(events.iter().any(|event| event["kind"] == "tool_call"));
    assert!(events.iter().any(|event| event["kind"] == "diff"));
    assert!(events.iter().any(|event| event["kind"] == "pane_tail"));
    assert!(events.iter().any(|event| event["kind"] == "artifact"));
    let orders = events
        .iter()
        .map(|event| event["order"].as_u64().expect("event order"))
        .collect::<Vec<_>>();
    let sorted = {
        let mut sorted = orders.clone();
        sorted.sort_unstable();
        sorted
    };
    assert_eq!(orders, sorted);
}

#[test]
fn git_diff_timeline_summary_and_detail_cover_available_states() {
    let response = |available: bool,
                    status_short: &str,
                    staged_diff: &str,
                    unstaged_diff: &str,
                    truncated: bool,
                    message: Option<&str>| {
        SessionGitDiffResponse {
            session_id: "sess-diff".to_string(),
            available,
            cwd: "/tmp/project".to_string(),
            repo_root: Some("/tmp/project".to_string()),
            status_short: status_short.to_string(),
            staged_diff: staged_diff.to_string(),
            unstaged_diff: unstaged_diff.to_string(),
            truncated,
            message: message.map(str::to_string),
            files: Vec::new(),
        }
    };

    let clean = response(true, "", "", "", false, None);
    assert_eq!(git_diff_timeline_summary(&clean), "clean");
    assert_eq!(git_diff_timeline_detail(&clean), None);

    let dirty = response(
        true,
        " M app.txt",
        "",
        "diff --git a/app.txt b/app.txt\n@@ -1 +1 @@\n-old\n+new\n",
        false,
        None,
    );
    assert_eq!(git_diff_timeline_summary(&dirty), "dirty");
    let dirty_detail = git_diff_timeline_detail(&dirty).expect("dirty detail");
    assert!(dirty_detail.contains("M app.txt"));
    assert!(dirty_detail.contains("diff --git"));

    let truncated = response(true, "", "diff --git a/lib.rs b/lib.rs\n", "", true, None);
    assert_eq!(git_diff_timeline_summary(&truncated), "dirty, truncated");

    let unavailable = response(false, "", "", "", false, Some("not a git repo"));
    assert_eq!(git_diff_timeline_summary(&unavailable), "not a git repo");

    let unavailable_default = response(false, "", "", "", false, None);
    assert_eq!(
        git_diff_timeline_summary(&unavailable_default),
        "git diff unavailable"
    );
}

#[test]
fn git_diff_has_no_changes_treats_whitespace_only_fields_as_clean() {
    let response = SessionGitDiffResponse {
        session_id: "sess-diff".to_string(),
        available: true,
        cwd: "/tmp/project".to_string(),
        repo_root: Some("/tmp/project".to_string()),
        status_short: " \n\t".to_string(),
        staged_diff: "\n".to_string(),
        unstaged_diff: "\t".to_string(),
        truncated: false,
        message: None,
        files: Vec::new(),
    };

    assert!(git_diff_has_no_changes(&response));
}

#[test]
fn git_diff_has_no_changes_detects_each_dirty_field() {
    let response =
        |status_short: &str, staged_diff: &str, unstaged_diff: &str| SessionGitDiffResponse {
            session_id: "sess-diff".to_string(),
            available: true,
            cwd: "/tmp/project".to_string(),
            repo_root: Some("/tmp/project".to_string()),
            status_short: status_short.to_string(),
            staged_diff: staged_diff.to_string(),
            unstaged_diff: unstaged_diff.to_string(),
            truncated: false,
            message: None,
            files: Vec::new(),
        };

    assert!(!git_diff_has_no_changes(&response(" M app.txt", "", "")));
    assert!(!git_diff_has_no_changes(&response("", "diff --git", "")));
    assert!(!git_diff_has_no_changes(&response("", "", "diff --git")));
}

#[tokio::test]
async fn get_timeline_keeps_working_without_structured_context() {
    let state = test_state();
    let tmp = tempdir().expect("tempdir");
    let mut session = summary("sess-shell-timeline", SessionState::Idle);
    session.cwd = tmp.path().to_string_lossy().into_owned();
    session.tool = Some("shell".to_string());
    insert_timeline_test_handle(
        &state,
        session,
        "shell output\n".to_string(),
        MermaidArtifactResponse {
            session_id: "sess-shell-timeline".to_string(),
            available: false,
            path: None,
            updated_at: None,
            source: None,
            error: Some("no artifact".to_string()),
            slice_name: None,
            plan_files: None,
        },
    )
    .await;

    let response = get_timeline(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-shell-timeline".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["available"], true);
    assert_eq!(json["pinned"]["pane_tail"]["summary"], "1 lines");
    let events = json["events"].as_array().expect("timeline events");
    assert!(events
        .iter()
        .any(|event| event["id"] == "context-unavailable"));
    assert!(events.iter().any(|event| event["kind"] == "diff"));
    assert!(events.iter().any(|event| event["kind"] == "artifact"));
}

#[test]
fn append_artifact_event_preserves_event_shape_order_and_pinned_plan_summary() {
    let artifact = artifact_response(
        true,
        Some("/tmp/project/docs/plan.mmd"),
        Some("flowchart TD; A-->B"),
        None,
        Some(vec!["plan.md", "WORKGRAPH.md"]),
    );

    let (event, pinned) = appended_artifact_payload(Some(&artifact));

    assert_eq!(event.id, "artifact");
    assert_eq!(event.kind, "artifact");
    assert_eq!(event.source, "mermaid-artifact");
    assert_eq!(event.title, "Artifacts");
    assert_eq!(event.summary, "2 plan files");
    assert_eq!(event.detail.as_deref(), Some("flowchart TD; A-->B"));
    assert_eq!(event.timestamp, None);
    assert_eq!(event.order, Some(1));
    assert_eq!(pinned.title, "Artifacts");
    assert_eq!(pinned.summary, "2 plan files");
    assert_eq!(pinned.source, "mermaid-artifact");
    assert_eq!(pinned.event_id.as_deref(), Some("artifact"));
}

#[test]
fn append_artifact_event_uses_path_for_available_artifact_without_plan_files() {
    let artifact = artifact_response(
        true,
        Some("/tmp/project/docs/plan.mmd"),
        Some("flowchart TD; A-->B"),
        None,
        None,
    );

    let (event, pinned) = appended_artifact_payload(Some(&artifact));

    assert_eq!(event.summary, "/tmp/project/docs/plan.mmd");
    assert_eq!(event.detail.as_deref(), Some("flowchart TD; A-->B"));
    assert_eq!(pinned.summary, "/tmp/project/docs/plan.mmd");
}

#[test]
fn append_artifact_event_uses_default_available_summary_without_path_or_plan_files() {
    let artifact = artifact_response(true, None, None, None, None);

    let (event, pinned) = appended_artifact_payload(Some(&artifact));

    assert_eq!(event.summary, "artifact available");
    assert_eq!(event.detail, None);
    assert_eq!(pinned.summary, "artifact available");
}

#[test]
fn append_artifact_event_uses_error_for_unavailable_artifact() {
    let artifact = artifact_response(
        false,
        None,
        Some("ignored source"),
        Some("no artifact"),
        None,
    );

    let (event, pinned) = appended_artifact_payload(Some(&artifact));

    assert_eq!(event.summary, "no artifact");
    assert_eq!(event.detail, None);
    assert_eq!(pinned.summary, "no artifact");
}

#[test]
fn append_artifact_event_uses_default_unavailable_summary_without_artifact_or_error() {
    let artifact = artifact_response(false, None, None, None, None);

    let (event, pinned) = appended_artifact_payload(Some(&artifact));
    assert_eq!(event.summary, "artifact unavailable");
    assert_eq!(event.detail, None);
    assert_eq!(pinned.summary, "artifact unavailable");

    let (missing_event, missing_pinned) = appended_artifact_payload(None);
    assert_eq!(missing_event.summary, "artifact unavailable");
    assert_eq!(missing_event.detail, None);
    assert_eq!(missing_pinned.summary, "artifact unavailable");
}

#[tokio::test]
async fn get_timeline_returns_not_found_for_missing_session() {
    let response = get_timeline(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("missing-timeline".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn get_timeline_prefers_remote_namespace_errors() {
    let response = get_timeline(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path(remote_sessions::namespace_session_id(
            "not-configured-timeline-target",
            "shadow",
        )),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("not-configured-timeline-target"));
}

#[tokio::test]
async fn get_timeline_reports_summary_lookup_failure() {
    let state = test_state();
    let summary_task = insert_dropping_summary_test_handle(&state, "dropped-timeline").await;

    let response = get_timeline(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("dropped-timeline".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    summary_task.await.expect("summary task");
}

#[tokio::test]
async fn get_git_diff_returns_session_repo_diff() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let repo = tempdir().expect("repo tempdir");
    seed_app_git_diff(repo.path());

    let json = git_diff_json_for_session_cwd("sess-diff", repo.path()).await;
    assert_session_repo_diff_response(&json, "sess-diff", repo.path());
}

#[tokio::test]
async fn get_git_diff_returns_empty_structured_files_for_clean_repo() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let repo = tempdir().expect("repo tempdir");
    init_git_repo(repo.path());

    let json = git_diff_json_for_session_cwd("sess-clean-diff", repo.path()).await;
    assert_eq!(json["available"], true);
    assert_eq!(json["status_short"], "");
    assert_eq!(json["staged_diff"], "");
    assert_eq!(json["unstaged_diff"], "");
    assert!(json["files"].as_array().expect("files").is_empty());
}

#[tokio::test]
async fn get_git_diff_returns_unavailable_for_non_repo() {
    let tmp = tempdir().expect("tempdir");

    let json = git_diff_json_for_session_cwd("sess-no-repo", tmp.path()).await;
    assert_eq!(json["available"], false);
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("repo root unavailable"));
}
