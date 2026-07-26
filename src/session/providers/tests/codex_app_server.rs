use super::*;
use crate::types::ProviderResumeCapability;
use serde_json::{json, Map, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::io::{split, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::time::sleep;

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
enum ThreadMessageOrder {
    ResponseFirst,
    NotificationFirst,
}

struct SuccessFixture {
    thread_start: Value,
    turn_start: Value,
}

fn test_config(label: &str) -> Map<String, Value> {
    serde_json::from_value(json!({
        "model": format!("model-{label}"),
        "sandbox_mode": "workspace-write",
        "nested": {
            "fixture": label,
            "enabled": true
        }
    }))
    .expect("object fixture")
}

async fn read_json(
    lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
) -> Value {
    let line = lines
        .next_line()
        .await
        .expect("fixture read")
        .expect("client message");
    serde_json::from_str(&line).expect("client JSON")
}

async fn write_json(writer: &mut tokio::io::WriteHalf<DuplexStream>, message: Value) {
    writer
        .write_all(serde_json::to_string(&message).unwrap().as_bytes())
        .await
        .expect("fixture write");
    writer.write_all(b"\n").await.expect("fixture newline");
    writer.flush().await.expect("fixture flush");
}

async fn initialize_fixture(
    lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    writer: &mut tokio::io::WriteHalf<DuplexStream>,
) {
    let initialize = read_json(lines).await;
    assert_eq!(initialize["method"], "initialize");
    assert_eq!(initialize["id"], INITIALIZE_REQUEST_ID);
    assert_eq!(initialize["params"]["clientInfo"]["name"], "swimmers");
    assert!(initialize.get("jsonrpc").is_none());
    write_json(
        writer,
        json!({"id": INITIALIZE_REQUEST_ID, "result": {
            "codexHome": "/tmp/codex",
            "platformFamily": "unix",
            "platformOs": "linux",
            "userAgent": "fixture"
        }}),
    )
    .await;

    let initialized = read_json(lines).await;
    assert_eq!(initialized, json!({"method": "initialized", "params": {}}));
}

async fn successful_server(
    stream: DuplexStream,
    thread_id: &'static str,
    turn_id: &'static str,
    order: ThreadMessageOrder,
) -> SuccessFixture {
    let (reader, mut writer) = split(stream);
    let mut lines = BufReader::new(reader).lines();
    initialize_fixture(&mut lines, &mut writer).await;

    let thread_start = read_json(&mut lines).await;
    assert_eq!(thread_start["method"], "thread/start");
    assert_eq!(thread_start["id"], THREAD_START_REQUEST_ID);
    let response = json!({
        "id": THREAD_START_REQUEST_ID,
        "result": {"thread": {"id": thread_id}}
    });
    let notification = json!({
        "method": "thread/started",
        "params": {"thread": {"id": thread_id}}
    });
    match order {
        ThreadMessageOrder::ResponseFirst => {
            write_json(&mut writer, response).await;
            write_json(&mut writer, notification).await;
        }
        ThreadMessageOrder::NotificationFirst => {
            write_json(&mut writer, notification).await;
            write_json(&mut writer, response).await;
        }
    }

    let turn_start = read_json(&mut lines).await;
    assert_eq!(turn_start["method"], "turn/start");
    assert_eq!(turn_start["id"], TURN_START_REQUEST_ID);
    assert_eq!(turn_start["params"]["threadId"], thread_id);
    write_json(
        &mut writer,
        json!({
            "method": "turn/started",
            "params": {
                "threadId": thread_id,
                "turn": {"id": turn_id}
            }
        }),
    )
    .await;
    write_json(
        &mut writer,
        json!({
            "id": TURN_START_REQUEST_ID,
            "result": {"turn": {"id": turn_id}}
        }),
    )
    .await;

    SuccessFixture {
        thread_start,
        turn_start,
    }
}

async fn run_duplex_protocol(
    stream: DuplexStream,
    cwd: &Path,
    config: &Map<String, Value>,
    prompt: &str,
    timeout: Duration,
    cancellation: &CodexAppServerCancellation,
) -> Result<ProtocolLaunch, ProtocolFailure> {
    let (reader, writer) = split(stream);
    let mut transport = JsonlTransport::new(reader, writer);
    run_protocol(
        &mut transport,
        &cwd.to_path_buf(),
        config,
        prompt,
        timeout,
        cancellation,
    )
    .await
}

fn assert_unknown_failure(error: &CodexAppServerLaunchError) {
    assert_eq!(
        error.provider_resume().provider(),
        ProviderResumeProvider::Codex
    );
    assert_eq!(
        error.provider_resume().capability(),
        ProviderResumeCapability::Unknown
    );
    assert!(!error.provider_resume().is_resumable());
    assert_eq!(error.provider_resume().conversation_id(), None);
    assert_eq!(error.provider_resume().resume_argv(), None);
    assert_eq!(error.provider_resume().resume_command(), None);
}

#[tokio::test]
async fn codex_app_server_provider_parallel_starts_keep_exact_ids_cwds_and_configs() {
    let (client_a, server_a) = tokio::io::duplex(32 * 1024);
    let (client_b, server_b) = tokio::io::duplex(32 * 1024);
    let server_a = tokio::spawn(successful_server(
        server_a,
        "thread-alpha",
        "turn-alpha",
        ThreadMessageOrder::ResponseFirst,
    ));
    let server_b = tokio::spawn(successful_server(
        server_b,
        "thread-beta",
        "turn-beta",
        ThreadMessageOrder::NotificationFirst,
    ));
    let config_a = test_config("alpha");
    let config_b = test_config("beta");
    let cancel_a = CodexAppServerCancellation::default();
    let cancel_b = CodexAppServerCancellation::default();
    let cwd_a = PathBuf::from("/tmp/swimmers-alpha");
    let cwd_b = PathBuf::from("/tmp/swimmers-beta");

    let (launch_a, launch_b) = tokio::join!(
        run_duplex_protocol(
            client_a,
            &cwd_a,
            &config_a,
            "alpha prompt",
            TEST_TIMEOUT,
            &cancel_a,
        ),
        run_duplex_protocol(
            client_b,
            &cwd_b,
            &config_b,
            "beta prompt",
            TEST_TIMEOUT,
            &cancel_b,
        )
    );
    let launch_a = launch_a.expect("alpha launch");
    let launch_b = launch_b.expect("beta launch");
    let fixture_a = server_a.await.expect("alpha fixture");
    let fixture_b = server_b.await.expect("beta fixture");

    assert_eq!(
        launch_a.provider_resume.conversation_id(),
        Some("thread-alpha")
    );
    assert_eq!(
        launch_b.provider_resume.conversation_id(),
        Some("thread-beta")
    );
    assert_eq!(launch_a.turn_id, "turn-alpha");
    assert_eq!(launch_b.turn_id, "turn-beta");
    assert_eq!(
        fixture_a.thread_start["params"]["cwd"],
        cwd_a.to_str().unwrap()
    );
    assert_eq!(
        fixture_b.thread_start["params"]["cwd"],
        cwd_b.to_str().unwrap()
    );
    assert_eq!(
        fixture_a.thread_start["params"]["config"],
        Value::Object(config_a)
    );
    assert_eq!(
        fixture_b.thread_start["params"]["config"],
        Value::Object(config_b)
    );
    assert_eq!(
        fixture_a.turn_start["params"]["input"],
        json!([{"type": "text", "text": "alpha prompt"}])
    );
    assert_eq!(
        fixture_b.turn_start["params"]["input"],
        json!([{"type": "text", "text": "beta prompt"}])
    );
}

#[tokio::test]
async fn codex_app_server_provider_response_notification_mismatch_is_unknown() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let fixture = tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        let mut lines = BufReader::new(reader).lines();
        initialize_fixture(&mut lines, &mut writer).await;
        let _ = read_json(&mut lines).await;
        write_json(
            &mut writer,
            json!({
                "id": THREAD_START_REQUEST_ID,
                "result": {"thread": {"id": "thread-response"}}
            }),
        )
        .await;
        write_json(
            &mut writer,
            json!({
                "method": "thread/started",
                "params": {"thread": {"id": "thread-notification"}}
            }),
        )
        .await;
    });
    let cancellation = CodexAppServerCancellation::default();
    let failure = run_duplex_protocol(
        client,
        Path::new("/tmp/swimmers-mismatch"),
        &test_config("mismatch"),
        "prompt",
        TEST_TIMEOUT,
        &cancellation,
    )
    .await
    .expect_err("mismatch must fail")
    .into_launch_error();
    fixture.await.expect("fixture");

    assert!(matches!(
        failure.kind(),
        CodexAppServerError::ThreadCorrelationMismatch {
            response_id,
            notification_id
        } if response_id == "thread-response" && notification_id == "thread-notification"
    ));
    assert_eq!(
        failure.provider_resume().capture_source(),
        ProviderResumeCaptureSource::ProviderNotification
    );
    assert_unknown_failure(&failure);
}

