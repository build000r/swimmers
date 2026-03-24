use std::env;
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::{info, warn};

use crate::thought::loop_runner::SessionInfo;
use crate::thought::protocol::{
    build_sync_request, DaemonInboundMessage, SyncRequestSequence, SyncResponse, EMIT_PROTOCOL_V1,
};
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};

const DEFAULT_CLAWGS_BIN: &str = "clawgs";
const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";
const EXTERNAL_CMD_WARN_THRESHOLD: Duration = Duration::from_secs(2);

struct DaemonProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl DaemonProcess {
    async fn read_non_empty_line(&mut self) -> Result<Option<String>, io::Error> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self.stdout.read_line(&mut line).await?;
            if read == 0 {
                return Ok(None);
            }

            let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n').trim();
            if !trimmed.is_empty() {
                return Ok(Some(trimmed.to_string()));
            }
        }
    }

    fn is_exited(&mut self) -> Result<bool, io::Error> {
        Ok(self.child.try_wait()?.is_some())
    }

    async fn write_line(&mut self, line: &str) -> Result<(), io::Error> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await
    }
}

#[derive(Debug, Error)]
pub enum EmitterClientError {
    #[error("failed to spawn clawgs emit daemon `{bin}`: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: io::Error,
    },
    #[error("clawgs emit daemon missing stdin pipe")]
    MissingStdin,
    #[error("clawgs emit daemon missing stdout pipe")]
    MissingStdout,
    #[error("failed to read hello from clawgs emit daemon: {source}")]
    HelloRead {
        #[source]
        source: io::Error,
    },
    #[error("clawgs emit daemon exited before sending hello handshake")]
    HandshakeEof,
    #[error("malformed hello message from clawgs emit daemon: {line}")]
    MalformedHello {
        line: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unexpected daemon message `{found}` during hello handshake: {line}")]
    UnexpectedHelloType { found: String, line: String },
    #[error("unsupported clawgs emit protocol `{actual}` (expected `{expected}`)")]
    HelloProtocolMismatch {
        expected: &'static str,
        actual: String,
    },
    #[error("failed to serialize sync request: {source}")]
    RequestSerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write sync request to clawgs emit daemon: {source}")]
    RequestWrite {
        #[source]
        source: io::Error,
    },
    #[error("failed to read sync response from clawgs emit daemon: {source}")]
    ResponseRead {
        #[source]
        source: io::Error,
    },
    #[error("clawgs emit daemon closed stdout before emitting a sync response")]
    ResponseEof,
    #[error("malformed sync response from clawgs emit daemon: {line}")]
    MalformedResponse {
        line: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("daemon returned error `{code}`: {message}")]
    DaemonError {
        code: String,
        message: String,
        request_id: Option<String>,
    },
    #[error("unexpected daemon message `{found}` while waiting for `{expected}`: {line}")]
    UnexpectedResponseType {
        expected: &'static str,
        found: String,
        line: String,
    },
    #[error("failed to inspect clawgs emit daemon status: {source}")]
    StatusCheck {
        #[source]
        source: io::Error,
    },
}

impl EmitterClientError {
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::HelloRead { .. }
                | Self::HandshakeEof
                | Self::RequestWrite { .. }
                | Self::ResponseRead { .. }
                | Self::ResponseEof
                | Self::StatusCheck { .. }
        )
    }
}

/// Line-delimited JSON client for `clawgs emit --stdio`.
pub struct EmitterClient {
    bin: String,
    daemon: Option<DaemonProcess>,
    request_sequence: Arc<SyncRequestSequence>,
}

impl Default for EmitterClient {
    fn default() -> Self {
        Self::new()
    }
}

impl EmitterClient {
    pub fn new() -> Self {
        Self::with_request_sequence(Arc::new(SyncRequestSequence::new()))
    }

