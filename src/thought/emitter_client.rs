use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::{info, warn};

use crate::thought::loop_runner::SessionInfo;
use crate::thought::protocol::{
    build_sync_request, DaemonInboundMessage, SyncRequestSequence, SyncResponse,
    SUPPORTED_EMIT_PROTOCOLS, SUPPORTED_EMIT_PROTOCOLS_DISPLAY,
};
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};

const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";
const EXTERNAL_CMD_WARN_THRESHOLD: Duration = Duration::from_secs(2);
const EMIT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const EMIT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on a single line read from the daemon's stdout. The daemon is an
/// external process (`clawgs emit --stdio`); a malformed or runaway emitter
/// could otherwise stream an unbounded line and force this process to allocate
/// without limit. Sync responses are small JSON objects, so 8 MiB is far above
/// any legitimate payload while still bounding worst-case memory.
const MAX_DAEMON_LINE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULTS_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

struct DaemonProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl DaemonProcess {
    async fn read_non_empty_line(&mut self) -> Result<Option<String>, io::Error> {
        let mut buf = Vec::new();
        loop {
            buf.clear();
            // Bound the read so a daemon that never emits a newline cannot force
            // an unbounded allocation. `take` caps this single line; the
            // underlying buffered reader keeps its position across iterations.
            let read = {
                let mut limited = (&mut self.stdout).take(MAX_DAEMON_LINE_BYTES + 1);
                limited.read_until(b'\n', &mut buf).await?
            };
            if read == 0 {
                return Ok(None);
            }
            if buf.len() as u64 > MAX_DAEMON_LINE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "clawgs emit daemon line exceeded the size cap",
                ));
            }

            let line = String::from_utf8_lossy(&buf);
            let trimmed = line.trim_end_matches(['\r', '\n']).trim();
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
    #[error("clawgs emit daemon did not send hello handshake within {timeout_ms}ms")]
    HelloTimeout { timeout_ms: u64 },
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
    #[error("clawgs emit daemon did not send sync response within {timeout_ms}ms")]
    ResponseTimeout { timeout_ms: u64 },
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
    #[error("daemon sync response id mismatch: expected `{expected}`, got `{actual}`")]
    ResponseRequestMismatch { expected: String, actual: String },
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
                | Self::HelloTimeout { .. }
                | Self::RequestWrite { .. }
                | Self::ResponseRead { .. }
                | Self::ResponseEof
                | Self::ResponseTimeout { .. }
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

    #[cfg(test)]
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

    pub async fn restart_daemon(&mut self) -> Result<(), EmitterClientError> {
        self.restart().await
    }

    async fn next_sync_response_once(
        &mut self,
        runtime_config: &ThoughtConfig,
        sessions: &[SessionInfo],
    ) -> Result<SyncResponse, EmitterClientError> {
        self.next_sync_response_once_with_timeout(runtime_config, sessions, EMIT_RESPONSE_TIMEOUT)
            .await
    }

    async fn next_sync_response_once_with_timeout(
        &mut self,
        runtime_config: &ThoughtConfig,
        sessions: &[SessionInfo],
        response_timeout: Duration,
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

        let response_line =
            match tokio::time::timeout(response_timeout, daemon.read_non_empty_line()).await {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => return Err(EmitterClientError::ResponseEof),
                Ok(Err(source)) => return Err(EmitterClientError::ResponseRead { source }),
                Err(_) => {
                    return Err(EmitterClientError::ResponseTimeout {
                        timeout_ms: response_timeout.as_millis() as u64,
                    });
                }
            };

        let response = parse_sync_response_line(&response_line)?;
        let expected_request_id = request_id.to_string();
        if response.request_id != expected_request_id {
            return Err(EmitterClientError::ResponseRequestMismatch {
                expected: expected_request_id,
                actual: response.request_id,
            });
        }
        Ok(response)
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
        let mut command = Command::new(&self.bin);
        command
            .arg("emit")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = command
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

        let hello_line = match tokio::time::timeout(
            EMIT_HANDSHAKE_TIMEOUT,
            daemon.read_non_empty_line(),
        )
        .await
        {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => return Err(EmitterClientError::HandshakeEof),
            Ok(Err(source)) => return Err(EmitterClientError::HelloRead { source }),
            Err(_) => {
                return Err(EmitterClientError::HelloTimeout {
                    timeout_ms: EMIT_HANDSHAKE_TIMEOUT.as_millis() as u64,
                });
            }
        };

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
    fetch_daemon_defaults_for_bin(&bin).await
}

async fn fetch_daemon_defaults_for_bin(bin: &str) -> Option<DaemonDefaults> {
    fetch_daemon_defaults_for_bin_with_timeout(bin, DEFAULTS_COMMAND_TIMEOUT).await
}

