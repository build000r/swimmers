use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tracing::warn;

use crate::config::resolve_tmux_emit_socket;
use crate::thought::protocol::{SyncResponse, SyncUpdate, EMIT_PROTOCOL_V1};
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};

const DEFAULT_CLAWGS_BIN: &str = "clawgs";
const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";

struct DaemonProcess {
    child: Child,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TmuxEmitSpawnConfig {
    interval_ms: u64,
    socket_path: PathBuf,
    config_json: String,
}

impl TmuxEmitSpawnConfig {
    fn from_runtime_config(
        runtime_config: &ThoughtConfig,
        interval: Duration,
        socket_path: impl AsRef<Path>,
    ) -> Result<Self, EmitterClientError> {
        let interval_ms = interval.as_millis().clamp(1, u64::MAX as u128) as u64;
        let config_json = serde_json::to_string(runtime_config)
            .map_err(|source| EmitterClientError::ConfigSerialization { source })?;

        Ok(Self {
            interval_ms,
            socket_path: socket_path.as_ref().to_path_buf(),
            config_json,
        })
    }
}

#[derive(Debug, Error)]
pub enum EmitterClientError {
    #[error("failed to spawn clawgs tmux-emit daemon `{bin}`: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: io::Error,
    },
    #[error("clawgs tmux-emit daemon missing stdout pipe")]
    MissingStdout,
    #[error("failed to read hello from clawgs tmux-emit daemon: {source}")]
    HelloRead {
        #[source]
        source: io::Error,
    },
    #[error("clawgs tmux-emit daemon exited before sending hello handshake")]
    HandshakeEof,
    #[error("malformed hello message from clawgs tmux-emit daemon: {line}")]
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
    #[error("failed to read sync response from clawgs tmux-emit daemon: {source}")]
    ResponseRead {
        #[source]
        source: io::Error,
    },
    #[error("clawgs tmux-emit daemon closed stdout before emitting a sync response")]
    ResponseEof,
    #[error("malformed sync response from clawgs tmux-emit daemon: {line}")]
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
    #[error("failed to inspect clawgs tmux-emit daemon status: {source}")]
    StatusCheck {
        #[source]
        source: io::Error,
    },
    #[error("failed to serialize tmux emit config: {source}")]
    ConfigSerialization {
        #[source]
        source: serde_json::Error,
    },
}

impl EmitterClientError {
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::HelloRead { .. }
                | Self::HandshakeEof
                | Self::ResponseRead { .. }
                | Self::ResponseEof
                | Self::StatusCheck { .. }
        )
    }
}

/// Line-delimited JSON client for `clawgs tmux-emit`.
pub struct EmitterClient {
    bin: String,
    socket_path: PathBuf,
    daemon: Option<DaemonProcess>,
    spawn_config: Option<TmuxEmitSpawnConfig>,
}

impl Default for EmitterClient {
    fn default() -> Self {
        Self::new()
    }
}

