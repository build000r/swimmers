use std::env;
use std::io;
use std::process::Stdio;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::warn;

use crate::thought::loop_runner::SessionInfo;
use crate::thought::protocol::{
    DaemonInboundMessage, SyncRequest, SyncResponse, EMIT_PROTOCOL_V1, SYNC_RESPONSE_MESSAGE_TYPE,
};

const DEFAULT_CLAWGS_BIN: &str = "clawgs";

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
        request_id: Option<u64>,
    },
    #[error("sync response request_id mismatch: expected {expected}, got {actual}")]
    RequestIdMismatch { expected: u64, actual: u64 },
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
    ) -> Result<SyncResponse, EmitterClientError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }

        let request = SyncRequest::from_session_snapshots(request_id, snapshots);
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

        let encoded = serde_json::to_string(&request)
            .map_err(|source| EmitterClientError::RequestSerialization { source })?;

        daemon
            .write_line(&encoded)
            .await
            .map_err(|source| EmitterClientError::RequestWrite { source })?;

        let response_line = daemon
            .read_non_empty_line()
            .await
            .map_err(|source| EmitterClientError::ResponseRead { source })?
            .ok_or(EmitterClientError::ResponseEof)?;

        parse_sync_response_line(&response_line, request.request_id)
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

fn parse_hello_line(line: &str) -> Result<(), EmitterClientError> {
    let message: DaemonInboundMessage =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedHello {
            line: line.to_string(),
            source,
        })?;

    match message {
        DaemonInboundMessage::Hello { protocol } if protocol == EMIT_PROTOCOL_V1 => Ok(()),
        DaemonInboundMessage::Hello { protocol } => {
            Err(EmitterClientError::HelloProtocolMismatch {
                expected: EMIT_PROTOCOL_V1,
                actual: protocol,
            })
        }
        other => Err(EmitterClientError::UnexpectedHelloType {
            found: other.message_type().to_string(),
            line: line.to_string(),
        }),
    }
}

fn parse_sync_response_line(
    line: &str,
    expected_request_id: u64,
) -> Result<SyncResponse, EmitterClientError> {
    let message: DaemonInboundMessage =
        serde_json::from_str(line).map_err(|source| EmitterClientError::MalformedResponse {
            line: line.to_string(),
            source,
        })?;

    match message {
        DaemonInboundMessage::SyncResponse {
            request_id,
            updates,
        } => {
            if request_id != expected_request_id {
                return Err(EmitterClientError::RequestIdMismatch {
                    expected: expected_request_id,
                    actual: request_id,
                });
            }

            Ok(SyncResponse {
                request_id,
                updates,
            })
        }
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
            expected: SYNC_RESPONSE_MESSAGE_TYPE,
            found: other.message_type().to_string(),
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
        let raw = r#"{"type":"hello","protocol":"clawgs.emit.v1"}"#;
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
            "type": "sync_response",
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

        let response =
            parse_sync_response_line(&raw, 9).expect("sync response should parse successfully");

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
    fn parse_sync_response_surfaces_daemon_error() {
        let raw =
            r#"{"type":"error","code":"bad_request","message":"invalid payload","request_id":4}"#;
        let err = parse_sync_response_line(raw, 4).expect_err("error envelope should fail sync");

        match err {
            EmitterClientError::DaemonError {
                code,
                message,
                request_id,
            } => {
                assert_eq!(code, "bad_request");
                assert_eq!(message, "invalid payload");
                assert_eq!(request_id, Some(4));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
