use super::*;
use crate::api::PublishedSelectionState;
use crate::auth::{OBSERVER_SCOPES, OPERATOR_SCOPES};
use crate::config::Config;
use crate::session::actor::ActorHandle;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::protocol::{SyncRequestSequence, ThoughtDeliveryState};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{
    ErrorResponse, RestState, SessionGroupInputRequest, SessionPaneTailResponse,
    SessionTranscriptRecord, StateEvidence, TerminalSnapshot, ThoughtSource, ThoughtState,
    TransportHealth, MAX_SESSION_INPUT_BYTES,
};
use axum::body::to_bytes;
use axum::extract::{Json, Path, Query, State};
use axum::response::IntoResponse;
use chrono::Utc;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};
use tokio::sync::{mpsc, RwLock};

fn p95_duration(mut samples: Vec<Duration>) -> Duration {
    assert!(!samples.is_empty(), "p95 requires at least one sample");
    samples.sort_unstable();
    let index = samples
        .len()
        .saturating_mul(95)
        .div_ceil(100)
        .saturating_sub(1);
    samples[index]
}

fn test_state() -> Arc<AppState> {
    let config = Arc::new(Config::default());
    let supervisor = SessionSupervisor::new(config.clone());
    Arc::new(AppState {
        supervisor,
        config,
        thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
        native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
        ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
        sync_request_sequence: Arc::new(SyncRequestSequence::new()),
        daemon_defaults: crate::api::once_lock_with(None),
        file_store: crate::api::once_lock_with(None),
        bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
            std::time::Duration::from_secs(15),
        )),
        published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
        repo_actions: crate::host_actions::RepoActionTracker::default(),
    })
}

fn summary(session_id: &str, state: SessionState) -> crate::types::SessionSummary {
    let state_evidence = match state {
        SessionState::Busy => StateEvidence::new("osc133_command"),
        SessionState::Exited => StateEvidence::new("process_exit"),
        _ => StateEvidence::new("osc133_prompt"),
    };
    crate::types::SessionSummary {
        session_id: session_id.to_string(),
        tmux_name: format!("tmux-{session_id}"),
        state,
        current_command: None,
        state_evidence,
        cwd: "/tmp/project".to_string(),
        tool: Some("Codex".to_string()),
        token_count: 0,
        context_limit: 192_000,
        thought: None,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        thought_updated_at: None,
        rest_state: crate::types::fallback_rest_state(state, ThoughtState::Holding),
        commit_candidate: false,
        action_cues: Vec::new(),
        objective_changed_at: None,
        last_skill: None,
        is_stale: false,
        attached_clients: 0,
        stale_attached_clients: 0,
        transport_health: TransportHealth::Healthy,
        last_activity_at: Utc::now(),
        repo_theme_id: None,
        batch: None,
        environment: Default::default(),
    }
}

fn with_test_batch(mut summary: SessionSummary, batch_id: &str) -> SessionSummary {
    summary.batch = Some(session_batch_membership(
        batch_id.to_string(),
        "test batch".to_string(),
        0,
        2,
        Utc::now(),
        Some("continue".to_string()),
    ));
    summary
}

async fn insert_summary_test_handle(
    state: &Arc<AppState>,
    summary: SessionSummary,
) -> mpsc::Receiver<Vec<u8>> {
    let session_id = summary.session_id.clone();
    let tmux_name = summary.tmux_name.clone();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let (write_tx, write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = write_tx.send(data).await;
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                }
                _ => {}
            }
        }
    });
    write_rx
}

async fn insert_group_input_delivery_test_handle(
    state: &Arc<AppState>,
    summary: SessionSummary,
    delivery: Option<InputDeliveryResult>,
) -> mpsc::Receiver<Vec<u8>> {
    let session_id = summary.session_id.clone();
    let tmux_name = summary.tmux_name.clone();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let (write_tx, write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary.clone());
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = write_tx.send(data).await;
                    if let Some(delivery) = delivery.clone() {
                        let _ = ack.send(delivery);
                    }
                }
                _ => {}
            }
        }
    });
    write_rx
}

async fn insert_dropping_summary_test_handle(
    state: &Arc<AppState>,
    session_id: &str,
) -> tokio::task::JoinHandle<()> {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            session_id,
            &format!("tmux-{session_id}"),
            cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        if let Some(SessionCommand::GetSummary(reply)) = cmd_rx.recv().await {
            drop(reply);
        }
    })
}

