use crate::types::{
    AuthorizedProviderResumeReceipt, ProviderResumeCaptureSource, ProviderResumeContractError,
    ProviderResumeProvider,
};
use serde_json::{json, Map, Value};
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, Lines,
};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};

const INITIALIZE_REQUEST_ID: u64 = 1;
const THREAD_START_REQUEST_ID: u64 = 2;
const TURN_START_REQUEST_ID: u64 = 3;
const DEFAULT_LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexAppServerStage {
    InitializeWrite,
    InitializeResponse,
    InitializedWrite,
    ThreadStartWrite,
    ThreadStartResponse,
    ThreadStartedNotification,
    TurnStartWrite,
    TurnStartResponse,
    TurnStartedNotification,
    ActiveSession,
}

impl fmt::Display for CodexAppServerStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InitializeWrite => "initialize write",
            Self::InitializeResponse => "initialize response",
            Self::InitializedWrite => "initialized write",
            Self::ThreadStartWrite => "thread/start write",
            Self::ThreadStartResponse => "thread/start response",
            Self::ThreadStartedNotification => "thread/started notification",
            Self::TurnStartWrite => "turn/start write",
            Self::TurnStartResponse => "turn/start response",
            Self::TurnStartedNotification => "turn/started notification",
            Self::ActiveSession => "active app-server session",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Error)]
pub enum CodexAppServerError {
    #[error("Codex app-server cwd must be absolute: {0}")]
    RelativeCwd(PathBuf),
    #[error("Codex app-server cwd is not valid UTF-8: {0:?}")]
    NonUtf8Cwd(PathBuf),
    #[error("Codex app-server initial prompt cannot be empty")]
    EmptyInitialPrompt,
    #[error("failed to spawn Codex app-server: {0}")]
    Spawn(#[source] io::Error),
    #[error("Codex app-server did not expose piped {0}")]
    MissingStdio(&'static str),
    #[error("Codex app-server {stage} timed out")]
    Timeout { stage: CodexAppServerStage },
    #[error("Codex app-server {stage} was cancelled")]
    Cancelled { stage: CodexAppServerStage },
    #[error("Codex app-server transport failed during {stage}: {source}")]
    Transport {
        stage: CodexAppServerStage,
        #[source]
        source: io::Error,
    },
    #[error("Codex app-server closed stdout during {stage}")]
    EndOfStream { stage: CodexAppServerStage },
    #[error("Codex app-server sent invalid JSON during {stage}: {source}")]
    InvalidJson {
        stage: CodexAppServerStage,
        #[source]
        source: serde_json::Error,
    },
    #[error("Codex app-server returned response id {actual:?}; expected {expected}")]
    ResponseIdMismatch { expected: u64, actual: Value },
    #[error("Codex app-server returned an invalid response during {stage}: {reason}")]
    InvalidResponse {
        stage: CodexAppServerStage,
        reason: &'static str,
    },
    #[error("Codex app-server RPC failed during {stage}: code {code}, {message}")]
    Rpc {
        stage: CodexAppServerStage,
        code: i64,
        message: String,
    },
    #[error("Codex thread/start response did not contain thread.id")]
    MissingThread,
    #[error("Codex thread/started notification did not contain thread.id")]
    MissingThreadNotification,
    #[error(
        "Codex thread correlation mismatch: response thread {response_id:?}, notification thread {notification_id:?}"
    )]
    ThreadCorrelationMismatch {
        response_id: String,
        notification_id: String,
    },
    #[error("Codex turn/start response did not contain turn.id")]
    MissingTurn,
    #[error("Codex turn/started notification did not contain threadId and turn.id")]
    MissingTurnNotification,
    #[error(
        "Codex turn correlation mismatch: expected thread {expected_thread:?}/turn {response_turn:?}, notification thread {notification_thread:?}/turn {notification_turn:?}"
    )]
    TurnCorrelationMismatch {
        expected_thread: String,
        response_turn: String,
        notification_thread: String,
        notification_turn: String,
    },
    #[error("Codex provider resume receipt violated shared contract: {0}")]
    ResumeContract(#[from] ProviderResumeContractError),
}

#[derive(Debug)]
pub struct CodexAppServerLaunchError {
    kind: CodexAppServerError,
    provider_resume: AuthorizedProviderResumeReceipt,
}

