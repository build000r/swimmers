use super::*;
use crate::api::PublishedSelectionState;
use crate::auth::{OBSERVER_SCOPES, OPERATOR_SCOPES};
use crate::config::Config;
use crate::session::actor::ActorHandle;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::protocol::{SyncRequestSequence, ThoughtDeliveryState};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{
    ErrorResponse, RestState, SessionGroupInputRequest, SessionTranscriptRecord, StateEvidence,
    ThoughtSource, ThoughtState, TransportHealth,
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

#[tokio::test]
async fn create_session_requires_write_scope() {
    let response = create_session(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionRequest {
            name: None,
            cwd: None,
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn adopt_session_requires_write_scope() {
    let response = adopt_session(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(AdoptSessionRequest {
            tmux_name: "alpha".to_string(),
            session_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn adopt_session_rejects_already_tracked_tmux_without_duplication() {
    let state = test_state();
    let active = summary("sess-1", SessionState::Idle);
    let tmux_name = active.tmux_name.clone();
    let _rx = insert_summary_test_handle(&state, active.clone()).await;

    let response = adopt_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(AdoptSessionRequest {
            tmux_name,
            session_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = response_json(response).await;
    assert_eq!(json["code"], "TMUX_SESSION_ALREADY_TRACKED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("sess-1"));
}

#[tokio::test]
async fn create_session_rejects_unknown_non_local_launch_target_explicitly() {
    let response = create_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionRequest {
            name: None,
            // Remote launch now requires an explicit cwd; supply the current
            // dir (what launch_cwd used to inject implicitly) so this test
            // still reaches the unknown-launch-target check rather than the
            // missing-cwd validation that would otherwise preempt it.
            cwd: Some(
                std::env::current_dir()
                    .expect("current dir")
                    .to_string_lossy()
                    .into_owned(),
            ),
            spawn_tool: None,
            launch_target: Some("not-configured-target-for-test".to_string()),
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
    let message = json["message"].as_str().expect("message");
    assert!(
        message.contains("launch target 'not-configured-target-for-test' is not configured")
            || message.contains("no skillbox-config overlay is available"),
        "{message}"
    );
}

#[tokio::test]
async fn create_session_rejects_missing_cwd_as_validation_error() {
    let missing = tempdir().expect("tempdir").path().join("missing");
    let response = create_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionRequest {
            name: None,
            cwd: Some(missing.to_string_lossy().into_owned()),
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("cwd does not exist"));
}

#[tokio::test]
async fn create_sessions_batch_requires_write_scope() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: vec!["/tmp/project".to_string()],
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_sessions_batch_rejects_empty_dirs() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: Vec::new(),
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not be empty");
}

#[tokio::test]
async fn create_remote_sessions_batch_response_maps_validation_errors() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: Vec::new(),
            spawn_tool: None,
            launch_target: Some("remote-target".to_string()),
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not be empty");
}

#[tokio::test]
async fn create_sessions_batch_rejects_oversized_batches() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: (0..=BATCH_CREATE_MAX_DIRS)
                .map(|idx| format!("/tmp/project-{idx}"))
                .collect(),
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(
        json["message"],
        format!("dirs must include at most {BATCH_CREATE_MAX_DIRS} entries")
    );
}

#[tokio::test]
async fn create_sessions_batch_assigns_shared_batch_metadata() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);

    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(CreateSessionsBatchRequest {
            dirs,
            spawn_tool: None,
            launch_target: None,
            initial_request: Some("wire jwt refresh + tests".to_string()),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CREATED);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results");
    let first_batch = &results[0]["session"]["batch"];
    let second_batch = &results[1]["session"]["batch"];

    assert!(first_batch["id"]
        .as_str()
        .expect("batch id")
        .starts_with("batch-"));
    assert_eq!(second_batch["id"], first_batch["id"]);
    assert_eq!(first_batch["label"], "wire jwt refresh + tests");
    assert_eq!(first_batch["prompt_excerpt"], "wire jwt refresh + tests");
    assert_eq!(first_batch["index"], 0);
    assert_eq!(second_batch["index"], 1);
    assert_eq!(first_batch["total"], 2);
    assert_eq!(second_batch["total"], 2);
    assert!(first_batch["created_at"].is_string());

    cleanup_created_sessions(&state, &json).await;
}

#[tokio::test]
async fn create_sessions_batch_mr_permutation_preserves_cwd_result_classes() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");

    for (case_index, names) in generated_dir_name_sets().into_iter().enumerate() {
        let dirs = create_case_dirs(root.path(), case_index, &names);
        let reversed_dirs = dirs.iter().rev().cloned().collect::<Vec<_>>();

        let response = create_batch(state.clone(), dirs.clone()).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let forward_json = response_json(response).await;

        let response = create_batch(state.clone(), reversed_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let reversed_json = response_json(response).await;

        assert_eq!(
            cwd_result_classes(&forward_json),
            cwd_result_classes(&reversed_json)
        );

        cleanup_created_sessions(&state, &forward_json).await;
        cleanup_created_sessions(&state, &reversed_json).await;
    }
}

#[tokio::test]
async fn create_sessions_batch_mr_additive_valid_dir_increases_success_count() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let base_dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
    let mut extended_dirs = base_dirs.clone();
    extended_dirs.extend(create_case_dirs(root.path(), 1, &["docs".to_string()]));

    let response = create_batch(state.clone(), base_dirs).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let base_json = response_json(response).await;

    let response = create_batch(state.clone(), extended_dirs).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let extended_json = response_json(response).await;

    assert_eq!(success_count(&extended_json), success_count(&base_json) + 1);
    assert_eq!(
        extended_json["results"].as_array().expect("results").len(),
        base_json["results"].as_array().expect("results").len() + 1
    );

    cleanup_created_sessions(&state, &base_json).await;
    cleanup_created_sessions(&state, &extended_json).await;
}

#[tokio::test]
async fn create_sessions_batch_mr_invalid_dir_injection_is_exclusive() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let valid_dirs = create_case_dirs(
        root.path(),
        0,
        &["frontend".to_string(), "backend".to_string()],
    );
    let missing_dir = root.path().join("missing").to_string_lossy().into_owned();
    let dirs = vec![
        valid_dirs[0].clone(),
        missing_dir.clone(),
        valid_dirs[1].clone(),
    ];

    let response = create_batch(state.clone(), dirs).await;
    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results");

    assert_eq!(results.len(), 3);
    assert_eq!(success_count(&json), 2);
    assert_eq!(results[1]["index"], 1);
    assert_eq!(results[1]["cwd"], missing_dir);
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["error"]["code"], "VALIDATION_FAILED");
    assert!(results[0]["session"]["session_id"].is_string());
    assert!(results[2]["session"]["session_id"].is_string());

    cleanup_created_sessions(&state, &json).await;
}

#[tokio::test]
async fn delete_session_rejects_invalid_mode() {
    let response = delete_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
        Query(DeleteSessionQuery {
            mode: Some("invalid".to_string()),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
}

#[test]
fn delete_session_mode_parse_accepts_supported_modes() {
    assert!(matches!(
        parse_delete_session_mode(None),
        Ok(SessionDeleteMode::DetachBridge)
    ));
    assert!(matches!(
        parse_delete_session_mode(Some("detach_bridge")),
        Ok(SessionDeleteMode::DetachBridge)
    ));
    assert!(matches!(
        parse_delete_session_mode(Some("kill_tmux")),
        Ok(SessionDeleteMode::KillTmux)
    ));
}

#[tokio::test]
async fn delete_session_returns_not_found_for_missing_session() {
    let response = delete_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
        Query(DeleteSessionQuery { mode: None }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
    assert_eq!(json["message"], Value::Null);
}

#[tokio::test]
async fn delete_session_error_response_maps_internal_errors() {
    let response = delete_session_error_response(anyhow::anyhow!("tmux kill failed"));

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("tmux kill failed"));
}

#[tokio::test]
async fn send_input_rejects_empty_text() {
    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: String::new(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
}

#[tokio::test]
async fn send_input_returns_not_found_for_missing_session() {
    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn send_input_rejects_exited_session() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-exited", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let SessionCommand::GetSummary(reply) = cmd {
                let _ = reply.send(summary("sess-exited", SessionState::Exited));
                return;
            }
        }
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-exited".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_EXITED");
    worker.await.expect("worker");
}

#[tokio::test]
async fn send_input_forwards_text_to_session_actor() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                    return data;
                }
                _ => {}
            }
        }
        Vec::new()
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(worker.await.expect("worker"), b"status".to_vec());
}

#[tokio::test]
async fn send_input_submit_forwards_submit_line_to_session_actor() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::SubmitLineAck { text, ack } => {
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                    return text;
                }
                _ => {}
            }
        }
        String::new()
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: true,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(worker.await.expect("worker"), "status");
}