async fn insert_timeline_test_handle(
    state: &Arc<AppState>,
    summary: SessionSummary,
    pane_tail: String,
    artifact: MermaidArtifactResponse,
) {
    let session_id = summary.session_id.clone();
    let tmux_name = summary.tmux_name.clone();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary.clone());
                }
                SessionCommand::GetPaneTail { lines, reply } => {
                    assert_eq!(lines, PANE_TAIL_LINES);
                    let _ = reply.send(pane_tail.clone());
                }
                SessionCommand::GetMermaidArtifact(reply) => {
                    let _ = reply.send(artifact.clone());
                }
                _ => {}
            }
        }
    });
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&body).expect("json body")
}

fn run_git(repo: &FsPath, args: &[&str], description: &str) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap_or_else(|err| panic!("{description}: {err}"));
    assert!(status.success(), "{description} should succeed");
}

fn init_git_repo(repo: &FsPath) {
    run_git(repo, &["init", "-q"], "git init");
}

fn stage_git_file(repo: &FsPath, path: &str) {
    run_git(repo, &["add", path], "git add");
}

fn seed_app_git_diff(repo: &FsPath) {
    init_git_repo(repo);
    std::fs::write(repo.join("app.txt"), "before\n").expect("write app");
    stage_git_file(repo, "app.txt");
    std::fs::write(repo.join("app.txt"), "before\nafter\n").expect("modify app");
}

async fn git_diff_json_for_session_cwd(session_id: &str, cwd: &FsPath) -> Value {
    let state = test_state();
    let mut session = summary(session_id, SessionState::Idle);
    session.cwd = cwd.to_string_lossy().into_owned();
    let _write_rx = insert_summary_test_handle(&state, session).await;

    let response = get_git_diff(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path(session_id.to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

fn expected_repo_root(repo: &FsPath) -> String {
    std::fs::canonicalize(repo)
        .unwrap_or_else(|_| repo.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn assert_session_repo_diff_response(json: &Value, session_id: &str, repo: &FsPath) {
    assert_eq!(json["session_id"], session_id);
    assert_eq!(json["available"], true);
    assert_eq!(
        json["repo_root"].as_str().unwrap(),
        expected_repo_root(repo)
    );
    assert!(json["status_short"].as_str().unwrap().contains("app.txt"));
    assert!(json["staged_diff"].as_str().unwrap().contains("new file"));
    assert!(json["unstaged_diff"].as_str().unwrap().contains("+after"));
    let files = structured_diff_files(json);
    assert_staged_added_app_file(files);
    assert_unstaged_modified_app_file(files);
}

fn structured_diff_files(json: &Value) -> &[Value] {
    json["files"].as_array().expect("structured diff files")
}

fn assert_staged_added_app_file(files: &[Value]) {
    let file = find_app_diff_file(files, "staged", "added");
    assert!(file["added_lines"].as_u64().expect("added lines") >= 1);
    assert!(!file["hunks"].as_array().expect("hunks").is_empty());
}

fn assert_unstaged_modified_app_file(files: &[Value]) {
    let file = find_app_diff_file(files, "unstaged", "modified");
    assert_eq!(file["added_lines"], 1);
}

fn find_app_diff_file<'a>(files: &'a [Value], source: &str, change: &str) -> &'a Value {
    files
        .iter()
        .find(|file| {
            file["path"] == "app.txt" && file["source"] == source && file["change"] == change
        })
        .unwrap_or_else(|| panic!("missing {source} {change} app.txt diff file"))
}

fn agent_context_fixture(session_id: &str) -> SessionAgentContextResponse {
    SessionAgentContextResponse {
        session_id: session_id.to_string(),
        available: true,
        tool: Some("Codex".to_string()),
        cwd: "/monoserver/opensource/swimmers".to_string(),
        user_task: Some("remote task".to_string()),
        turns: Vec::new(),
        current_tool: None,
        recent_actions: Vec::new(),
        token_count: 42,
        context_limit: 192_000,
        message: None,
    }
}

fn remote_agent_context_target(base_url: String) -> LaunchTargetSummary {
    LaunchTargetSummary {
        id: "remote-test".to_string(),
        label: "Remote Test".to_string(),
        kind: "swimmers_api".to_string(),
        base_url: Some(base_url),
        auth_token_env: None,
        bootstrap_hint: None,
        path_mappings: Vec::new(),
    }
}

async fn remote_agent_context_ok(
    Path(session_id): Path<String>,
) -> Json<SessionAgentContextResponse> {
    Json(agent_context_fixture(&session_id))
}

async fn remote_agent_context_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::with_message(
            "SESSION_NOT_FOUND",
            "missing remote session",
        )),
    )
        .into_response()
}