impl CodexAppServerLaunchError {
    fn new(kind: CodexAppServerError, capture_source: ProviderResumeCaptureSource) -> Self {
        Self {
            kind,
            provider_resume: AuthorizedProviderResumeReceipt::unknown(
                ProviderResumeProvider::Codex,
                capture_source,
            ),
        }
    }

    pub fn kind(&self) -> &CodexAppServerError {
        &self.kind
    }

    /// Failures never expose a guessed thread id or claim resumable capability.
    pub fn provider_resume(&self) -> &AuthorizedProviderResumeReceipt {
        &self.provider_resume
    }
}

impl fmt::Display for CodexAppServerLaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl std::error::Error for CodexAppServerLaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

#[derive(Debug)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

/// Cloneable cancellation signal for a launch still in protocol negotiation.
#[derive(Clone, Debug)]
pub struct CodexAppServerCancellation {
    state: Arc<CancellationState>,
}

impl Default for CodexAppServerCancellation {
    fn default() -> Self {
        Self {
            state: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }
}

impl CodexAppServerCancellation {
    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.state.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Clone, Debug)]
pub struct CodexAppServerLaunchRequest {
    pub codex_binary: PathBuf,
    pub cwd: PathBuf,
    pub thread_config: Map<String, Value>,
    pub initial_prompt: String,
    pub timeout: Duration,
    pub cancellation: CodexAppServerCancellation,
}

impl CodexAppServerLaunchRequest {
    pub fn new(
        cwd: impl Into<PathBuf>,
        thread_config: Map<String, Value>,
        initial_prompt: impl Into<String>,
    ) -> Self {
        Self {
            codex_binary: PathBuf::from("codex"),
            cwd: cwd.into(),
            thread_config,
            initial_prompt: initial_prompt.into(),
            timeout: DEFAULT_LAUNCH_TIMEOUT,
            cancellation: CodexAppServerCancellation::default(),
        }
    }
}

#[derive(Debug)]
pub struct CodexAppServerLaunch {
    provider_resume: AuthorizedProviderResumeReceipt,
    turn_id: String,
    session: CodexAppServerSession,
}

impl CodexAppServerLaunch {
    pub fn provider_resume(&self) -> &AuthorizedProviderResumeReceipt {
        &self.provider_resume
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn session(&mut self) -> &mut CodexAppServerSession {
        &mut self.session
    }

    pub fn into_session(self) -> CodexAppServerSession {
        self.session
    }
}

#[derive(Debug)]
pub struct CodexAppServerSession {
    child: Child,
    transport: JsonlTransport<ChildStdout, ChildStdin>,
}

impl CodexAppServerSession {
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub async fn send_message(&mut self, message: &Value) -> Result<(), CodexAppServerError> {
        self.transport
            .send(message)
            .await
            .map_err(|source| CodexAppServerError::Transport {
                stage: CodexAppServerStage::ActiveSession,
                source,
            })
    }

    pub async fn next_message(&mut self) -> Result<Option<Value>, CodexAppServerError> {
        let Some(line) =
            self.transport
                .next_line()
                .await
                .map_err(|source| CodexAppServerError::Transport {
                    stage: CodexAppServerStage::ActiveSession,
                    source,
                })?
        else {
            return Ok(None);
        };
        serde_json::from_str(&line)
            .map(Some)
            .map_err(|source| CodexAppServerError::InvalidJson {
                stage: CodexAppServerStage::ActiveSession,
                source,
            })
    }

    pub async fn terminate(mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.start_kill()?;
        }
        self.child.wait().await.map(|_| ())
    }
}

#[derive(Debug)]
struct JsonlTransport<R, W> {
    lines: Lines<BufReader<R>>,
    writer: BufWriter<W>,
}

impl<R, W> JsonlTransport<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    fn new(reader: R, writer: W) -> Self {
        Self {
            lines: BufReader::new(reader).lines(),
            writer: BufWriter::new(writer),
        }
    }

    async fn send(&mut self, message: &Value) -> io::Result<()> {
        let encoded = serde_json::to_vec(message)
            .expect("serializing serde_json::Value into JSON cannot fail");
        self.writer.write_all(&encoded).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await
    }

    async fn next_line(&mut self) -> io::Result<Option<String>> {
        self.lines.next_line().await
    }
}

#[derive(Debug)]
struct ProtocolLaunch {
    provider_resume: AuthorizedProviderResumeReceipt,
    turn_id: String,
}