#[tokio::test]
async fn send_input_reports_failed_delivery_ack() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::WriteInputAck { ack, .. } => {
                    let _ = ack.send(InputDeliveryResult {
                        delivered: false,
                        method: "test",
                        message: Some("pty write failed".to_string()),
                    });
                    return;
                }
                _ => {}
            }
        }
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_DELIVERY_FAILED");
    assert_eq!(json["message"], "pty write failed");
    worker.await.expect("worker");
}

#[tokio::test]
async fn send_input_reports_dropped_delivery_ack() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::WriteInputAck { ack, .. } => {
                    drop(ack);
                    return;
                }
                _ => {}
            }
        }
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_DELIVERY_UNKNOWN");
    worker.await.expect("worker");
}

#[tokio::test]
async fn send_input_delivery_response_returns_success_payload() {
    let response = session_input_delivery_response(
        "sess-1".to_string(),
        InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        },
    );

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["session_id"], "sess-1");
    assert_eq!(json["delivered"], true);
    assert_eq!(json["delivery_method"], "test");
    assert_eq!(json["message"], Value::Null);
}

#[tokio::test]
async fn send_input_delivery_response_maps_failed_delivery() {
    let response = session_input_delivery_response(
        "sess-1".to_string(),
        InputDeliveryResult {
            delivered: false,
            method: "test",
            message: Some("pty write failed".to_string()),
        },
    );

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_DELIVERY_FAILED");
    assert_eq!(json["message"], "pty write failed");
}

