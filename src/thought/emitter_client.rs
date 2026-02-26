use std::env;
use std::io;
use std::process::Stdio;

use chrono::{SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::warn;

use crate::thought::loop_runner::SessionInfo;
use crate::thought::protocol::{
    SyncRequest, SyncRequestConfig, SyncResponse, SyncUpdate, EMIT_PROTOCOL_V1,
};
use crate::thought::runtime_config::ThoughtConfig;

const DEFAULT_CLAWGS_BIN: &str = "clawgs";
const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";

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

    async fn write_line(&mut self, line: &str) -> Result<(), io::Error> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await
    }

    fn is_exited(&mut self) -> Result<bool, io::Error> {
        Ok(self.child.try_wait()?.is_some())
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
    #[error("clawgs emit daemon closed stdout before replying to sync request")]
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
    #[error("sync response id mismatch: expected `{expected}`, got `{actual}`")]
    RequestIdMismatch { expected: String, actual: String },
    #[error("unexpected daemon message `{found}` while waiting for `{expected}`: {line}")]
    UnexpectedResponseType {
        expected: &'static str,
        found: String,
        line: String,
    },
    #[error("sync request must serialize to a JSON object")]
    InvalidRequestShape,
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
    next_request_id: u64,
}

impl Default for EmitterClient {
    fn default() -> Self {
        Self::new()
    }
}