async fn fetch_daemon_defaults_for_bin_with_timeout(
    bin: &str,
    timeout_duration: Duration,
) -> Option<DaemonDefaults> {
    let started = Instant::now();
    info!(phase = "clawgs_defaults_command", bin = %bin, "running clawgs defaults");
    let mut command = Command::new(bin);
    command
        .arg("defaults")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = match tokio::time::timeout(timeout_duration, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            warn!(bin = %bin, error = %err, "failed to run clawgs defaults");
            return None;
        }
        Err(_) => {
            warn!(
                bin = %bin,
                timeout_ms = timeout_duration.as_millis() as u64,
                "clawgs defaults timed out"
            );
            return None;
        }
    };

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

pub fn resolve_clawgs_bin() -> String {
    resolve_clawgs_bin_with(
        env::var("CLAWGS_BIN").ok(),
        env::current_exe().ok(),
        env::current_dir().ok(),
    )
}

fn resolve_clawgs_bin_with(
    explicit_bin: Option<String>,
    current_exe: Option<PathBuf>,
    current_dir: Option<PathBuf>,
) -> String {
    normalize_bin_override(explicit_bin)
        .or_else(|| packaged_clawgs_bin(current_exe.as_deref()))
        .or_else(|| adjacent_checkout_clawgs_bin(current_dir.as_deref(), current_exe.as_deref()))
        .unwrap_or_else(|| default_clawgs_bin_name().to_string())
}