#[tokio::test]
async fn remote_agent_context_response_returns_namespaced_success() {
    let (base_url, handle) = spawn_remote_agent_context_ok_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_agent_context_response(&target, "sess/remote?x#frag").await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "remote-test::sess/remote?x#frag");
    assert_eq!(json["available"], true);
    assert_eq!(json["user_task"], "remote task");
    handle.abort();
}

#[tokio::test]
async fn remote_agent_context_response_maps_remote_failure() {
    let (base_url, handle) = spawn_remote_agent_context_error_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_agent_context_response(&target, "missing-remote").await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "REMOTE_SESSION_REQUEST_FAILED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("missing remote session"));
    handle.abort();
}

#[tokio::test]
async fn get_agent_context_prefers_remote_namespace_error_over_local_session() {
    let state = test_state();
    let session_id =
        remote_sessions::namespace_session_id("not-configured-agent-context-target", "shadow");
    let _write_rx =
        insert_summary_test_handle(&state, summary(&session_id, SessionState::Idle)).await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path(session_id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
}

#[tokio::test]
async fn get_agent_context_returns_codex_jsonl_snapshot() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempdir().expect("tempdir");
    let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("05")
        .join("07");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    std::fs::write(
            sessions_dir.join("rollout-target.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"build the workbench\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"command\\\":\\\"cargo test agent_context\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":777}},\"model_context_window\":258400}}\n"
            ),
        )
        .expect("target rollout");

    let state = test_state();
    let _write_rx =
        insert_summary_test_handle(&state, summary("sess-context", SessionState::Idle)).await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-context".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-context");
    assert_eq!(json["available"], true);
    assert_eq!(json["tool"], "Codex");
    assert_eq!(json["cwd"], "/tmp/project");
    assert_eq!(json["user_task"], "build the workbench");
    assert_eq!(json["turns"].as_array().unwrap().len(), 1);
    assert_eq!(json["turns"][0]["text"], "build the workbench");
    assert_eq!(json["current_tool"]["tool"], "exec");
    assert_eq!(json["current_tool"]["detail"], "cargo test agent_context");
    assert_eq!(json["recent_actions"][0]["tool"], "exec");
    assert_eq!(json["token_count"], 777);
    assert_eq!(json["context_limit"], 258400);
}

#[tokio::test]
async fn agent_context_read_response_returns_ok_for_successful_read() {
    let response = agent_context_read_response(Ok(agent_context_fixture("sess-read-ok")));

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-read-ok");
    assert_eq!(json["available"], true);
    assert_eq!(json["user_task"], "remote task");
}

#[tokio::test]
async fn get_agent_context_returns_internal_error_when_summary_lookup_fails() {
    let state = test_state();
    let worker = insert_dropping_summary_test_handle(&state, "sess-summary-error").await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-summary-error".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("session summary actor dropped reply"));
    worker.await.expect("summary worker");
}

#[tokio::test]
async fn agent_context_read_response_returns_internal_error_for_read_failure() {
    let response = agent_context_read_response(Err(anyhow::anyhow!("context read failed")));

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "context read failed");
}