async fn spawn_remote_agent_context_ok_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/sessions/{session_id}/agent-context",
        axum::routing::get(remote_agent_context_ok),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote context server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve remote context api");
    });
    (format!("http://{addr}"), handle)
}

async fn spawn_remote_agent_context_error_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/sessions/{session_id}/agent-context",
        axum::routing::get(remote_agent_context_not_found),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote context server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve remote context api");
    });
    (format!("http://{addr}"), handle)
}

async fn remote_pane_tail_ok(Path(session_id): Path<String>) -> Json<SessionPaneTailResponse> {
    Json(SessionPaneTailResponse {
        session_id,
        text: "remote pane output".to_string(),
    })
}

async fn spawn_remote_pane_tail_ok_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/sessions/{session_id}/pane-tail",
        axum::routing::get(remote_pane_tail_ok),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote pane-tail server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve remote pane-tail api");
    });
    (format!("http://{addr}"), handle)
}

async fn remote_snapshot_ok(Path(session_id): Path<String>) -> Json<TerminalSnapshot> {
    Json(TerminalSnapshot {
        session_id,
        latest_seq: 17,
        truncated: false,
        screen_text: "remote screen output".to_string(),
    })
}

async fn spawn_remote_snapshot_ok_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/sessions/{session_id}/snapshot",
        axum::routing::get(remote_snapshot_ok),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote snapshot server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve remote snapshot api");
    });
    (format!("http://{addr}"), handle)
}

fn transcript_fixture(session_id: &str) -> SessionTranscriptResponse {
    SessionTranscriptResponse {
        session_id: session_id.to_string(),
        available: true,
        tool: Some("Codex".to_string()),
        cwd: "/remote/project".to_string(),
        selected_turn_id: None,
        selected_turn: None,
        next_cursor: 0,
        records: Vec::new(),
        turns: Vec::new(),
        message: None,
    }
}

async fn remote_transcript_ok(
    Path(session_id): Path<String>,
    Query(query): Query<TranscriptQuery>,
) -> Json<SessionTranscriptResponse> {
    let turn_id = query.turn_id;
    let after = query.after.unwrap_or_default();
    let limit = query.limit.unwrap_or_default();
    let mut response = transcript_fixture(&session_id);
    response.selected_turn_id = turn_id;
    response.next_cursor = after;
    response.records.push(SessionTranscriptRecord {
        id: "remote-record".to_string(),
        source: "jsonl".to_string(),
        kind: "query_echo".to_string(),
        role: None,
        summary: limit.to_string(),
        raw: "{}".to_string(),
        byte_start: after,
        byte_end: limit as u64,
        timestamp: None,
        truncated: false,
    });
    Json(response)
}

async fn remote_transcript_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::with_message(
            "SESSION_NOT_FOUND",
            "missing remote transcript",
        )),
    )
        .into_response()
}

async fn spawn_remote_transcript_ok_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/sessions/{session_id}/transcript",
        axum::routing::get(remote_transcript_ok),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote transcript server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve remote transcript api");
    });
    (format!("http://{addr}"), handle)
}

async fn spawn_remote_transcript_error_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/v1/sessions/{session_id}/transcript",
        axum::routing::get(remote_transcript_not_found),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote transcript server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve remote transcript api");
    });
    (format!("http://{addr}"), handle)
}

struct TestPathGuard(Option<OsString>);

impl Drop for TestPathGuard {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }
}