#[derive(Debug)]
struct ProtocolFailure {
    kind: CodexAppServerError,
    capture_source: ProviderResumeCaptureSource,
}

impl ProtocolFailure {
    fn new(kind: CodexAppServerError, capture_source: ProviderResumeCaptureSource) -> Self {
        Self {
            kind,
            capture_source,
        }
    }

    fn unknown(kind: CodexAppServerError) -> Self {
        Self::new(kind, ProviderResumeCaptureSource::Unknown)
    }

    fn response(kind: CodexAppServerError) -> Self {
        Self::new(kind, ProviderResumeCaptureSource::ProviderResponse)
    }

    fn notification(kind: CodexAppServerError) -> Self {
        Self::new(kind, ProviderResumeCaptureSource::ProviderNotification)
    }

    fn into_launch_error(self) -> CodexAppServerLaunchError {
        CodexAppServerLaunchError::new(self.kind, self.capture_source)
    }
}

pub async fn launch_codex_app_server(
    request: CodexAppServerLaunchRequest,
) -> Result<CodexAppServerLaunch, CodexAppServerLaunchError> {
    validate_request(&request)
        .map_err(ProtocolFailure::unknown)
        .map_err(ProtocolFailure::into_launch_error)?;
    if request.cancellation.is_cancelled() {
        return Err(ProtocolFailure::unknown(CodexAppServerError::Cancelled {
            stage: CodexAppServerStage::InitializeWrite,
        })
        .into_launch_error());
    }

    let mut command = Command::new(&request.codex_binary);
    command
        .args(["app-server", "--stdio"])
        .current_dir(&request.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|source| {
        ProtocolFailure::unknown(CodexAppServerError::Spawn(source)).into_launch_error()
    })?;
    let stdin = child.stdin.take().ok_or_else(|| {
        ProtocolFailure::unknown(CodexAppServerError::MissingStdio("stdin")).into_launch_error()
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ProtocolFailure::unknown(CodexAppServerError::MissingStdio("stdout")).into_launch_error()
    })?;
    let mut transport = JsonlTransport::new(stdout, stdin);

    let protocol = match run_protocol(
        &mut transport,
        &request.cwd,
        &request.thread_config,
        &request.initial_prompt,
        request.timeout,
        &request.cancellation,
    )
    .await
    {
        Ok(protocol) => protocol,
        Err(failure) => {
            drop(transport);
            terminate_failed_child(&mut child).await;
            return Err(failure.into_launch_error());
        }
    };

    Ok(CodexAppServerLaunch {
        provider_resume: protocol.provider_resume,
        turn_id: protocol.turn_id,
        session: CodexAppServerSession { child, transport },
    })
}

fn validate_request(request: &CodexAppServerLaunchRequest) -> Result<(), CodexAppServerError> {
    if !request.cwd.is_absolute() {
        return Err(CodexAppServerError::RelativeCwd(request.cwd.clone()));
    }
    if request.cwd.to_str().is_none() {
        return Err(CodexAppServerError::NonUtf8Cwd(request.cwd.clone()));
    }
    if request.initial_prompt.trim().is_empty() {
        return Err(CodexAppServerError::EmptyInitialPrompt);
    }
    Ok(())
}

async fn terminate_failed_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