#[tokio::test]
async fn fetch_transcript_remote_response_returns_namespaced_success() {
    let (base_url, handle) = spawn_remote_transcript_ok_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_transcript_response(
        &target,
        "remote-ready",
        TranscriptQuery {
            turn_id: Some("turn-1".to_string()),
            after: Some(7),
            limit: Some(3),
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "remote-test::remote-ready");
    assert_eq!(json["selected_turn_id"], "turn-1");
    assert_eq!(json["next_cursor"], 7);
    assert_eq!(json["records"][0]["byte_start"], 7);
    assert_eq!(json["records"][0]["byte_end"], 3);
    handle.abort();
}

#[tokio::test]
async fn fetch_transcript_remote_response_maps_remote_failure() {
    let (base_url, handle) = spawn_remote_transcript_error_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_transcript_response(
        &target,
        "missing-remote",
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "REMOTE_SESSION_REQUEST_FAILED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("missing remote transcript"));
    handle.abort();
}

#[tokio::test]
async fn fetch_transcript_response_prefers_remote_namespace_error_over_local_session() {
    let state = test_state();
    let session_id =
        remote_sessions::namespace_session_id("not-configured-transcript-target", "shadow");
    let _write_rx =
        insert_summary_test_handle(&state, summary(&session_id, SessionState::Idle)).await;

    let response = fetch_transcript_response(
        &state,
        &session_id,
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
}

#[tokio::test]
async fn fetch_transcript_response_returns_not_found_for_missing_local_session() {
    let response = fetch_transcript_response(
        &test_state(),
        "missing-transcript",
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn fetch_transcript_response_returns_internal_error_when_summary_lookup_fails() {
    let state = test_state();
    let worker = insert_dropping_summary_test_handle(&state, "sess-summary-error").await;

    let response = fetch_transcript_response(
        &state,
        "sess-summary-error",
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("session summary actor dropped reply"));
    worker.await.expect("summary worker");
}

#[tokio::test]
async fn fetch_transcript_read_response_returns_ok_for_successful_read() {
    let response = transcript_read_response(Ok(transcript_fixture("sess-read-ok")));

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-read-ok");
    assert_eq!(json["available"], true);
    assert_eq!(json["cwd"], "/remote/project");
}

#[tokio::test]
async fn fetch_transcript_read_response_returns_internal_error_for_read_failure() {
    let response = transcript_read_response(Err(anyhow::anyhow!("transcript read failed")));

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "transcript read failed");
}

#[tokio::test]
async fn fetch_transcript_get_returns_records_after_selected_user_turn() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempdir().expect("tempdir");
    let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("05")
        .join("10");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    std::fs::write(
            sessions_dir.join("rollout-transcript.jsonl"),
            [
                json!({"type": "session_meta", "payload": {"cwd": "/tmp/project"}}).to_string(),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>skip me</environment_context>"}]}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "first turn"}}).to_string(),
                json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec", "arguments": "{\"command\":\"cargo test first\"}"}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "second turn"}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "working after second"}}).to_string(),
            ]
            .join("\n")
                + "\n",
        )
        .expect("target rollout");

    let state = test_state();
    let _write_rx =
        insert_summary_test_handle(&state, summary("sess-transcript", SessionState::Idle)).await;

    let context_response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Path("sess-transcript".to_string()),
    )
    .await;
    assert_eq!(context_response.status(), StatusCode::OK);
    let context_json = response_json(context_response).await;
    let turns = context_json["turns"].as_array().expect("turns");
    assert_eq!(
        turns
            .iter()
            .map(|turn| turn["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["first turn", "second turn"]
    );
    assert!(
        !turns.iter().any(|turn| turn["text"]
            .as_str()
            .unwrap()
            .contains("environment_context")),
        "system/environment records must not appear as user turns"
    );

    let first_turn_id = turns[0]["id"].as_str().unwrap().to_string();
    let response = get_transcript(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-transcript".to_string()),
        Query(TranscriptQuery {
            turn_id: Some(first_turn_id),
            after: None,
            limit: Some(10),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["available"], true);
    assert_eq!(json["selected_turn"]["text"], "first turn");
    let records = json["records"].as_array().expect("records");
    assert_eq!(records[0]["kind"], "function_call");
    assert!(records[0]["summary"]
        .as_str()
        .unwrap()
        .contains("cargo test first"));
    assert!(
        records
            .iter()
            .any(|record| record["summary"].as_str().unwrap().contains("second turn")),
        "stream should include later JSONL records after the selected turn"
    );
    assert!(json["next_cursor"].as_u64().unwrap() > turns[0]["byte_end"].as_u64().unwrap());
}

#[tokio::test]
async fn get_agent_context_returns_unavailable_for_unsupported_tool() {
    let state = test_state();
    let mut unsupported = summary("sess-shell", SessionState::Idle);
    unsupported.tool = Some("shell".to_string());
    unsupported.context_limit = 0;
    let _write_rx = insert_summary_test_handle(&state, unsupported).await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-shell".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-shell");
    assert_eq!(json["available"], false);
    assert_eq!(json["tool"], "shell");
    assert_eq!(json["recent_actions"].as_array().unwrap().len(), 0);
    assert!(json["message"].as_str().unwrap().contains("not supported"));
}

#[tokio::test]
async fn get_agent_context_returns_not_found_for_missing_session() {
    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("missing-context".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

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

#[tokio::test]
async fn send_group_input_rejects_empty_session_ids() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: Vec::new(),
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "session_ids must not be empty");
}

#[tokio::test]
async fn send_group_input_rejects_whitespace_text() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["one".to_string(), "two".to_string()],
            text: " \n\t ".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "text must not be empty");
}