impl EmitterClient {
    pub fn new() -> Self {
        Self::with_bin(resolve_clawgs_bin())
    }

    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self {
            bin: bin.into(),
            daemon: None,
            next_request_id: 1,
        }
    }

    pub async fn sync_sessions(
        &mut self,
        snapshots: &[SessionInfo],
        runtime_config: &ThoughtConfig,
    ) -> Result<SyncResponse, EmitterClientError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }

        let request = SyncRequest::from_session_snapshots_with_config(
            request_id,
            snapshots,
            SyncRequestConfig::from(runtime_config),
        );
        self.sync(request).await
    }

    pub async fn sync(&mut self, request: SyncRequest) -> Result<SyncResponse, EmitterClientError> {
        match self.sync_once(request.clone()).await {
            Ok(response) => Ok(response),
            Err(first_err) if first_err.is_retryable() => {
                warn!(
                    error = %first_err,
                    "clawgs emit sync failed; restarting daemon and retrying once"
                );
                self.restart().await?;
                self.sync_once(request).await
            }
            Err(err) => Err(err),
        }
    }

    pub async fn restart(&mut self) -> Result<(), EmitterClientError> {
        self.stop_current_daemon().await;
        self.daemon = Some(self.spawn_daemon().await?);
        Ok(())
    }

    async fn sync_once(
        &mut self,
        request: SyncRequest,
    ) -> Result<SyncResponse, EmitterClientError> {
        self.ensure_running().await?;

        let daemon = self
            .daemon
            .as_mut()
            .expect("daemon must exist after ensure_running");

        let (encoded, expected_id) = normalize_sync_request(&request)?;

        daemon
            .write_line(&encoded)
            .await
            .map_err(|source| EmitterClientError::RequestWrite { source })?;

        let response_line = daemon
            .read_non_empty_line()
            .await
            .map_err(|source| EmitterClientError::ResponseRead { source })?
            .ok_or(EmitterClientError::ResponseEof)?;

        parse_sync_response_line(&response_line, &expected_id)
    }

    async fn ensure_running(&mut self) -> Result<(), EmitterClientError> {
        let should_spawn = match self.daemon.as_mut() {
            Some(daemon) => daemon
                .is_exited()
                .map_err(|source| EmitterClientError::StatusCheck { source })?,
            None => true,
        };

        if should_spawn {
            self.daemon = None;
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

fn resolve_clawgs_bin() -> String {
    env::var("CLAWGS_BIN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_CLAWGS_BIN.to_string())
}

fn normalize_sync_request(request: &SyncRequest) -> Result<(String, String), EmitterClientError> {
    let mut value = serde_json::to_value(request)
        .map_err(|source| EmitterClientError::RequestSerialization { source })?;

    let object = value
        .as_object_mut()
        .ok_or(EmitterClientError::InvalidRequestShape)?;

    object.insert("type".to_string(), Value::String("sync".to_string()));

    let request_id =
        extract_correlation_id(object).unwrap_or_else(|| Utc::now().timestamp_millis().to_string());
    object.insert("id".to_string(), Value::String(request_id.clone()));

    object
        .entry("now".to_string())
        .or_insert_with(|| Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)));
    object
        .entry("config".to_string())
        .or_insert_with(default_sync_config);
    object
        .entry("sessions".to_string())
        .or_insert_with(|| Value::Array(vec![]));

    let encoded = serde_json::to_string(&value)
        .map_err(|source| EmitterClientError::RequestSerialization { source })?;

    Ok((encoded, request_id))
}

fn default_sync_config() -> Value {
    serde_json::to_value(SyncRequestConfig::default()).unwrap_or_else(|_| {
        json!({
            "enabled": true,
            "model": "",
            "cadence_hot_ms": 15_000,
            "cadence_warm_ms": 45_000,
            "cadence_cold_ms": 120_000,
            "agent_prompt": "",
            "terminal_prompt": ""
        })
    })
}

fn extract_correlation_id(object: &Map<String, Value>) -> Option<String> {
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

fn require_matching_id(
    object: &Map<String, Value>,
    expected_id: &str,
) -> Result<Option<String>, EmitterClientError> {
    let actual_id = extract_correlation_id(object);
    if let Some(actual_id_value) = actual_id.as_ref() {
        if actual_id_value != expected_id {
            return Err(EmitterClientError::RequestIdMismatch {
                expected: expected_id.to_string(),
                actual: actual_id_value.clone(),
            });
        }
    }
    Ok(actual_id)
}

fn message_type(object: &Map<String, Value>) -> String {
    object
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "<missing>".to_string())
}

fn parse_updates(
    object: &Map<String, Value>,
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

fn parse_error_field(object: &Map<String, Value>, field: &str, fallback: &str) -> String {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| object.get(field).map(Value::to_string))
        .unwrap_or_else(|| fallback.to_string())
}

fn resolve_numeric_request_id(
    object: &Map<String, Value>,
    expected_id: &str,
    actual_id: Option<&str>,
) -> u64 {
    object
        .get("request_id")
        .and_then(Value::as_u64)
        .or_else(|| actual_id.and_then(|id| id.parse::<u64>().ok()))
        .or_else(|| expected_id.parse::<u64>().ok())
        .unwrap_or_default()
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

fn parse_sync_response_line(
    line: &str,
    expected_id: &str,
) -> Result<SyncResponse, EmitterClientError> {
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
        "sync_result" | "sync_response" | "sync" => {
            let actual_id = require_matching_id(object, expected_id)?;
            let updates = parse_updates(object, line)?;

            Ok(SyncResponse {
                request_id: resolve_numeric_request_id(object, expected_id, actual_id.as_deref()),
                updates,
            })
        }
        "error" => {
            let actual_id = require_matching_id(object, expected_id)?;
            Err(EmitterClientError::DaemonError {
                code: parse_error_field(object, "code", "unknown_error"),
                message: parse_error_field(object, "message", "daemon returned error"),
                request_id: actual_id,
            })
        }
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
    fn parse_sync_response_extracts_updates() {
        let now = Utc::now();
        let raw = serde_json::json!({
            "type": "sync_result",
            "id": "req-9",
            "request_id": 9,
            "updates": [
                {
                    "session_id": "sess-1",
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

        let response = parse_sync_response_line(&raw, "req-9")
            .expect("sync response should parse successfully");

        assert_eq!(response.request_id, 9);
        assert_eq!(response.updates.len(), 1);
        let update = &response.updates[0];
        assert_eq!(update.session_id, "sess-1");
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
    fn parse_sync_response_detects_id_mismatch() {
        let raw = r#"{"type":"sync_result","id":"req-daemon","updates":[]}"#;
        let err =
            parse_sync_response_line(raw, "req-client").expect_err("id mismatch should fail sync");

        match err {
            EmitterClientError::RequestIdMismatch { expected, actual } => {
                assert_eq!(expected, "req-client");
                assert_eq!(actual, "req-daemon");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn parse_sync_response_surfaces_daemon_error() {
        let raw =
            r#"{"type":"error","id":"req-4","code":"bad_request","message":"invalid payload"}"#;
        let err =
            parse_sync_response_line(raw, "req-4").expect_err("error envelope should fail sync");

        match err {
            EmitterClientError::DaemonError {
                code,
                message,
                request_id,
            } => {
                assert_eq!(code, "bad_request");
                assert_eq!(message, "invalid payload");
                assert_eq!(request_id, Some("req-4".to_string()));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn normalize_sync_request_uses_clawgs_wire_shape() {
        let request = SyncRequest::from_session_snapshots(7, &[]);
        let (encoded, expected_id) =
            normalize_sync_request(&request).expect("sync request should normalize");
        let parsed: Value =
            serde_json::from_str(&encoded).expect("normalized request should be valid JSON");

        assert_eq!(expected_id, "7");
        assert_eq!(parsed["type"], "sync");
        assert_eq!(parsed["id"], "7");
        assert!(parsed.get("request_id").is_none());
        assert!(parsed["now"].is_string());
        assert_eq!(parsed["config"]["enabled"], true);
        assert_eq!(parsed["config"]["model"], "");
        assert_eq!(parsed["config"]["agent_prompt"], "");
        assert_eq!(parsed["config"]["terminal_prompt"], "");
        assert!(parsed["sessions"].as_array().is_some());
    }

    #[test]
    fn normalize_sync_request_generates_id_when_blank() {
        let mut request = SyncRequest::from_session_snapshots(22, &[]);
        request.id = "   ".to_string();

        let (encoded, expected_id) =
            normalize_sync_request(&request).expect("sync request should normalize");
        let parsed: Value =
            serde_json::from_str(&encoded).expect("normalized request should be valid JSON");

        assert!(!expected_id.is_empty());
        assert_eq!(parsed["id"], expected_id);
    }

    #[test]
    fn parse_sync_response_accepts_legacy_string_request_id_field() {
        let raw = r#"{"type":"sync_response","request_id":"123","updates":[]}"#;
        let response = parse_sync_response_line(raw, "123")
            .expect("legacy string request_id should parse successfully");
        assert_eq!(response.request_id, 123);
        assert!(response.updates.is_empty());
    }
}