#[tokio::test]
async fn codex_app_server_provider_missing_thread_is_unknown() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let fixture = tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        let mut lines = BufReader::new(reader).lines();
        initialize_fixture(&mut lines, &mut writer).await;
        let _ = read_json(&mut lines).await;
        write_json(
            &mut writer,
            json!({
                "id": THREAD_START_REQUEST_ID,
                "result": {"thread": {}}
            }),
        )
        .await;
    });
    let cancellation = CodexAppServerCancellation::default();
    let failure = run_duplex_protocol(
        client,
        Path::new("/tmp/swimmers-missing"),
        &test_config("missing"),
        "prompt",
        TEST_TIMEOUT,
        &cancellation,
    )
    .await
    .expect_err("missing thread must fail")
    .into_launch_error();
    fixture.await.expect("fixture");

    assert!(matches!(failure.kind(), CodexAppServerError::MissingThread));
    assert_eq!(
        failure.provider_resume().capture_source(),
        ProviderResumeCaptureSource::ProviderResponse
    );
    assert_unknown_failure(&failure);
}

#[tokio::test]
async fn codex_app_server_provider_missing_notification_times_out_unknown() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let fixture = tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        let mut lines = BufReader::new(reader).lines();
        initialize_fixture(&mut lines, &mut writer).await;
        let _ = read_json(&mut lines).await;
        write_json(
            &mut writer,
            json!({
                "id": THREAD_START_REQUEST_ID,
                "result": {"thread": {"id": "thread-timeout"}}
            }),
        )
        .await;
        sleep(Duration::from_millis(100)).await;
    });
    let cancellation = CodexAppServerCancellation::default();
    let failure = run_duplex_protocol(
        client,
        Path::new("/tmp/swimmers-timeout"),
        &test_config("timeout"),
        "prompt",
        Duration::from_millis(20),
        &cancellation,
    )
    .await
    .expect_err("missing notification must time out")
    .into_launch_error();
    fixture.await.expect("fixture");

    assert!(matches!(
        failure.kind(),
        CodexAppServerError::Timeout {
            stage: CodexAppServerStage::ThreadStartedNotification
        }
    ));
    assert_unknown_failure(&failure);
}