#[tokio::test]
async fn send_group_input_rejects_fewer_than_two_unique_session_ids() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["only".to_string(), "only".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(
        json["message"],
        "session_ids must include at least two unique sessions"
    );
}

#[tokio::test]
async fn send_group_input_returns_not_found_for_all_missing_sessions() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["missing-a".to_string(), "missing-b".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["skipped"], 2);
    let results = json["results"].as_array().expect("results");
    assert_eq!(results[0]["session_id"], "missing-a");
    assert_eq!(results[0]["ok"], false);
    assert_eq!(results[0]["error"]["code"], "SESSION_NOT_FOUND");
    assert_eq!(results[1]["session_id"], "missing-b");
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["error"]["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn send_group_input_sends_only_ready_sessions() {
    let state = test_state();

    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let mut busy = with_test_batch(summary("busy", SessionState::Busy), "batch-group");
    busy.rest_state = RestState::Active;

    let (ready_cmd_tx, mut ready_cmd_rx) = mpsc::channel(8);
    let (ready_write_tx, mut ready_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "ready",
            "tmux-ready",
            ready_cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = ready_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(ready.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = ready_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = ready_write_tx.send(data).await;
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

    let (busy_cmd_tx, mut busy_cmd_rx) = mpsc::channel(8);
    let (busy_write_tx, mut busy_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("busy", "tmux-busy", busy_cmd_tx))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = busy_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(busy.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = busy_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = busy_write_tx.send(data).await;
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

    state
        .supervisor
        .persist_thought(
            "ready",
            Some("waiting for direction"),
            0,
            192_000,
            ThoughtState::Sleeping,
            ThoughtSource::Llm,
            RestState::Sleeping,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec![
                "ready".to_string(),
                "ready".to_string(),
                "busy".to_string(),
                "missing".to_string(),
                remote_sessions::namespace_session_id("jeremy-skillbox", "remote-ready"),
            ],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 3);
    let results = json["results"].as_array().expect("results");
    assert_eq!(results.len(), 4, "duplicate session IDs should be deduped");
    assert_eq!(results[3]["session_id"], "jeremy-skillbox::remote-ready");
    assert_eq!(results[3]["ok"], false);
    assert_eq!(results[3]["error"]["code"], "SESSION_NOT_FOUND");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    let duplicate_ready_write =
        tokio::time::timeout(Duration::from_millis(25), ready_write_rx.recv()).await;
    assert!(
        matches!(duplicate_ready_write, Err(_) | Ok(None)),
        "duplicate session IDs must not receive duplicate group input"
    );
    let busy_write = tokio::time::timeout(Duration::from_millis(25), busy_write_rx.recv()).await;
    assert!(
        matches!(busy_write, Err(_) | Ok(None)),
        "busy sessions must not receive group input"
    );
}

#[tokio::test]
async fn send_group_input_reports_failed_delivery_ack() {
    let state = test_state();
    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let failed = with_test_batch(summary("failed", SessionState::Idle), "batch-group");
    let mut ready_write_rx = insert_group_input_delivery_test_handle(
        &state,
        ready,
        Some(InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        }),
    )
    .await;
    let mut failed_write_rx = insert_group_input_delivery_test_handle(
        &state,
        failed,
        Some(InputDeliveryResult {
            delivered: false,
            method: "test",
            message: Some("pty write failed".to_string()),
        }),
    )
    .await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "failed".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(json["results"][1]["session_id"], "failed");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(json["results"][1]["error"]["code"], "INPUT_DELIVERY_FAILED");
    assert_eq!(json["results"][1]["error"]["message"], "pty write failed");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    assert_eq!(
        failed_write_rx.recv().await.expect("failed write"),
        b"continue\r\r".to_vec()
    );
}

#[tokio::test]
async fn send_group_input_reports_actor_unavailable_send_failure() {
    let state = test_state();
    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let mut ready_write_rx = insert_group_input_delivery_test_handle(
        &state,
        ready,
        Some(InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        }),
    )
    .await;

    let unavailable = with_test_batch(summary("unavailable", SessionState::Idle), "batch-group");
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "unavailable",
            "tmux-unavailable",
            cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        if let Some(SessionCommand::GetSummary(reply)) = cmd_rx.recv().await {
            let _ = reply.send(unavailable);
        }
    });

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "unavailable".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(json["results"][1]["session_id"], "unavailable");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_FOUND");
    assert_eq!(json["results"][1]["error"]["message"], "channel closed");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
}