async fn run_protocol<R, W>(
    transport: &mut JsonlTransport<R, W>,
    cwd: &Path,
    thread_config: &Map<String, Value>,
    initial_prompt: &str,
    timeout: Duration,
    cancellation: &CodexAppServerCancellation,
) -> Result<ProtocolLaunch, ProtocolFailure>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let deadline = Instant::now() + timeout;

    send_bounded(
        transport,
        &json!({
            "method": "initialize",
            "id": INITIALIZE_REQUEST_ID,
            "params": {
                "clientInfo": {
                    "name": "swimmers",
                    "title": "Swimmers",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
        CodexAppServerStage::InitializeWrite,
        deadline,
        cancellation,
    )
    .await
    .map_err(ProtocolFailure::unknown)?;
    wait_for_response(
        transport,
        INITIALIZE_REQUEST_ID,
        CodexAppServerStage::InitializeResponse,
        deadline,
        cancellation,
    )
    .await
    .map_err(ProtocolFailure::unknown)?;
    send_bounded(
        transport,
        &json!({"method": "initialized", "params": {}}),
        CodexAppServerStage::InitializedWrite,
        deadline,
        cancellation,
    )
    .await
    .map_err(ProtocolFailure::unknown)?;

    let cwd = cwd
        .to_str()
        .expect("request validation guarantees a UTF-8 cwd");
    send_bounded(
        transport,
        &json!({
            "method": "thread/start",
            "id": THREAD_START_REQUEST_ID,
            "params": {
                "cwd": cwd,
                "config": Value::Object(thread_config.clone())
            }
        }),
        CodexAppServerStage::ThreadStartWrite,
        deadline,
        cancellation,
    )
    .await
    .map_err(ProtocolFailure::response)?;

    let thread_id = correlate_thread_start(transport, deadline, cancellation).await?;

    send_bounded(
        transport,
        &json!({
            "method": "turn/start",
            "id": TURN_START_REQUEST_ID,
            "params": {
                "threadId": thread_id,
                "input": [{
                    "type": "text",
                    "text": initial_prompt
                }]
            }
        }),
        CodexAppServerStage::TurnStartWrite,
        deadline,
        cancellation,
    )
    .await
    .map_err(ProtocolFailure::response)?;

    let turn_id = correlate_turn_start(transport, &thread_id, deadline, cancellation).await?;
    let resume_argv = vec!["codex".to_string(), "resume".to_string(), thread_id.clone()];
    let resume_command = format!("codex resume {}", display_arg(&thread_id));
    let provider_resume = AuthorizedProviderResumeReceipt::resumable(
        ProviderResumeProvider::Codex,
        thread_id,
        resume_argv,
        resume_command,
        ProviderResumeCaptureSource::ProviderResponse,
    )
    .map_err(|error| ProtocolFailure::response(CodexAppServerError::ResumeContract(error)))?;

    Ok(ProtocolLaunch {
        provider_resume,
        turn_id,
    })
}

async fn wait_for_response<R, W>(
    transport: &mut JsonlTransport<R, W>,
    expected_id: u64,
    stage: CodexAppServerStage,
    deadline: Instant,
    cancellation: &CodexAppServerCancellation,
) -> Result<Value, CodexAppServerError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let message = receive_bounded(transport, stage, deadline, cancellation).await?;
        if message.get("id").is_none() {
            continue;
        }
        return extract_response(message, expected_id, stage);
    }
}

async fn correlate_thread_start<R, W>(
    transport: &mut JsonlTransport<R, W>,
    deadline: Instant,
    cancellation: &CodexAppServerCancellation,
) -> Result<String, ProtocolFailure>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut response_id: Option<String> = None;
    let mut notification_id: Option<String> = None;

    loop {
        let stage = if response_id.is_none() {
            CodexAppServerStage::ThreadStartResponse
        } else {
            CodexAppServerStage::ThreadStartedNotification
        };
        let message = receive_bounded(transport, stage, deadline, cancellation)
            .await
            .map_err(|kind| {
                if response_id.is_some() {
                    ProtocolFailure::notification(kind)
                } else {
                    ProtocolFailure::response(kind)
                }
            })?;

        if message.get("id").is_some() {
            let result = extract_response(
                message,
                THREAD_START_REQUEST_ID,
                CodexAppServerStage::ThreadStartResponse,
            )
            .map_err(ProtocolFailure::response)?;
            let thread_id = nested_nonempty_string(&result, &["thread", "id"])
                .ok_or_else(|| ProtocolFailure::response(CodexAppServerError::MissingThread))?;
            response_id = Some(thread_id);
        } else if message.get("method").and_then(Value::as_str) == Some("thread/started") {
            let thread_id = nested_nonempty_string(&message, &["params", "thread", "id"])
                .ok_or_else(|| {
                    ProtocolFailure::notification(CodexAppServerError::MissingThreadNotification)
                })?;
            notification_id = Some(thread_id);
        }

        if let (Some(response), Some(notification)) = (&response_id, &notification_id) {
            if response != notification {
                return Err(ProtocolFailure::notification(
                    CodexAppServerError::ThreadCorrelationMismatch {
                        response_id: response.clone(),
                        notification_id: notification.clone(),
                    },
                ));
            }
            return Ok(response.clone());
        }
    }
}