    pub fn with_request_sequence(request_sequence: Arc<SyncRequestSequence>) -> Self {
        Self::with_bin_and_request_sequence(resolve_clawgs_bin(), request_sequence)
    }

    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self::with_bin_and_request_sequence(bin, Arc::new(SyncRequestSequence::new()))
    }

    pub fn with_bin_and_request_sequence(
        bin: impl Into<String>,
        request_sequence: Arc<SyncRequestSequence>,
    ) -> Self {
        Self {
            bin: bin.into(),
            daemon: None,
            request_sequence,
        }
    }

    pub async fn next_sync_response(
        &mut self,
        runtime_config: &ThoughtConfig,
        sessions: &[SessionInfo],
    ) -> Result<SyncResponse, EmitterClientError> {
        match self.next_sync_response_once(runtime_config, sessions).await {
            Ok(response) => Ok(response),
            Err(first_err) if first_err.is_retryable() => {
                warn!(
                    error = %first_err,
                    "clawgs emit read failed; restarting daemon and retrying once"
                );
                self.restart().await?;
                self.next_sync_response_once(runtime_config, sessions).await
            }
            Err(err) => Err(err),
        }
    }

    async fn restart(&mut self) -> Result<(), EmitterClientError> {
        self.stop_current_daemon().await;
        self.daemon = Some(self.spawn_daemon().await?);
        Ok(())
    }

    async fn next_sync_response_once(
        &mut self,
        runtime_config: &ThoughtConfig,
        sessions: &[SessionInfo],
    ) -> Result<SyncResponse, EmitterClientError> {
        self.ensure_running().await?;

        let request_id = self.request_sequence.next();
        let request = build_sync_request(request_id, runtime_config, sessions);
        let request_line = serde_json::to_string(&request)
            .map_err(|source| EmitterClientError::RequestSerialization { source })?;
        let daemon = self
            .daemon
            .as_mut()
            .expect("daemon must exist after ensure_running");
        daemon
            .write_line(&request_line)
            .await
            .map_err(|source| EmitterClientError::RequestWrite { source })?;

        let response_line = daemon
            .read_non_empty_line()
            .await
            .map_err(|source| EmitterClientError::ResponseRead { source })?
            .ok_or(EmitterClientError::ResponseEof)?;

        parse_sync_response_line(&response_line)
    }

    async fn ensure_running(&mut self) -> Result<(), EmitterClientError> {
        let should_spawn = match self.daemon.as_mut() {
            Some(daemon) => daemon
                .is_exited()
                .map_err(|source| EmitterClientError::StatusCheck { source })?,
            None => true,
        };

        if should_spawn {
            self.stop_current_daemon().await;
            self.daemon = Some(self.spawn_daemon().await?);
        }

        Ok(())
    }

    async fn spawn_daemon(&self) -> Result<DaemonProcess, EmitterClientError> {
        let mut child = Command::new(&self.bin)
            .arg("emit")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| EmitterClientError::Spawn {
                bin: self.bin.clone(),
                source,
            })?;

        let stdin = child.stdin.take().ok_or(EmitterClientError::MissingStdin)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(EmitterClientError::MissingStdout)?;

        let mut daemon = DaemonProcess {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };

        let hello_line = daemon
            .read_non_empty_line()
            .await
            .map_err(|source| EmitterClientError::HelloRead { source })?
            .ok_or(EmitterClientError::HandshakeEof)?;

        parse_hello_line(&hello_line)?;

        Ok(daemon)
    }

    async fn stop_current_daemon(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.child.start_kill();
            let _ = daemon.child.wait().await;
        }
    }
}

impl Drop for EmitterClient {
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon.as_mut() {
            let _ = daemon.child.start_kill();
        }
    }
}