#[tokio::test]
async fn send_group_input_reports_dropped_delivery_ack() {
    let state = test_state();
    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let dropped = with_test_batch(summary("dropped", SessionState::Idle), "batch-group");
    let mut ready_write_rx = insert_group_input_delivery_test_handle(
        &state,
        ready,
        Some(InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        }),
    )
    .await;
    let mut dropped_write_rx = insert_group_input_delivery_test_handle(&state, dropped, None).await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "dropped".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(json["results"][1]["session_id"], "dropped");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(
        json["results"][1]["error"]["code"],
        "INPUT_DELIVERY_UNKNOWN"
    );
    assert_eq!(
        json["results"][1]["error"]["message"],
        "session actor dropped input delivery ack"
    );
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    assert_eq!(
        dropped_write_rx.recv().await.expect("dropped write"),
        b"continue\r\r".to_vec()
    );
}

#[tokio::test]
async fn send_group_input_skips_stale_and_disconnected_sessions() {
    let state = test_state();

    let mut ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    ready.rest_state = RestState::Sleeping;
    let mut stale = with_test_batch(summary("stale", SessionState::Idle), "batch-group");
    stale.rest_state = RestState::Sleeping;
    stale.is_stale = true;
    let mut disconnected =
        with_test_batch(summary("disconnected", SessionState::Idle), "batch-group");
    disconnected.rest_state = RestState::Sleeping;
    disconnected.transport_health = TransportHealth::Disconnected;

    let mut ready_write_rx = insert_summary_test_handle(&state, ready).await;
    let mut stale_write_rx = insert_summary_test_handle(&state, stale).await;
    let mut disconnected_write_rx = insert_summary_test_handle(&state, disconnected).await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec![
                "ready".to_string(),
                "stale".to_string(),
                "disconnected".to_string(),
            ],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 2);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(json["results"][2]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    let stale_write = tokio::time::timeout(Duration::from_millis(25), stale_write_rx.recv()).await;
    assert!(
        matches!(stale_write, Err(_) | Ok(None)),
        "stale sessions must not receive group input"
    );
    let disconnected_write =
        tokio::time::timeout(Duration::from_millis(25), disconnected_write_rx.recv()).await;
    assert!(
        matches!(disconnected_write, Err(_) | Ok(None)),
        "disconnected sessions must not receive group input"
    );
}