async fn correlate_turn_start<R, W>(
    transport: &mut JsonlTransport<R, W>,
    expected_thread: &str,
    deadline: Instant,
    cancellation: &CodexAppServerCancellation,
) -> Result<String, ProtocolFailure>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut response_turn: Option<String> = None;
    let mut notification: Option<(String, String)> = None;

    loop {
        let stage = if response_turn.is_none() {
            CodexAppServerStage::TurnStartResponse
        } else {
            CodexAppServerStage::TurnStartedNotification
        };
        let message = receive_bounded(transport, stage, deadline, cancellation)
            .await
            .map_err(|kind| {
                if response_turn.is_some() {
                    ProtocolFailure::notification(kind)
                } else {
                    ProtocolFailure::response(kind)
                }
            })?;

        if message.get("id").is_some() {
            let result = extract_response(
                message,
                TURN_START_REQUEST_ID,
                CodexAppServerStage::TurnStartResponse,
            )
            .map_err(ProtocolFailure::response)?;
            let turn_id = nested_nonempty_string(&result, &["turn", "id"])
                .ok_or_else(|| ProtocolFailure::response(CodexAppServerError::MissingTurn))?;
            response_turn = Some(turn_id);
        } else if message.get("method").and_then(Value::as_str) == Some("turn/started") {
            let thread_id = nested_nonempty_string(&message, &["params", "threadId"]);
            let turn_id = nested_nonempty_string(&message, &["params", "turn", "id"]);
            let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else {
                return Err(ProtocolFailure::notification(
                    CodexAppServerError::MissingTurnNotification,
                ));
            };
            notification = Some((thread_id, turn_id));
        }

        if let (Some(response_turn), Some((notification_thread, notification_turn))) =
            (&response_turn, &notification)
        {
            if notification_thread != expected_thread || notification_turn != response_turn {
                return Err(ProtocolFailure::notification(
                    CodexAppServerError::TurnCorrelationMismatch {
                        expected_thread: expected_thread.to_string(),
                        response_turn: response_turn.clone(),
                        notification_thread: notification_thread.clone(),
                        notification_turn: notification_turn.clone(),
                    },
                ));
            }
            return Ok(response_turn.clone());
        }
    }
}

fn nested_nonempty_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
}

fn extract_response(
    message: Value,
    expected_id: u64,
    stage: CodexAppServerStage,
) -> Result<Value, CodexAppServerError> {
    let actual_id = message
        .get("id")
        .cloned()
        .ok_or(CodexAppServerError::InvalidResponse {
            stage,
            reason: "missing id",
        })?;
    if actual_id.as_u64() != Some(expected_id) {
        return Err(CodexAppServerError::ResponseIdMismatch {
            expected: expected_id,
            actual: actual_id,
        });
    }
    if let Some(error) = message.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown app-server error")
            .to_string();
        return Err(CodexAppServerError::Rpc {
            stage,
            code,
            message,
        });
    }
    message
        .get("result")
        .cloned()
        .ok_or(CodexAppServerError::InvalidResponse {
            stage,
            reason: "missing result and error",
        })
}

async fn send_bounded<R, W>(
    transport: &mut JsonlTransport<R, W>,
    message: &Value,
    stage: CodexAppServerStage,
    deadline: Instant,
    cancellation: &CodexAppServerCancellation,
) -> Result<(), CodexAppServerError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    bounded_io(stage, deadline, cancellation, transport.send(message)).await
}

async fn receive_bounded<R, W>(
    transport: &mut JsonlTransport<R, W>,
    stage: CodexAppServerStage,
    deadline: Instant,
    cancellation: &CodexAppServerCancellation,
) -> Result<Value, CodexAppServerError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let line = bounded_io(stage, deadline, cancellation, transport.next_line())
        .await?
        .ok_or(CodexAppServerError::EndOfStream { stage })?;
    serde_json::from_str(&line).map_err(|source| CodexAppServerError::InvalidJson { stage, source })
}

async fn bounded_io<T, F>(
    stage: CodexAppServerStage,
    deadline: Instant,
    cancellation: &CodexAppServerCancellation,
    operation: F,
) -> Result<T, CodexAppServerError>
where
    F: Future<Output = io::Result<T>>,
{
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(CodexAppServerError::Cancelled { stage }),
        _ = sleep_until(deadline) => Err(CodexAppServerError::Timeout { stage }),
        result = operation => result.map_err(|source| CodexAppServerError::Transport { stage, source }),
    }
}

fn display_arg(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
#[path = "tests/codex_app_server.rs"]
mod tests;