impl EmitterClient {
    pub fn new() -> Self {
        Self::with_bin_and_socket(resolve_clawgs_bin(), resolve_tmux_emit_socket())
    }

    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self::with_bin_and_socket(bin, resolve_tmux_emit_socket())
    }

    pub fn with_bin_and_socket(bin: impl Into<String>, socket_path: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            socket_path: socket_path.into(),
            daemon: None,
            spawn_config: None,
        }
    }

    pub async fn next_sync_response(
        &mut self,
        runtime_config: &ThoughtConfig,
        interval: Duration,
    ) -> Result<SyncResponse, EmitterClientError> {
        let desired =
            TmuxEmitSpawnConfig::from_runtime_config(runtime_config, interval, &self.socket_path)?;

        match self.next_sync_response_once(&desired).await {
            Ok(response) => Ok(response),
            Err(first_err) if first_err.is_retryable() => {
                warn!(
                    error = %first_err,
                    "clawgs tmux-emit read failed; restarting daemon and retrying once"
                );
                self.restart(desired.clone()).await?;
                self.next_sync_response_once(&desired).await
            }
            Err(err) => Err(err),
        }
    }

    async fn restart(&mut self, desired: TmuxEmitSpawnConfig) -> Result<(), EmitterClientError> {
        self.stop_current_daemon().await;
        self.daemon = Some(self.spawn_daemon(&desired).await?);
        self.spawn_config = Some(desired);
        Ok(())
    }

    async fn next_sync_response_once(
        &mut self,
        desired: &TmuxEmitSpawnConfig,
    ) -> Result<SyncResponse, EmitterClientError> {
        self.ensure_running(desired).await?;

        let daemon = self
            .daemon
            .as_mut()
            .expect("daemon must exist after ensure_running");

        let response_line = daemon
            .read_non_empty_line()
            .await
            .map_err(|source| EmitterClientError::ResponseRead { source })?
            .ok_or(EmitterClientError::ResponseEof)?;

        parse_sync_response_line(&response_line)
    }

    async fn ensure_running(
        &mut self,
        desired: &TmuxEmitSpawnConfig,
    ) -> Result<(), EmitterClientError> {
        let should_spawn = match self.daemon.as_mut() {
            Some(daemon) => {
                daemon
                    .is_exited()
                    .map_err(|source| EmitterClientError::StatusCheck { source })?
                    || self.spawn_config.as_ref() != Some(desired)
            }
            None => true,
        };

        if should_spawn {
            self.stop_current_daemon().await;
            self.daemon = Some(self.spawn_daemon(desired).await?);
            self.spawn_config = Some(desired.clone());
        }

        Ok(())
    }

    async fn spawn_daemon(
        &self,
        desired: &TmuxEmitSpawnConfig,
    ) -> Result<DaemonProcess, EmitterClientError> {
        let mut child = Command::new(&self.bin)
            .arg("tmux-emit")
            .arg("--interval-ms")
            .arg(desired.interval_ms.to_string())
            .arg("--socket")
            .arg(&desired.socket_path)
            .arg("--config-json")
            .arg(&desired.config_json)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| EmitterClientError::Spawn {
                bin: self.bin.clone(),
                source,
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or(EmitterClientError::MissingStdout)?;

        let mut daemon = DaemonProcess {
            child,
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

fn parse_error_field(
    object: &serde_json::Map<String, Value>,
    field: &str,
    fallback: &str,
) -> String {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| object.get(field).map(Value::to_string))
        .unwrap_or_else(|| fallback.to_string())
}

fn extract_correlation_id(object: &serde_json::Map<String, Value>) -> Option<String> {
    object
        .get("id")
        .and_then(value_as_correlation_id)
        .or_else(|| object.get("request_id").and_then(value_as_correlation_id))
}

fn value_as_correlation_id(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn message_type(object: &serde_json::Map<String, Value>) -> String {
    object
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "<missing>".to_string())
}

fn parse_updates(
    object: &serde_json::Map<String, Value>,
    line: &str,
) -> Result<Vec<SyncUpdate>, EmitterClientError> {
    let updates_value = match object.get("updates") {
        Some(Value::Null) | None => Value::Array(vec![]),
        Some(updates) => updates.clone(),
    };

    serde_json::from_value(updates_value).map_err(|source| EmitterClientError::MalformedResponse {
        line: line.to_string(),
        source,
    })
}

fn parse_hello_line(line: &str) -> Result<(), EmitterClientError> {
    let value: Value =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedHello {
            line: line.to_string(),
            source,
        })?;

    let object = value
        .as_object()
        .ok_or_else(|| EmitterClientError::UnexpectedHelloType {
            found: "<non_object>".to_string(),
            line: line.to_string(),
        })?;

    let message_type = message_type(object);
    if message_type != "hello" {
        return Err(EmitterClientError::UnexpectedHelloType {
            found: message_type,
            line: line.to_string(),
        });
    }

    let protocol = object
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if protocol != EMIT_PROTOCOL_V1 {
        return Err(EmitterClientError::HelloProtocolMismatch {
            expected: EMIT_PROTOCOL_V1,
            actual: protocol.to_string(),
        });
    }

    Ok(())
}

fn parse_sync_response_line(line: &str) -> Result<SyncResponse, EmitterClientError> {
    let value: Value =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedResponse {
            line: line.to_string(),
            source,
        })?;

    let object = value
        .as_object()
        .ok_or_else(|| EmitterClientError::UnexpectedResponseType {
            expected: SYNC_RESULT_MESSAGE_TYPE,
            found: "<non_object>".to_string(),
            line: line.to_string(),
        })?;

    match object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "sync_result" | "sync_response" | "sync" => Ok(SyncResponse {
            request_id: extract_correlation_id(object).unwrap_or_default(),
            stream_instance_id: object
                .get("stream_instance_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            updates: parse_updates(object, line)?,
        }),
        "error" => Err(EmitterClientError::DaemonError {
            code: parse_error_field(object, "code", "unknown_error"),
            message: parse_error_field(object, "message", "daemon returned error"),
            request_id: extract_correlation_id(object),
        }),
        _ => Err(EmitterClientError::UnexpectedResponseType {
            expected: SYNC_RESULT_MESSAGE_TYPE,
            found: message_type(object),
            line: line.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BubblePrecedence, ThoughtSource, ThoughtState};
    use chrono::Utc;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
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

    #[test]
    fn spawn_config_serializes_runtime_config() {
        let runtime = ThoughtConfig {
            enabled: false,
            model: "openrouter/custom".to_string(),
            cadence_hot_ms: 9_000,
            cadence_warm_ms: 60_000,
            cadence_cold_ms: 120_000,
            agent_prompt: Some("Agent prompt".to_string()),
            terminal_prompt: Some("Terminal prompt".to_string()),
        };

        let config = TmuxEmitSpawnConfig::from_runtime_config(
            &runtime,
            Duration::from_secs(15),
            "/tmp/throng-home/.tmux/clawgs-tmux.sock",
        )
        .expect("spawn config");
        assert_eq!(config.interval_ms, 15_000);
        assert_eq!(
            config.socket_path,
            PathBuf::from("/tmp/throng-home/.tmux/clawgs-tmux.sock")
        );
        let parsed: ThoughtConfig = serde_json::from_str(&config.config_json).expect("config json");
        assert!(!parsed.enabled);
        assert_eq!(parsed.model, "openrouter/custom");
        assert_eq!(parsed.agent_prompt.as_deref(), Some("Agent prompt"));
        assert_eq!(parsed.terminal_prompt.as_deref(), Some("Terminal prompt"));
    }

    #[tokio::test]
    async fn daemon_spawn_includes_socket_and_returns_sync_response() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let args_log = temp.path().join("args.log");
        let fake_bin = write_fake_clawgs_script(&args_log, temp.path());
        let socket_path = PathBuf::from("/tmp/throng-home/.tmux/clawgs-tmux.sock");
        let mut client = EmitterClient::with_bin_and_socket(
            fake_bin.to_string_lossy().into_owned(),
            &socket_path,
        );

        let response = client
            .next_sync_response(&ThoughtConfig::default(), Duration::from_millis(15_000))
            .await
            .expect("sync response");

        assert_eq!(response.request_id, "tmux-1");
        assert_eq!(response.stream_instance_id.as_deref(), Some("stream-a"));
        assert!(response.updates.is_empty());

        let logged = fs::read_to_string(&args_log).expect("read args log");
        let line = logged.lines().next().expect("spawned command line");
        assert!(line.contains("tmux-emit --interval-ms 15000"));
        assert!(line.contains("--socket /tmp/throng-home/.tmux/clawgs-tmux.sock"));
        assert!(line.contains("--config-json"));
    }

    #[tokio::test]
    async fn respawn_keeps_socket_while_updating_runtime_config_payload() {
        let _lock = TEST_ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let args_log = temp.path().join("args.log");
        let fake_bin = write_fake_clawgs_script(&args_log, temp.path());
        let socket_path = PathBuf::from("/tmp/throng-home/.tmux/clawgs-tmux.sock");
        let mut client = EmitterClient::with_bin_and_socket(
            fake_bin.to_string_lossy().into_owned(),
            &socket_path,
        );

        let baseline = ThoughtConfig::default();
        client
            .next_sync_response(&baseline, Duration::from_millis(15_000))
            .await
            .expect("initial sync response");

        let mut updated = baseline.clone();
        updated.agent_prompt = Some("Hook wakeup prompt".to_string());
        client
            .next_sync_response(&updated, Duration::from_millis(15_000))
            .await
            .expect("respawned sync response");

        let logged = fs::read_to_string(&args_log).expect("read args log");
        let lines: Vec<&str> = logged.lines().collect();
        assert_eq!(lines.len(), 2, "expected one spawn per runtime config");
        assert_ne!(lines[0], lines[1], "config change should respawn daemon");
        for line in &lines {
            assert!(line.contains("--socket /tmp/throng-home/.tmux/clawgs-tmux.sock"));
        }
        assert!(
            lines[1].contains("Hook\\u0020wakeup\\u0020prompt")
                || lines[1].contains("Hook wakeup prompt")
        );
    }

    fn write_fake_clawgs_script(args_log: &Path, dir: &Path) -> PathBuf {
        let script_path = dir.join("fake-clawgs.sh");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nprintf '%s\\n' '{{\"type\":\"hello\",\"protocol\":\"clawgs.emit.v1\",\"engine_version\":\"0.1.0\"}}'\nprintf '%s\\n' '{{\"type\":\"sync_result\",\"id\":\"tmux-1\",\"stream_instance_id\":\"stream-a\",\"updates\":[]}}'\nsleep 5\n",
            args_log.display()
        );
        fs::write(&script_path, script).expect("write fake clawgs");
        let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("chmod fake clawgs");
        script_path
    }
}