#[tokio::test]
async fn codex_app_server_provider_late_notification_cannot_restore_resumable_claim() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let fixture = tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        let mut lines = BufReader::new(reader).lines();
        initialize_fixture(&mut lines, &mut writer).await;
        let _ = read_json(&mut lines).await;
        write_json(
            &mut writer,
            json!({
                "id": THREAD_START_REQUEST_ID,
                "result": {"thread": {"id": "thread-late"}}
            }),
        )
        .await;
        sleep(Duration::from_millis(60)).await;
        let _ = writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "method": "thread/started",
                        "params": {"thread": {"id": "thread-late"}}
                    })
                )
                .as_bytes(),
            )
            .await;
    });
    let cancellation = CodexAppServerCancellation::default();
    let failure = run_duplex_protocol(
        client,
        Path::new("/tmp/swimmers-late"),
        &test_config("late"),
        "prompt",
        Duration::from_millis(15),
        &cancellation,
    )
    .await
    .expect_err("late notification must fail")
    .into_launch_error();
    fixture.await.expect("fixture");

    assert!(matches!(
        failure.kind(),
        CodexAppServerError::Timeout {
            stage: CodexAppServerStage::ThreadStartedNotification
        }
    ));
    assert_unknown_failure(&failure);
}

#[tokio::test]
async fn codex_app_server_provider_cancellation_is_typed_and_unknown() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let fixture = tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        let mut lines = BufReader::new(reader).lines();
        initialize_fixture(&mut lines, &mut writer).await;
        let _ = read_json(&mut lines).await;
        sleep(Duration::from_millis(100)).await;
    });
    let cancellation = CodexAppServerCancellation::default();
    let cancel_signal = cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(15)).await;
        cancel_signal.cancel();
    });
    let failure = run_duplex_protocol(
        client,
        Path::new("/tmp/swimmers-cancel"),
        &test_config("cancel"),
        "prompt",
        TEST_TIMEOUT,
        &cancellation,
    )
    .await
    .expect_err("cancelled launch must fail")
    .into_launch_error();
    cancel_task.await.expect("cancel task");
    fixture.await.expect("fixture");

    assert!(matches!(
        failure.kind(),
        CodexAppServerError::Cancelled {
            stage: CodexAppServerStage::ThreadStartResponse
        }
    ));
    assert_unknown_failure(&failure);
}