struct TestEnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl TestEnvVarGuard {
    fn set_path(key: &'static str, value: &FsPath) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for TestEnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn write_executable(path: &FsPath, contents: &str) {
    std::fs::write(path, contents).expect("write executable");
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

fn prepend_test_path(bin_dir: &FsPath, original_path: Option<&OsStr>) {
    let mut entries = vec![bin_dir.as_os_str().to_os_string()];
    if let Some(existing) = original_path {
        entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
    }
    std::env::set_var("PATH", std::env::join_paths(entries).expect("path"));
}

fn install_fake_tmux(script: &str) -> (TempDir, TestPathGuard) {
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin");
    write_executable(&bin_dir.join("tmux"), script);
    let original_path = std::env::var_os("PATH");
    prepend_test_path(&bin_dir, original_path.as_deref());
    (dir, TestPathGuard(original_path))
}

const FAKE_TMUX_FOR_CREATE: &str = r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  new-session|attach-session)
    while IFS= read -r line; do
      printf '%s\r\n' "$line"
    done
    ;;
  send-keys|kill-session)
    exit 0
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_path}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_CWD:-/tmp/project}" ;;
      "#{pane_current_command}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_COMMAND:-zsh}" ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
  capture-pane)
    printf 'captured pane\n'
    ;;
  list-sessions)
    exit 0
    ;;
esac
"##;

fn generated_dir_name_sets() -> Vec<Vec<String>> {
    let mut runner = TestRunner::deterministic();
    let name = proptest::string::string_regex("[a-z]{1,8}").expect("valid regex");
    let strategy = proptest::collection::btree_set(name, 1..=4);
    (0..4)
        .map(|_| {
            strategy
                .new_tree(&mut runner)
                .expect("generate dir names")
                .current()
                .into_iter()
                .collect()
        })
        .collect()
}

fn create_case_dirs(root: &FsPath, case_index: usize, names: &[String]) -> Vec<String> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let path = root.join(format!("case-{case_index}-{index}-{name}"));
            std::fs::create_dir_all(&path).expect("create test cwd");
            path.to_string_lossy().into_owned()
        })
        .collect()
}

async fn create_batch(state: Arc<AppState>, dirs: Vec<String>) -> axum::response::Response {
    create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(CreateSessionsBatchRequest {
            dirs,
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response()
}

async fn cleanup_created_sessions(state: &Arc<AppState>, json: &Value) {
    let Some(results) = json["results"].as_array() else {
        return;
    };
    for result in results {
        let Some(session_id) = result["session"]["session_id"].as_str() else {
            continue;
        };
        let _ = state
            .supervisor
            .delete_session(session_id, crate::config::SessionDeleteMode::DetachBridge)
            .await;
    }
}

fn cwd_result_classes(json: &Value) -> BTreeMap<String, bool> {
    json["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|result| {
            (
                result["cwd"].as_str().expect("cwd").to_string(),
                result["ok"].as_bool().expect("ok"),
            )
        })
        .collect()
}

fn success_count(json: &Value) -> usize {
    json["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter(|result| result["ok"].as_bool() == Some(true))
        .count()
}

async fn spawn_summary_handle(summary: crate::types::SessionSummary) -> ActorHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle(
        summary.session_id.clone(),
        summary.tmux_name.clone(),
        cmd_tx,
    );
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary.clone());
                }
                SessionCommand::Shutdown => break,
                _ => {}
            }
        }
    });
    handle
}

fn artifact_response(
    available: bool,
    path: Option<&str>,
    source: Option<&str>,
    error: Option<&str>,
    plan_files: Option<Vec<&str>>,
) -> MermaidArtifactResponse {
    MermaidArtifactResponse {
        session_id: "sess-artifact".to_string(),
        available,
        path: path.map(str::to_string),
        updated_at: None,
        source: source.map(str::to_string),
        error: error.map(str::to_string),
        slice_name: None,
        plan_files: plan_files.map(|files| files.into_iter().map(str::to_string).collect()),
    }
}

fn appended_artifact_payload(
    artifact: Option<&MermaidArtifactResponse>,
) -> (SessionTimelineEvent, SessionTimelinePinnedItem) {
    let mut builder = TimelineBuilder::default();
    let mut pinned = SessionTimelinePinned::default();

    append_artifact_event(&mut builder, &mut pinned, artifact);

    assert_eq!(builder.events.len(), 1);
    (
        builder.events.remove(0),
        pinned.artifact.expect("artifact pinned item"),
    )
}

#[path = "tests/batch_create.rs"]
mod batch_create;
#[path = "tests/context_transcript.rs"]
mod context_transcript;
#[path = "tests/group_input.rs"]
mod group_input;
#[path = "tests/send_input.rs"]
mod send_input;
#[path = "tests/session_lifecycle.rs"]
mod session_lifecycle;
#[path = "tests/terminal_routes.rs"]
mod terminal_routes;
#[path = "tests/timeline_git_diff.rs"]
mod timeline_git_diff;