#[tokio::test]
async fn send_group_input_skips_degraded_overloaded_and_unobserved_sessions() {
    let state = test_state();

    let mut ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    ready.rest_state = RestState::Sleeping;
    let mut degraded = with_test_batch(summary("degraded", SessionState::Idle), "batch-group");
    degraded.rest_state = RestState::Sleeping;
    degraded.transport_health = TransportHealth::Degraded;
    degraded.state_evidence = StateEvidence::unobserved("summary_cache_degraded");
    let mut overloaded = with_test_batch(
        summary("overloaded", SessionState::Attention),
        "batch-group",
    );
    overloaded.transport_health = TransportHealth::Overloaded;
    overloaded.state_evidence = StateEvidence::unobserved("summary_cache_overloaded");
    let mut unobserved = with_test_batch(summary("unobserved", SessionState::Idle), "batch-group");
    unobserved.rest_state = RestState::Sleeping;
    unobserved.state_evidence = StateEvidence::unobserved("initial_state");

    let mut ready_write_rx = insert_summary_test_handle(&state, ready).await;
    let degraded_write_rx = insert_summary_test_handle(&state, degraded).await;
    let overloaded_write_rx = insert_summary_test_handle(&state, overloaded).await;
    let unobserved_write_rx = insert_summary_test_handle(&state, unobserved).await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec![
                "ready".to_string(),
                "degraded".to_string(),
                "overloaded".to_string(),
                "unobserved".to_string(),
            ],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 3);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(json["results"][2]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(json["results"][3]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    for (mut rx, label) in [
        (degraded_write_rx, "degraded"),
        (overloaded_write_rx, "overloaded"),
        (unobserved_write_rx, "unobserved"),
    ] {
        let write = tokio::time::timeout(Duration::from_millis(25), rx.recv()).await;
        assert!(
            matches!(write, Err(_) | Ok(None)),
            "{label} sessions must not receive group input"
        );
    }
}

#[tokio::test]
async fn send_group_input_rejects_attention_deep_sleep_sessions() {
    let state = test_state();

    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let mut deep_attention = with_test_batch(
        summary("deep-attention", SessionState::Attention),
        "batch-group",
    );
    deep_attention.rest_state = RestState::DeepSleep;

    let (ready_cmd_tx, mut ready_cmd_rx) = mpsc::channel(8);
    let (ready_write_tx, mut ready_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "ready",
            "tmux-ready",
            ready_cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = ready_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(ready.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = ready_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = ready_write_tx.send(data).await;
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

    let (deep_cmd_tx, mut deep_cmd_rx) = mpsc::channel(8);
    let (deep_write_tx, mut deep_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "deep-attention",
            "tmux-deep-attention",
            deep_cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = deep_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(deep_attention.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = deep_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = deep_write_tx.send(data).await;
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

    state
        .supervisor
        .persist_thought(
            "ready",
            Some("waiting for direction"),
            0,
            192_000,
            ThoughtState::Sleeping,
            ThoughtSource::Llm,
            RestState::Sleeping,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "deep-attention".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][1]["session_id"], "deep-attention");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    let deep_write = tokio::time::timeout(Duration::from_millis(25), deep_write_rx.recv()).await;
    assert!(
        matches!(deep_write, Err(_) | Ok(None)),
        "deep sleep sessions must not receive group input"
    );
}

#[tokio::test]
async fn send_group_input_rejects_unbatched_or_mixed_batch_groups() {
    let state = test_state();

    let unbatched = summary("unbatched", SessionState::Idle);
    let batch_a = with_test_batch(summary("batch-a", SessionState::Idle), "batch-a");
    let batch_b = with_test_batch(summary("batch-b", SessionState::Idle), "batch-b");

    for (session_id, summary) in [
        ("unbatched", unbatched),
        ("batch-a", batch_a),
        ("batch-b", batch_b),
    ] {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                session_id,
                format!("tmux-{session_id}"),
                cmd_tx,
            ))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSummary(reply) = cmd {
                    let _ = reply.send(summary.clone());
                }
            }
        });
    }

    let unbatched_response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["unbatched".to_string(), "batch-a".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(unbatched_response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(unbatched_response).await;
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["skipped"], 2);
    assert_eq!(json["results"][0]["error"]["code"], "SESSION_NOT_IN_BATCH");
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_IN_BATCH");

    let mixed_response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["batch-a".to_string(), "batch-b".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(mixed_response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(mixed_response).await;
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["skipped"], 2);
    assert_eq!(
        json["results"][0]["error"]["code"],
        "SESSION_BATCH_MISMATCH"
    );
    assert_eq!(
        json["results"][1]["error"]["code"],
        "SESSION_BATCH_MISMATCH"
    );
}

#[tokio::test]
async fn get_snapshot_returns_actor_snapshot() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-snap", "tmux-snap", cmd_tx))
        .await;

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let SessionCommand::GetSnapshot(reply) = cmd {
                let _ = reply.send(TerminalSnapshot {
                    session_id: "sess-snap".to_string(),
                    latest_seq: 9,
                    truncated: false,
                    screen_text: "hello from tmux".to_string(),
                });
                break;
            }
        }
    });

    let response = get_snapshot(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-snap".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-snap");
    assert_eq!(json["screen_text"], "hello from tmux");
}

#[tokio::test]
async fn get_snapshot_returns_not_found_for_missing_session() {
    let response = get_snapshot(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn get_snapshot_returns_actor_unavailable_error() {
    let state = test_state();
    let (cmd_tx, cmd_rx) = mpsc::channel(1);
    drop(cmd_rx);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-dead", "tmux-dead", cmd_tx))
        .await;

    let response = get_snapshot(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-dead".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "session actor unavailable");
}

#[tokio::test]
async fn request_terminal_snapshot_detects_dropped_reply() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-drop", "tmux-drop", cmd_tx))
        .await;
    tokio::spawn(async move {
        if let Some(SessionCommand::GetSnapshot(reply)) = cmd_rx.recv().await {
            drop(reply);
        }
    });

    let handle = state
        .supervisor
        .get_session("sess-drop")
        .await
        .expect("test handle");
    let err = request_terminal_snapshot(&handle)
        .await
        .expect_err("reply should be dropped");

    assert_eq!(err, SnapshotRequestError::ReplyDropped);
}

#[tokio::test]
async fn snapshot_error_response_maps_timeout_detail() {
    let response = snapshot_error_response(SnapshotRequestError::Timeout);

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "snapshot request timed out");
}

#[tokio::test]
async fn list_sessions_perf_gate_batches_tmux_lookup_within_budget() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, _path_guard) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 0.20
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
work-3	1	1	0.0:%3
work-4	1	1	0.0:%4
work-5	1	1	0.0:%5
work-6	1	1	0.0:%6
EOF
    ;;
  display-message)
    sleep 0.20
    printf '0.0:%%1\n'
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
    );

    let state = test_state();
    let mut expected_ids = Vec::new();
    for index in 1..=6 {
        let session_id = format!("sess-{index}");
        let mut live_summary = summary(&session_id, SessionState::Idle);
        live_summary.tmux_name = format!("work-{index}");
        state
            .supervisor
            .insert_test_handle(spawn_summary_handle(live_summary).await)
            .await;
        expected_ids.push(session_id);
    }
    expected_ids.sort();

    let mut samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let Json(payload) = list_sessions(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state.clone()),
        )
        .await
        .expect("session list should succeed");
        let elapsed = started.elapsed();
        samples.push(elapsed);

        let mut actual_ids = payload
            .sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<Vec<_>>();
        actual_ids.sort();
        assert_eq!(actual_ids, expected_ids);
    }

    let p95 = p95_duration(samples);
    eprintln!("/v1/sessions p95: {p95:?} (budget 500ms)");
    assert!(
        p95 < Duration::from_millis(500),
        "expected /v1/sessions p95 under 500ms, got {:?}",
        p95
    );
}