fn normalize_bin_override(explicit_bin: Option<String>) -> Option<String> {
    explicit_bin
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn packaged_clawgs_bin(executable: Option<&Path>) -> Option<String> {
    let executable = executable?;
    let candidate = executable.with_file_name(default_clawgs_bin_name());
    candidate
        .is_file()
        .then(|| candidate.to_string_lossy().into_owned())
}

fn adjacent_checkout_clawgs_bin(
    current_dir: Option<&Path>,
    executable: Option<&Path>,
) -> Option<String> {
    current_dir
        .into_iter()
        .chain(executable.and_then(Path::parent))
        .flat_map(adjacent_checkout_candidates)
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

fn adjacent_checkout_candidates(root: &Path) -> [PathBuf; 1] {
    let base = root.parent().unwrap_or(root);
    [base
        .join("clawgs/target/release")
        .join(default_clawgs_bin_name())]
}

fn default_clawgs_bin_name() -> &'static str {
    if cfg!(windows) {
        "clawgs.exe"
    } else {
        "clawgs"
    }
}

fn parse_hello_line(line: &str) -> Result<(), EmitterClientError> {
    let message: DaemonInboundMessage =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedHello {
            line: line.to_string(),
            source,
        })?;

    match message {
        DaemonInboundMessage::Hello { protocol } => {
            if !SUPPORTED_EMIT_PROTOCOLS.contains(&protocol.as_str()) {
                return Err(EmitterClientError::HelloProtocolMismatch {
                    expected: SUPPORTED_EMIT_PROTOCOLS_DISPLAY,
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
            metrics,
        } => Ok(SyncResponse {
            request_id,
            stream_instance_id,
            updates,
            llm_calls: metrics.llm_calls,
            last_backend_error: metrics.last_backend_error,
        }),
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

    async fn wait_for_non_empty_file(path: &Path, timeout: Duration) -> String {
        tokio::time::timeout(timeout, async {
            loop {
                if let Ok(contents) = fs::read_to_string(path) {
                    if !contents.trim().is_empty() {
                        return contents;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("file should become non-empty")
    }

    #[test]
    fn parse_hello_accepts_expected_protocol() {
        let raw = r#"{"type":"hello","protocol":"clawgs.emit.v1","engine_version":"0.1.0"}"#;
        let result = parse_hello_line(raw);
        assert!(result.is_ok(), "expected valid hello, got: {result:?}");
    }

    #[test]
    fn parse_hello_accepts_current_protocol() {
        let raw = r#"{"type":"hello","protocol":"clawgs.emit.v2","engine_version":"0.2.0"}"#;
        let result = parse_hello_line(raw);
        assert!(result.is_ok(), "expected valid hello, got: {result:?}");
    }

    #[test]
    fn parse_hello_rejects_unexpected_protocol() {
        let raw = r#"{"type":"hello","protocol":"clawgs.emit.v3"}"#;
        let err = parse_hello_line(raw).expect_err("hello with wrong protocol should fail");
        match err {
            EmitterClientError::HelloProtocolMismatch { expected, actual } => {
                assert_eq!(expected, SUPPORTED_EMIT_PROTOCOLS_DISPLAY);
                assert_eq!(actual, "clawgs.emit.v3");
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
        assert_eq!(response.llm_calls, 0);
        assert_eq!(response.last_backend_error, None);
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
        assert_eq!(response.llm_calls, 0);
        assert_eq!(response.last_backend_error, None);
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
            commit_candidate: false,
            action_cues: Vec::new(),
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

        let request_line = wait_for_non_empty_file(&input_log, Duration::from_millis(500)).await;
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

    #[tokio::test]
    async fn sync_response_times_out_when_daemon_hangs_after_request() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let args_log = temp.path().join("args.log");
        let input_log = temp.path().join("input.log");
        let script_path = temp.path().join("hanging-clawgs.sh");
        let script = r#"#!/bin/sh
printf '%s\n' "$*" >> "__ARGS_LOG__"
printf '%s\n' '{"type":"hello","protocol":"clawgs.emit.v1","engine_version":"0.1.0"}'
IFS= read -r line || exit 0
printf '%s\n' "$line" >> "__INPUT_LOG__"
exec /bin/sleep 2
"#
        .replace("__ARGS_LOG__", &args_log.display().to_string())
        .replace("__INPUT_LOG__", &input_log.display().to_string());
        fs::write(&script_path, script).expect("write hanging clawgs");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod hanging clawgs");

        let mut client = EmitterClient::with_bin(script_path.to_string_lossy().into_owned());
        let err = client
            .next_sync_response_once_with_timeout(
                &ThoughtConfig::default(),
                &[sample_session_info()],
                Duration::from_millis(50),
            )
            .await
            .expect_err("daemon response should time out");

        match err {
            EmitterClientError::ResponseTimeout { timeout_ms } => assert_eq!(timeout_ms, 50),
            other => panic!("unexpected error variant: {other:?}"),
        }

        let request_line = wait_for_non_empty_file(&input_log, Duration::from_millis(500)).await;
        let request: Value =
            serde_json::from_str(request_line.lines().next().expect("first sync request"))
                .expect("sync request json");
        assert_eq!(request["type"], "sync");
        assert_eq!(request["id"], "1");
    }

    #[tokio::test]
    async fn sync_response_rejects_mismatched_request_id() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let script_path = temp.path().join("wrong-id-clawgs.sh");
        let script = r#"#!/bin/sh
printf '%s\n' '{"type":"hello","protocol":"clawgs.emit.v1","engine_version":"0.1.0"}'
IFS= read -r line || exit 0
printf '%s\n' '{"type":"sync_result","id":"stale-request","updates":[]}'
"#;
        fs::write(&script_path, script).expect("write wrong-id clawgs");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod wrong-id clawgs");

        let mut client = EmitterClient::with_bin(script_path.to_string_lossy().into_owned());
        let err = client
            .next_sync_response_once_with_timeout(
                &ThoughtConfig::default(),
                &[sample_session_info()],
                Duration::from_millis(500),
            )
            .await
            .expect_err("mismatched response id should fail");

        match err {
            EmitterClientError::ResponseRequestMismatch { expected, actual } => {
                assert_eq!(expected, "1");
                assert_eq!(actual, "stale-request");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_daemon_defaults_reads_json_from_packaged_binary() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let original = env::var("CLAWGS_BIN").ok();
        let temp = tempdir().expect("tempdir");
        let args_log = temp.path().join("args.log");
        let input_log = temp.path().join("input.log");
        let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());

        env::set_var("CLAWGS_BIN", fake_bin.as_os_str());
        let defaults = fetch_daemon_defaults().await;
        restore_env_var("CLAWGS_BIN", original);

        let defaults = defaults.expect("defaults should parse");
        assert_eq!(defaults.model, "test-model");
        assert_eq!(
            defaults.agent_prompt,
            "You are a status reporter for a coding agent session."
        );
        assert_eq!(
            defaults.terminal_prompt,
            "Terminal session status reporter."
        );
    }

    #[tokio::test]
    async fn fetch_daemon_defaults_times_out() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let script_path = temp.path().join("hanging-clawgs.sh");
        fs::write(
            &script_path,
            "#!/bin/sh\nif [ \"$1\" = \"defaults\" ]; then exec sleep 2; fi\n",
        )
        .expect("write hanging clawgs");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod hanging clawgs");

        let defaults = fetch_daemon_defaults_for_bin_with_timeout(
            &script_path.to_string_lossy(),
            Duration::from_millis(50),
        )
        .await;

        assert!(defaults.is_none());
    }

    #[test]
    fn resolve_clawgs_bin_prefers_explicit_env_override() {
        let resolved = resolve_clawgs_bin_with(
            Some(" /tmp/custom-clawgs ".to_string()),
            Some(PathBuf::from("/tmp/swimmers")),
            Some(PathBuf::from("/tmp/project")),
        );

        assert_eq!(resolved, "/tmp/custom-clawgs");
    }

    #[test]
    fn resolve_clawgs_bin_uses_packaged_sibling_before_path_lookup() {
        let temp = tempdir().expect("tempdir");
        let executable = temp.path().join("swimmers");
        let packaged = temp.path().join(default_clawgs_bin_name());
        fs::write(&packaged, "#!/bin/sh\n").expect("write packaged clawgs");

        let resolved =
            resolve_clawgs_bin_with(None, Some(executable), Some(temp.path().to_path_buf()));

        assert_eq!(resolved, packaged.to_string_lossy());
    }

    #[test]
    fn resolve_clawgs_bin_prefers_adjacent_opensource_checkout_before_path_lookup() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("opensource/swimmers");
        let adjacent = write_adjacent_checkout_bin(&temp.path().join("opensource"));

        let resolved = resolve_clawgs_bin_with(
            None,
            Some(PathBuf::from("/tmp/swimmers-bin")),
            Some(repo_root),
        );

        assert_eq!(resolved, adjacent.to_string_lossy());
    }

    #[test]
    fn resolve_clawgs_bin_prefers_current_dir_adjacent_before_executable_parent() {
        let temp = tempdir().expect("tempdir");
        let current_dir = temp.path().join("work/swimmers");
        let executable = temp.path().join("bin/swimmers");
        let current_dir_adjacent = write_adjacent_checkout_bin(&temp.path().join("work"));
        let executable_adjacent = write_adjacent_checkout_bin(temp.path());

        let resolved = resolve_clawgs_bin_with(None, Some(executable), Some(current_dir));

        assert_eq!(resolved, current_dir_adjacent.to_string_lossy());
        assert_ne!(resolved, executable_adjacent.to_string_lossy());
    }

    #[test]
    fn resolve_clawgs_bin_uses_executable_parent_adjacent_checkout() {
        let temp = tempdir().expect("tempdir");
        let current_dir = temp.path().join("work/swimmers");
        let executable = temp.path().join("bin/swimmers");
        let executable_adjacent = write_adjacent_checkout_bin(temp.path());

        let resolved = resolve_clawgs_bin_with(None, Some(executable), Some(current_dir));

        assert_eq!(resolved, executable_adjacent.to_string_lossy());
    }

    #[test]
    fn resolve_clawgs_bin_falls_back_to_default_name() {
        let resolved = resolve_clawgs_bin_with(
            None,
            Some(PathBuf::from("/tmp/swimmers")),
            Some(PathBuf::from("/tmp/project")),
        );
        assert_eq!(resolved, default_clawgs_bin_name());
    }

    fn write_adjacent_checkout_bin(base: &Path) -> PathBuf {
        let adjacent = base
            .join("clawgs/target/release")
            .join(default_clawgs_bin_name());
        fs::create_dir_all(adjacent.parent().expect("parent dir"))
            .expect("create adjacent clawgs dir");
        fs::write(&adjacent, "#!/bin/sh\n").expect("write adjacent clawgs");
        adjacent
    }

    fn restore_env_var(key: &str, value: Option<String>) {
        match value {
            Some(value) => env::set_var(key, value),
            None => env::remove_var(key),
        }
    }

    fn write_fake_clawgs_script(
        args_log: &Path,
        input_log: &Path,
        dir: &Path,
    ) -> std::path::PathBuf {
        let script_path = dir.join("fake-clawgs.sh");
        let script = r#"#!/bin/sh
printf '%s\n' "$*" >> "__ARGS_LOG__"
if [ "$1" = "defaults" ]; then
  printf '%s\n' '{"model":"test-model","agent_prompt":"You are a status reporter for a coding agent session.","terminal_prompt":"Terminal session status reporter."}'
  exit 0
fi
printf '%s\n' '{"type":"hello","protocol":"clawgs.emit.v1","engine_version":"0.1.0"}'
count=1
while IFS= read -r line; do
  printf '%s\n' "$line" >> "__INPUT_LOG__"
  printf '%s\n' '{"type":"sync_result","id":"'"$count"'","stream_instance_id":"stream-a","updates":[],"metrics":{"sessions_seen":1,"llm_calls":1,"suppressed":0}}'
  count=$((count + 1))
done
sleep 5
"#
        .replace("__ARGS_LOG__", &args_log.display().to_string())
        .replace("__INPUT_LOG__", &input_log.display().to_string());
        fs::write(&script_path, script).expect("write fake clawgs");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod fake clawgs");
        script_path
    }
}