#[tokio::test]
async fn codex_app_server_provider_turn_mismatch_blocks_launch_success() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let fixture = tokio::spawn(async move {
        let (reader, mut writer) = split(server);
        let mut lines = BufReader::new(reader).lines();
        initialize_fixture(&mut lines, &mut writer).await;
        let _ = read_json(&mut lines).await;
        write_json(
            &mut writer,
            json!({
                "id": THREAD_START_REQUEST_ID,
                "result": {"thread": {"id": "thread-turn-mismatch"}}
            }),
        )
        .await;
        write_json(
            &mut writer,
            json!({
                "method": "thread/started",
                "params": {"thread": {"id": "thread-turn-mismatch"}}
            }),
        )
        .await;
        let _ = read_json(&mut lines).await;
        write_json(
            &mut writer,
            json!({
                "id": TURN_START_REQUEST_ID,
                "result": {"turn": {"id": "turn-response"}}
            }),
        )
        .await;
        write_json(
            &mut writer,
            json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-turn-mismatch",
                    "turn": {"id": "turn-notification"}
                }
            }),
        )
        .await;
    });
    let cancellation = CodexAppServerCancellation::default();
    let failure = run_duplex_protocol(
        client,
        Path::new("/tmp/swimmers-turn-mismatch"),
        &test_config("turn-mismatch"),
        "prompt",
        TEST_TIMEOUT,
        &cancellation,
    )
    .await
    .expect_err("turn mismatch must fail")
    .into_launch_error();
    fixture.await.expect("fixture");

    assert!(matches!(
        failure.kind(),
        CodexAppServerError::TurnCorrelationMismatch {
            response_turn,
            notification_turn,
            ..
        } if response_turn == "turn-response" && notification_turn == "turn-notification"
    ));
    assert_unknown_failure(&failure);
}

#[tokio::test]
async fn codex_app_server_provider_spawn_and_resume_argv_use_returned_thread_id() {
    let directory = tempfile::tempdir().expect("temp directory");
    let fake_codex = directory.path().join("fake-codex");
    std::fs::write(
        &fake_codex,
        r#"#!/usr/bin/env python3
import json
import sys

assert sys.argv[1:] == ["app-server", "--stdio"]
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
    elif method == "thread/start":
        thread_id = "thread-from-response"
        print(json.dumps({"method": "thread/started", "params": {"thread": {"id": thread_id}}}), flush=True)
        print(json.dumps({"id": message["id"], "result": {"thread": {"id": thread_id}}}), flush=True)
    elif method == "turn/start":
        turn_id = "turn-from-response"
        print(json.dumps({"id": message["id"], "result": {"turn": {"id": turn_id}}}), flush=True)
        print(json.dumps({"method": "turn/started", "params": {"threadId": message["params"]["threadId"], "turn": {"id": turn_id}}}), flush=True)
"#,
    )
    .expect("write fake Codex");
    let mut permissions = std::fs::metadata(&fake_codex)
        .expect("fake Codex metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_codex, permissions).expect("fake Codex executable");

    let mut request =
        CodexAppServerLaunchRequest::new(directory.path(), test_config("spawn"), "initial turn");
    request.codex_binary = fake_codex;
    request.timeout = TEST_TIMEOUT;
    let launch = launch_codex_app_server(request).await.expect("launch");

    assert_eq!(
        launch.provider_resume().capability(),
        ProviderResumeCapability::Resumable
    );
    assert_eq!(
        launch.provider_resume().conversation_id(),
        Some("thread-from-response")
    );
    assert_eq!(
        launch.provider_resume().resume_argv(),
        Some(
            [
                "codex".to_string(),
                "resume".to_string(),
                "thread-from-response".to_string()
            ]
            .as_slice()
        )
    );
    assert_eq!(
        launch.provider_resume().resume_command(),
        Some("codex resume thread-from-response")
    );
    assert_eq!(launch.turn_id(), "turn-from-response");
    launch.into_session().terminate().await.expect("terminate");
}

#[test]
fn codex_app_server_provider_rejects_non_absolute_cwd_before_spawn() {
    let request = CodexAppServerLaunchRequest::new("relative/repo", Map::new(), "initial prompt");
    let error = validate_request(&request).expect_err("relative cwd");
    assert!(matches!(error, CodexAppServerError::RelativeCwd(_)));
}