#[tokio::test]
async fn list_sessions_perf_gate_skips_hung_tmux_active_pane_lookup() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, _path_guard) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 2
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
EOF
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
    );

    let state = test_state();
    let mut expected_ids = Vec::new();
    for index in 1..=2 {
        let session_id = format!("sess-{index}");
        let mut live_summary = summary(&session_id, SessionState::Idle);
        live_summary.tmux_name = format!("work-{index}");
        state
            .supervisor
            .insert_test_handle(spawn_summary_handle(live_summary).await)
            .await;
        expected_ids.push(session_id);
    }

    let started = Instant::now();
    let Json(payload) = list_sessions(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(state),
    )
    .await
    .expect("session list should succeed when tmux stalls");
    let elapsed = started.elapsed();

    let mut actual_ids = payload
        .sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();
    actual_ids.sort();
    expected_ids.sort();

    assert_eq!(actual_ids, expected_ids);
    assert!(
        elapsed < Duration::from_millis(900),
        "expected /v1/sessions to degrade gracefully when tmux list-panes stalls, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn get_pane_tail_returns_actor_text() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx))
        .await;

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let SessionCommand::GetPaneTail { lines, reply } = cmd {
                assert_eq!(lines, 300);
                let _ = reply.send("recent pane output".to_string());
                break;
            }
        }
    });

    let response = get_pane_tail(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-tail".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-tail");
    assert_eq!(json["text"], "recent pane output");
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_actor_text() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    tokio::spawn(async move {
        if let Some(SessionCommand::GetPaneTail { lines, reply }) = cmd_rx.recv().await {
            assert_eq!(lines, PANE_TAIL_LINES);
            let _ = reply.send("recent pane output".to_string());
        }
    });

    let text = request_pane_tail_from_actor(&handle)
        .await
        .expect("pane tail");

    assert_eq!(text, "recent pane output");
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_actor_unavailable_when_send_fails() {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    drop(cmd_rx);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    let result = request_pane_tail_from_actor(&handle).await;

    assert!(matches!(result, Err(PaneTailError::ActorUnavailable)));
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_reply_dropped_when_actor_drops_reply() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    tokio::spawn(async move {
        if let Some(SessionCommand::GetPaneTail { lines, reply }) = cmd_rx.recv().await {
            assert_eq!(lines, PANE_TAIL_LINES);
            drop(reply);
        }
    });

    let result = request_pane_tail_from_actor(&handle).await;

    assert!(matches!(result, Err(PaneTailError::ReplyDropped)));
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_timed_out_when_actor_keeps_reply() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    tokio::spawn(async move {
        if let Some(SessionCommand::GetPaneTail { lines, reply }) = cmd_rx.recv().await {
            assert_eq!(lines, PANE_TAIL_LINES);
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(reply);
        }
    });

    let result = request_pane_tail_from_actor_with_timeout(&handle, Duration::from_millis(1)).await;

    assert!(matches!(result, Err(PaneTailError::TimedOut)));
}

#[tokio::test]
async fn dismiss_attention_requires_write_scope() {
    let response = dismiss_attention(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-1".to_string()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn dismiss_attention_returns_not_found_for_unknown_session() {
    let response = dismiss_attention(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("missing".to_string()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn dismiss_attention_forwards_command_and_returns_ok() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-att", "tmux-att", cmd_tx))
        .await;

    let received = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if matches!(cmd, SessionCommand::DismissAttention) {
                return true;
            }
        }
        false
    });

    let response = dismiss_attention(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-att".to_string()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["ok"], true);
    assert!(
        received.await.expect("worker"),
        "actor never saw DismissAttention"
    );
}