/// Run `clawgs defaults` as a one-shot process and parse the JSON output.
/// Returns `None` on any failure (non-fatal — just means placeholders won't
/// show actual daemon values).
pub async fn fetch_daemon_defaults() -> Option<DaemonDefaults> {
    let bin = resolve_clawgs_bin();
    let started = Instant::now();
    info!(phase = "clawgs_defaults_command", bin = %bin, "running clawgs defaults");
    let output = Command::new(&bin)
        .arg("defaults")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| {
            warn!(bin = %bin, error = %err, "failed to run clawgs defaults");
            err
        })
        .ok()?;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if started.elapsed() >= EXTERNAL_CMD_WARN_THRESHOLD {
        warn!(
            phase = "clawgs_defaults_command",
            bin = %bin,
            elapsed_ms,
            status = %output.status,
            "clawgs defaults completed slowly"
        );
    } else {
        info!(
            phase = "clawgs_defaults_command",
            bin = %bin,
            elapsed_ms,
            status = %output.status,
            "clawgs defaults completed"
        );
    }

    if !output.status.success() {
        warn!(
            bin = %bin,
            status = %output.status,
            "clawgs defaults exited with non-zero status"
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .map_err(|err| {
            warn!(error = %err, "failed to parse clawgs defaults output");
            err
        })
        .ok()
}

fn resolve_clawgs_bin() -> String {
    env::var("CLAWGS_BIN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_CLAWGS_BIN.to_string())
}

fn parse_hello_line(line: &str) -> Result<(), EmitterClientError> {
    let message: DaemonInboundMessage =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedHello {
            line: line.to_string(),
            source,
        })?;

    match message {
        DaemonInboundMessage::Hello { protocol } => {
            if protocol != EMIT_PROTOCOL_V1 {
                return Err(EmitterClientError::HelloProtocolMismatch {
                    expected: EMIT_PROTOCOL_V1,
                    actual: protocol,
                });
            }
            Ok(())
        }
        other => Err(EmitterClientError::UnexpectedHelloType {
            found: other.message_type().to_string(),
            line: line.to_string(),
        }),
    }
}

fn parse_sync_response_line(line: &str) -> Result<SyncResponse, EmitterClientError> {
    let message: DaemonInboundMessage =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedResponse {
            line: line.to_string(),
            source,
        })?;

    match message {
        DaemonInboundMessage::SyncResponse {
            request_id,
            stream_instance_id,
            updates,
        } => Ok(SyncResponse::from(clawgs::emit::protocol::SyncResponse {
            request_id,
            stream_instance_id,
            updates,
        })),
        DaemonInboundMessage::Error {
            code,
            message,
            request_id,
        } => Err(EmitterClientError::DaemonError {
            code,
            message,
            request_id,
        }),
        other => Err(EmitterClientError::UnexpectedResponseType {
            expected: SYNC_RESULT_MESSAGE_TYPE,
            found: other.message_type().to_string(),
            line: line.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thought::loop_runner::SessionInfo;
    use crate::types::{BubblePrecedence, RestState, SessionState, ThoughtSource, ThoughtState};
    use chrono::Utc;
    use serde_json::Value;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::{LazyLock, Mutex as StdMutex};
    use tempfile::tempdir;

    static TEST_ENV_LOCK: LazyLock<StdMutex<()>> = LazyLock::new(|| StdMutex::new(()));

    #[test]
    fn parse_hello_accepts_expected_protocol() {
        let raw = r#"{"type":"hello","protocol":"clawgs.emit.v1","engine_version":"0.1.0"}"#;
        let result = parse_hello_line(raw);
        assert!(result.is_ok(), "expected valid hello, got: {result:?}");
    }

    #[test]
    fn parse_hello_rejects_unexpected_protocol() {
        let raw = r#"{"type":"hello","protocol":"clawgs.emit.v2"}"#;
        let err = parse_hello_line(raw).expect_err("hello with wrong protocol should fail");
        match err {
            EmitterClientError::HelloProtocolMismatch { expected, actual } => {
                assert_eq!(expected, EMIT_PROTOCOL_V1);
                assert_eq!(actual, "clawgs.emit.v2");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn parse_sync_response_extracts_updates_and_stream_identity() {
        let now = Utc::now();
        let raw = serde_json::json!({
            "type": "sync_result",
            "id": "tmux-9",
            "stream_instance_id": "stream-a",
            "updates": [
                {
                    "session_id": "tmux:work:1.0:%1",
                    "stream_instance_id": "stream-a",
                    "emission_seq": 1,
                    "thought": "Applying patch",
                    "token_count": 55,
                    "context_limit": 100,
                    "thought_state": "active",
                    "thought_source": "llm",
                    "rest_state": "drowsy",
                    "objective_changed": true,
                    "bubble_precedence": "thought_first",
                    "at": now,
                    "objective_fingerprint": "obj-9"
                }
            ]
        })
        .to_string();

        let response =
            parse_sync_response_line(&raw).expect("sync response should parse successfully");

        assert_eq!(response.request_id, "tmux-9");
        assert_eq!(response.stream_instance_id.as_deref(), Some("stream-a"));
        assert_eq!(response.updates.len(), 1);
        let update = &response.updates[0];
        assert_eq!(update.session_id, "tmux:work:1.0:%1");
        assert_eq!(update.stream_instance_id.as_deref(), Some("stream-a"));
        assert_eq!(update.emission_seq, Some(1));
        assert_eq!(update.thought.as_deref(), Some("Applying patch"));
        assert_eq!(update.token_count, 55);
        assert_eq!(update.context_limit, 100);
        assert_eq!(update.thought_state, ThoughtState::Active);
        assert_eq!(update.thought_source, ThoughtSource::Llm);
        assert_eq!(update.rest_state, RestState::Drowsy);
        assert!(update.objective_changed);
        assert_eq!(update.bubble_precedence, BubblePrecedence::ThoughtFirst);
        assert_eq!(update.objective_fingerprint.as_deref(), Some("obj-9"));
    }

    #[test]
    fn parse_sync_response_surfaces_daemon_error() {
        let raw =
            r#"{"type":"error","id":"tmux-4","code":"bad_request","message":"invalid payload"}"#;
        let err = parse_sync_response_line(raw).expect_err("error envelope should fail sync");

        match err {
            EmitterClientError::DaemonError {
                code,
                message,
                request_id,
            } => {
                assert_eq!(code, "bad_request");
                assert_eq!(message, "invalid payload");
                assert_eq!(request_id, Some("tmux-4".to_string()));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn parse_sync_response_accepts_legacy_request_id_field() {
        let raw = r#"{"type":"sync_response","request_id":"123","updates":[]}"#;
        let response = parse_sync_response_line(raw)
            .expect("legacy string request_id should parse successfully");
        assert_eq!(response.request_id, "123");
        assert!(response.updates.is_empty());
    }

    fn sample_session_info() -> SessionInfo {
        SessionInfo {
            session_id: "sess-1".to_string(),
            state: SessionState::Idle,
            exited: false,
            tool: Some("Codex".to_string()),
            cwd: "/tmp/project".to_string(),
            replay_text: "working".to_string(),
            thought: Some("reviewing diff".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Drowsy,
            objective_fingerprint: Some("obj-1".to_string()),
            thought_updated_at: Some(Utc::now()),
            token_count: 55,
            context_limit: 100,
            last_activity_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn daemon_spawn_uses_stdio_and_sends_sync_request() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let args_log = temp.path().join("args.log");
        let input_log = temp.path().join("input.log");
        let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());
        let mut client = EmitterClient::with_bin(fake_bin.to_string_lossy().into_owned());

        let response = client
            .next_sync_response(&ThoughtConfig::default(), &[sample_session_info()])
            .await
            .expect("sync response");

        assert_eq!(response.request_id, "1");
        assert_eq!(response.stream_instance_id.as_deref(), Some("stream-a"));
        assert!(response.updates.is_empty());

        let logged = fs::read_to_string(&args_log).expect("read args log");
        let line = logged.lines().next().expect("spawned command line");
        assert_eq!(line, "emit --stdio");

        let request_line = fs::read_to_string(&input_log).expect("read input log");
        let request: Value =
            serde_json::from_str(request_line.lines().next().expect("first sync request"))
                .expect("sync request json");
        assert_eq!(request["type"], "sync");
        assert_eq!(request["id"], "1");
        assert_eq!(request["config"]["enabled"], true);
        assert_eq!(request["sessions"][0]["session_id"], "sess-1");
        assert_eq!(request["sessions"][0]["rest_state"], "drowsy");
    }

    #[tokio::test]
    async fn successive_sync_requests_reuse_daemon_and_send_updated_config() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let args_log = temp.path().join("args.log");
        let input_log = temp.path().join("input.log");
        let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());
        let mut client = EmitterClient::with_bin(fake_bin.to_string_lossy().into_owned());

        let baseline = ThoughtConfig::default();
        client
            .next_sync_response(&baseline, &[sample_session_info()])
            .await
            .expect("initial sync response");

        let mut updated = baseline.clone();
        updated.agent_prompt = Some("Hook wakeup prompt".to_string());
        client
            .next_sync_response(&updated, &[sample_session_info()])
            .await
            .expect("second sync response");

        let logged = fs::read_to_string(&args_log).expect("read args log");
        let lines: Vec<&str> = logged.lines().collect();
        assert_eq!(lines, vec!["emit --stdio"], "daemon should be reused");

        let sent: Vec<Value> = fs::read_to_string(&input_log)
            .expect("read input log")
            .lines()
            .map(|line| serde_json::from_str(line).expect("request json"))
            .collect();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["id"], "1");
        assert_eq!(sent[1]["id"], "2");
        assert_eq!(sent[1]["config"]["agent_prompt"], "Hook wakeup prompt");
    }

    fn write_fake_clawgs_script(
        args_log: &Path,
        input_log: &Path,
        dir: &Path,
    ) -> std::path::PathBuf {
        let script_path = dir.join("fake-clawgs.sh");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nprintf '%s\\n' '{{\"type\":\"hello\",\"protocol\":\"clawgs.emit.v1\",\"engine_version\":\"0.1.0\"}}'\ncount=1\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"{}\"\n  printf '%s\\n' \"{{\\\"type\\\":\\\"sync_result\\\",\\\"id\\\":\\\"$count\\\",\\\"stream_instance_id\\\":\\\"stream-a\\\",\\\"updates\\\":[]}}\"\n  count=$((count + 1))\ndone\nsleep 5\n",
            args_log.display(),
            input_log.display()
        );
        fs::write(&script_path, script).expect("write fake clawgs");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod fake clawgs");
        script_path
    }
}
