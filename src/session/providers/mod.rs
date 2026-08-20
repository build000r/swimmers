#[allow(dead_code)]
mod codex_app_server;
#[allow(dead_code)]
mod grok_cli;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::Map;
use uuid::Uuid;

use crate::launcher::{create_private_file, prepare_private_dir};
use crate::types::{
    AuthorizedProviderResumeLaunchReceipt, AuthorizedProviderResumeReceipt, CassAdmissionCommandOp,
    CassAdmissionCommandRequest, CassAdmissionCommandResponse, CassAdmissionError,
    CassAdmissionReservationRef, CassAdmissionSubject, CassOrigin, CassProviderIdentity,
    LaunchReceipt, ProviderResumeCaptureSource, ProviderResumeProvider, SpawnTool,
    CASS_ADMISSION_COMMAND_SCHEMA,
};

use self::codex_app_server::{launch_codex_app_server, CodexAppServerLaunchRequest};
use self::grok_cli::{GrokCliProvider, GrokLaunchObservation, PreparedGrokLaunch};

const RECEIPT_DIR: &str = "provider_resume_receipts";
const GROK_PROMPT_DIR: &str = "provider_launch_prompts";
const GROK_START_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const GROK_START_CONFIRM_POLL: std::time::Duration = std::time::Duration::from_millis(20);
const CASS_ADMISSION_CMD_ENV: &str = "SKILLBOX_CASS_ADMISSION_CMD_JSON";
const CASS_ADMISSION_TIMEOUT_ENV: &str = "SKILLBOX_CASS_ADMISSION_TIMEOUT_MS";
const DEFAULT_CASS_ADMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(test)]
static CODEX_APP_SERVER_LAUNCH_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static LAST_CASS_PROVIDER_IDENTITY: std::sync::Mutex<Option<CassProviderIdentity>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
static LAST_CASS_COMMAND_OPS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub(super) struct PreparedProviderLaunch {
    command: String,
    cleanup_paths: Vec<PathBuf>,
    receipt: PreparedProviderReceipt,
}

enum PreparedProviderReceipt {
    Captured(AuthorizedProviderResumeReceipt),
    Grok {
        provider: GrokCliProvider,
        launch: PreparedGrokLaunch,
        confirmation: GrokLaunchConfirmation,
        confirmed: bool,
    },
}

impl PreparedProviderLaunch {
    pub(super) fn command(&self) -> &str {
        &self.command
    }

    #[allow(dead_code)]
    pub(super) fn provider_conversation_id(&self) -> Option<&str> {
        self.captured_resume()
            .and_then(AuthorizedProviderResumeReceipt::conversation_id)
    }

    pub(super) fn captured_resume(&self) -> Option<&AuthorizedProviderResumeReceipt> {
        match &self.receipt {
            PreparedProviderReceipt::Captured(receipt) => Some(receipt),
            PreparedProviderReceipt::Grok { .. } => None,
        }
    }

    pub(super) fn take_cleanup_paths(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.cleanup_paths)
    }

    pub(super) async fn confirm_started(&mut self) -> anyhow::Result<()> {
        let PreparedProviderReceipt::Grok {
            confirmation,
            confirmed,
            ..
        } = &mut self.receipt
        else {
            return Ok(());
        };
        confirmation.wait().await?;
        *confirmed = true;
        Ok(())
    }

    pub(super) fn finalize(
        self,
        launch_receipt: LaunchReceipt,
    ) -> anyhow::Result<AuthorizedProviderResumeLaunchReceipt> {
        match self.receipt {
            PreparedProviderReceipt::Captured(provider_resume) => Ok(
                AuthorizedProviderResumeLaunchReceipt::new(launch_receipt, provider_resume),
            ),
            PreparedProviderReceipt::Grok {
                provider,
                launch,
                confirmed,
                ..
            } => {
                anyhow::ensure!(
                    confirmed,
                    "Grok provider launch was not observed running in tmux"
                );
                let observation =
                    GrokLaunchObservation::running(launch.argv(), launch.cwd().to_path_buf());
                provider
                    .verify_observed_launch(&launch, observation)
                    .context("Grok provider launch correlation failed")?
                    .bind_receipt(launch_receipt)
                    .context("Grok provider receipt binding failed")
            }
        }
    }
}

pub(super) async fn prepare_provider_launch(
    tool: Option<SpawnTool>,
    cwd: Option<&str>,
    initial_request: Option<&str>,
) -> anyhow::Result<Option<PreparedProviderLaunch>> {
    let Some(tool) = tool else {
        return Ok(None);
    };
    let Some(cwd) = cwd else {
        return Ok(None);
    };
    let cwd = absolute_cwd(cwd)?;

    match tool {
        SpawnTool::Codex => {
            let Some(initial_request) = initial_request else {
                return Ok(None);
            };
            record_codex_app_server_launch();
            if let Some(launch) = cass_test_provider_launch(&cwd, initial_request)? {
                return Ok(Some(launch));
            }
            let request =
                CodexAppServerLaunchRequest::new(&cwd, Map::new(), initial_request.to_string());
            let launch = launch_codex_app_server(request)
                .await
                .context("Codex app-server provider launch failed")?;
            let provider_resume = launch.provider_resume().clone();
            let command = provider_resume
                .resume_command()
                .context("Codex provider omitted resume command")?
                .to_string();
            launch
                .into_session()
                .terminate()
                .await
                .context("failed to terminate Codex app-server handoff process")?;
            Ok(Some(PreparedProviderLaunch {
                command,
                cleanup_paths: Vec::new(),
                receipt: PreparedProviderReceipt::Captured(provider_resume),
            }))
        }
        SpawnTool::Grok => prepare_grok_launch(&cwd, initial_request).map(Some),
        SpawnTool::Claude => Ok(None),
    }
}

fn prepare_grok_launch(
    cwd: &Path,
    initial_request: Option<&str>,
) -> anyhow::Result<PreparedProviderLaunch> {
    let provider = GrokCliProvider::from_env().context("failed to initialize Grok provider")?;
    let mut trailing_args = Vec::new();
    let mut cleanup_paths = Vec::new();
    if let Some(initial_request) = initial_request {
        let prompt_path = write_provider_prompt(initial_request)?;
        trailing_args.push(OsString::from("--prompt-file"));
        trailing_args.push(prompt_path.as_os_str().to_owned());
        cleanup_paths.push(prompt_path);
    }
    trailing_args.push(OsString::from("--always-approve"));
    trailing_args.push(OsString::from("--no-alt-screen"));

    let launch = match provider.prepare_launch(cwd, trailing_args) {
        Ok(launch) => launch,
        Err(error) => {
            remove_prelaunch_files(&cleanup_paths);
            return Err(error).context("failed to prepare Grok provider launch");
        }
    };
    let confirmation = match GrokLaunchConfirmation::new() {
        Ok(confirmation) => confirmation,
        Err(error) => {
            remove_prelaunch_files(&cleanup_paths);
            return Err(error);
        }
    };
    cleanup_paths.push(confirmation.path.clone());
    let command = confirmation.wrap_command(&launch.display_command());
    Ok(PreparedProviderLaunch {
        command,
        cleanup_paths,
        receipt: PreparedProviderReceipt::Grok {
            provider,
            launch,
            confirmation,
            confirmed: false,
        },
    })
}

struct GrokLaunchConfirmation {
    path: PathBuf,
    nonce: String,
}

impl GrokLaunchConfirmation {
    fn new() -> anyhow::Result<Self> {
        let dir = std::env::temp_dir().join(GROK_PROMPT_DIR);
        prepare_private_dir(&dir)
            .context("failed to prepare private Grok confirmation directory")?;
        let nonce = Uuid::new_v4().to_string();
        Ok(Self {
            path: dir.join(format!("{nonce}.started")),
            nonce,
        })
    }

    fn wrap_command(&self, command: &str) -> String {
        let path = crate::launcher::shell_single_quote(&self.path.to_string_lossy());
        let nonce = crate::launcher::shell_single_quote(&self.nonce);
        format!(
            "({command}) </dev/tty & provider_pid=$!; \
             (sleep 0.15; if kill -0 \"$provider_pid\" 2>/dev/null; then \
             umask 077; printf %s {nonce} > {path}; fi) & confirmation_pid=$!; \
             wait \"$provider_pid\"; provider_status=$?; \
             wait \"$confirmation_pid\" 2>/dev/null; test \"$provider_status\" -eq 0"
        )
    }

    async fn wait(&self) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + GROK_START_CONFIRM_TIMEOUT;
        loop {
            match tokio::fs::read_to_string(&self.path).await {
                Ok(value) if value == self.nonce => {
                    let _ = tokio::fs::remove_file(&self.path).await;
                    return Ok(());
                }
                Ok(_) => anyhow::bail!("Grok launch confirmation token mismatch"),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("failed to read Grok launch confirmation");
                }
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("Grok process did not remain running long enough to confirm launch");
            }
            tokio::time::sleep(GROK_START_CONFIRM_POLL).await;
        }
    }
}

fn absolute_cwd(cwd: &str) -> anyhow::Result<PathBuf> {
    let cwd = Path::new(cwd);
    if cwd.is_absolute() {
        return Ok(cwd.to_path_buf());
    }
    std::env::current_dir()
        .context("failed to resolve current directory for provider launch")
        .map(|current| current.join(cwd))
}

fn write_provider_prompt(initial_request: &str) -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join(GROK_PROMPT_DIR);
    prepare_private_dir(&dir).context("failed to prepare private provider prompt directory")?;
    let path = dir.join(format!("{}.txt", Uuid::new_v4()));
    let mut file =
        create_private_file(&path).context("failed to create private provider prompt")?;
    let write_result = (|| -> anyhow::Result<()> {
        file.write_all(initial_request.as_bytes())
            .context("failed to write private provider prompt")?;
        file.sync_all()
            .context("failed to sync private provider prompt")?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

fn remove_prelaunch_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn unknown_launch_receipt(
    launch: LaunchReceipt,
    tool: Option<SpawnTool>,
) -> AuthorizedProviderResumeLaunchReceipt {
    let (provider, capture_source) = match tool {
        Some(SpawnTool::Codex) => (
            ProviderResumeProvider::Codex,
            ProviderResumeCaptureSource::Unknown,
        ),
        Some(SpawnTool::Grok) => (
            ProviderResumeProvider::Grok,
            ProviderResumeCaptureSource::Unknown,
        ),
        Some(SpawnTool::Claude) => (
            ProviderResumeProvider::Claude,
            ProviderResumeCaptureSource::Unsupported,
        ),
        None => (
            ProviderResumeProvider::Unknown,
            ProviderResumeCaptureSource::Unknown,
        ),
    };
    AuthorizedProviderResumeLaunchReceipt::new(
        launch,
        AuthorizedProviderResumeReceipt::unknown(provider, capture_source),
    )
}

#[derive(Clone, Debug)]
pub(super) struct ProviderReceiptStore {
    dir: PathBuf,
}

impl ProviderReceiptStore {
    #[cfg(not(test))]
    pub(super) fn for_default_data_dir() -> Self {
        Self::new(crate::startup::resolve_data_dir())
    }

    pub(super) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: data_dir.into().join(RECEIPT_DIR),
        }
    }

    pub(super) async fn persist(
        &self,
        receipt: &AuthorizedProviderResumeLaunchReceipt,
    ) -> anyhow::Result<()> {
        let dir = self.dir.clone();
        let receipt = receipt.clone();
        tokio::task::spawn_blocking(move || persist_receipt_blocking(&dir, &receipt))
            .await
            .context("provider receipt persistence task failed")?
    }

    pub(super) async fn load(&self) -> anyhow::Result<Vec<AuthorizedProviderResumeLaunchReceipt>> {
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || load_receipts_blocking(&dir))
            .await
            .context("provider receipt load task failed")?
    }
}

fn persist_receipt_blocking(
    dir: &Path,
    receipt: &AuthorizedProviderResumeLaunchReceipt,
) -> anyhow::Result<()> {
    let session_id = receipt
        .launch()
        .session_id
        .as_deref()
        .context("provider receipt requires Swimmers session_id")?;
    prepare_private_dir(dir).context("failed to prepare private provider receipt directory")?;
    let stem = hex_name(session_id);
    let destination = dir.join(format!("{stem}.json"));
    let temporary = dir.join(format!(".{stem}.{}.tmp", Uuid::new_v4()));
    let encoded =
        serde_json::to_vec_pretty(receipt).context("failed to encode provider receipt")?;

    let write_result = (|| -> anyhow::Result<()> {
        let mut file = create_private_file(&temporary)
            .context("failed to create provider receipt temp file")?;
        file.write_all(&encoded)
            .context("failed to write provider receipt temp file")?;
        file.write_all(b"\n")
            .context("failed to terminate provider receipt record")?;
        file.sync_all()
            .context("failed to sync provider receipt temp file")?;
        fs::rename(&temporary, &destination)
            .context("failed to atomically install provider receipt")?;
        if let Err(error) = fs::File::open(dir).and_then(|directory| directory.sync_all()) {
            let removal = fs::remove_file(&destination);
            let _ = fs::File::open(dir).and_then(|directory| directory.sync_all());
            removal.context(
                "provider receipt directory sync failed and installed receipt could not be removed",
            )?;
            return Err(error).context("failed to sync provider receipt directory");
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn load_receipts_blocking(
    dir: &Path,
) -> anyhow::Result<Vec<AuthorizedProviderResumeLaunchReceipt>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("failed to read provider receipt directory"),
    };
    let mut receipts = Vec::new();
    for entry in entries {
        let entry = entry.context("failed to read provider receipt entry")?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if !entry
            .file_type()
            .context("failed to inspect provider receipt entry")?
            .is_file()
        {
            continue;
        }
        let encoded =
            fs::read(entry.path()).context("failed to read durable provider receipt record")?;
        let receipt = serde_json::from_slice(&encoded)
            .context("failed to decode durable provider receipt record")?;
        receipts.push(receipt);
    }
    Ok(receipts)
}

fn hex_name(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn cass_provider_identity_from_resume(
    resume: &AuthorizedProviderResumeReceipt,
    swimmers_session_id: &str,
    batch_id: &str,
    batch_index: u64,
    origin: CassOrigin,
) -> Result<CassProviderIdentity, CassAdmissionError> {
    let provider_session_id = resume.conversation_id().ok_or_else(|| {
        CassAdmissionError::new(
            "malformed_uuid",
            "provider session UUID is missing after allocation",
        )
    })?;
    let provider = match resume.provider() {
        ProviderResumeProvider::Codex => "codex",
        ProviderResumeProvider::Grok => "grok",
        ProviderResumeProvider::Claude => "claude",
        ProviderResumeProvider::Unknown => "swimmers",
    };
    let identity = CassProviderIdentity {
        schema_version: crate::types::CASS_PROVIDER_IDENTITY_SCHEMA.to_string(),
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        swimmers_session_id: swimmers_session_id.to_string(),
        batch_id: batch_id.to_string(),
        batch_index,
        origin,
    };
    let value = serde_json::to_value(&identity).map_err(|error| {
        CassAdmissionError::new(
            "document_invalid",
            format!("failed to emit cass_provider_identity/v1: {error}"),
        )
    })?;
    let parsed = CassProviderIdentity::from_value(&value)?;
    remember_cass_provider_identity(&parsed);
    Ok(parsed)
}

pub(crate) async fn refine_cass_provider_identity(
    reservation: &CassAdmissionReservationRef,
    identity: CassProviderIdentity,
) -> Result<CassAdmissionSubject, CassAdmissionError> {
    let expected_reservation_id = reservation.reservation_id.clone();
    let expected_provider_session_id = identity.provider_session_id.clone();
    let response = run_cass_admission_command(CassAdmissionCommandRequest::refine(
        expected_reservation_id.clone(),
        identity,
    ))
    .await?;
    if !response.ok {
        let error = response
            .error
            .unwrap_or(crate::types::CassAdmissionCommandError {
                code: "refinement_failure".to_string(),
                message: "refine-provider-identity refused".to_string(),
            });
        return Err(CassAdmissionError::new(error.code, error.message));
    }
    if response.reservation_id.as_deref() != Some(expected_reservation_id.as_str())
        || response.state.as_deref() != Some("refined")
    {
        return Err(CassAdmissionError::new(
            "partial_result",
            "refine-provider-identity returned an unbound result",
        ));
    }
    let subject = response.subject.ok_or_else(|| {
        CassAdmissionError::new(
            "partial_result",
            "refine-provider-identity returned no subject",
        )
    })?;
    let subject =
        CassAdmissionSubject::from_value(&serde_json::to_value(subject).map_err(|_| {
            CassAdmissionError::new(
                "document_invalid",
                "refine-provider-identity returned an invalid subject",
            )
        })?)?;
    if subject.producer_session_id.as_deref() != Some(expected_provider_session_id.as_str()) {
        return Err(CassAdmissionError::new(
            "reservation_mismatch",
            "refined subject is not bound to the allocated provider session",
        ));
    }
    Ok(subject)
}

pub(super) async fn run_cass_admission_command(
    request: CassAdmissionCommandRequest,
) -> Result<CassAdmissionCommandResponse, CassAdmissionError> {
    remember_cass_command_op(request.op);
    let argv = cass_admission_command_argv()?;
    let stdin = serde_json::to_vec(&request).map_err(|error| {
        CassAdmissionError::new(
            "document_invalid",
            format!("failed to encode admission command request: {error}"),
        )
    })?;
    let output = tokio::time::timeout(cass_admission_timeout(), async {
        let mut child = tokio::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                CassAdmissionError::new(
                    "admission_command_unavailable",
                    redact_cass_command_text(&error.to_string()),
                )
            })?;
        if let Some(mut stdin_handle) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin_handle.write_all(&stdin).await.map_err(|error| {
                CassAdmissionError::new(
                    "transport_loss",
                    redact_cass_command_text(&error.to_string()),
                )
            })?;
        }
        child.wait_with_output().await.map_err(|error| {
            CassAdmissionError::new(
                "transport_loss",
                redact_cass_command_text(&error.to_string()),
            )
        })
    })
    .await
    .map_err(|_| CassAdmissionError::new("transport_loss", "admission command timed out"))??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = redact_cass_command_text(&String::from_utf8_lossy(&output.stderr));
    if stdout.trim().is_empty() {
        return Err(CassAdmissionError::new(
            "partial_result",
            if stderr.is_empty() {
                "admission command returned no JSON".to_string()
            } else {
                format!("admission command returned no JSON: {stderr}")
            },
        ));
    }
    let response: CassAdmissionCommandResponse =
        serde_json::from_str(stdout.trim()).map_err(|error| {
            CassAdmissionError::new(
                "partial_result",
                format!(
                    "admission command returned unreadable JSON: {}",
                    redact_cass_command_text(&error.to_string())
                ),
            )
        })?;
    if response.schema_version != CASS_ADMISSION_COMMAND_SCHEMA {
        return Err(CassAdmissionError::new(
            "unknown_version",
            "admission command schema_version is unknown",
        ));
    }
    if response.ok == response.error.is_some() {
        return Err(CassAdmissionError::new(
            "partial_result",
            "admission command returned an inconsistent success/error result",
        ));
    }
    if let Some(error) = response.error.as_ref() {
        if error.code.trim().is_empty() || error.message.trim().is_empty() {
            return Err(CassAdmissionError::new(
                "partial_result",
                "admission command returned an incomplete structured error",
            ));
        }
    }
    if !output.status.success() && response.ok {
        return Err(CassAdmissionError::new(
            "partial_result",
            format!("admission command exited {} with ok=true", output.status),
        ));
    }
    Ok(response)
}

pub(super) fn cass_admission_command_configured() -> bool {
    std::env::var(CASS_ADMISSION_CMD_ENV)
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn cass_admission_command_argv() -> Result<Vec<String>, CassAdmissionError> {
    let raw = std::env::var(CASS_ADMISSION_CMD_ENV).map_err(|_| {
        CassAdmissionError::new(
            "admission_command_unavailable",
            "SKILLBOX_CASS_ADMISSION_CMD_JSON is unset",
        )
    })?;
    let argv: Vec<String> = serde_json::from_str(&raw).map_err(|_| {
        CassAdmissionError::new(
            "admission_command_unavailable",
            "SKILLBOX_CASS_ADMISSION_CMD_JSON must be a JSON argv array",
        )
    })?;
    if argv.is_empty() || argv[0].trim().is_empty() {
        return Err(CassAdmissionError::new(
            "admission_command_unavailable",
            "SKILLBOX_CASS_ADMISSION_CMD_JSON argv is empty",
        ));
    }
    Ok(argv)
}

fn cass_admission_timeout() -> std::time::Duration {
    std::env::var(CASS_ADMISSION_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(DEFAULT_CASS_ADMISSION_TIMEOUT)
}

fn redact_cass_command_text(text: &str) -> String {
    let mut redacted = text.to_string();
    for key in [
        "AUTH_TOKEN",
        "OBSERVER_TOKEN",
        "SKILLBOX_CASS_ADMISSION_CMD_JSON",
    ] {
        if let Ok(secret) = std::env::var(key) {
            if !secret.is_empty() {
                redacted = redacted.replace(&secret, "[redacted]");
            }
        }
    }
    for needle in ["api_token", "transcript", "publisher_root", "cursor"] {
        if redacted.contains(needle) {
            redacted = redacted.replace(needle, "[redacted]");
        }
    }
    redacted
}

fn record_codex_app_server_launch() {
    #[cfg(test)]
    CODEX_APP_SERVER_LAUNCH_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

fn cass_test_provider_launch(
    cwd: &Path,
    initial_request: &str,
) -> anyhow::Result<Option<PreparedProviderLaunch>> {
    #[cfg(test)]
    {
        let Ok(thread_id) = std::env::var("SWIMMERS_CASS_TEST_PROVIDER_UUID") else {
            return Ok(None);
        };
        if thread_id.trim().is_empty() {
            return Ok(None);
        }
        let _ = (cwd, initial_request);
        let provider_resume = AuthorizedProviderResumeReceipt::resumable(
            ProviderResumeProvider::Codex,
            thread_id.clone(),
            vec!["codex".to_string(), "resume".to_string(), thread_id.clone()],
            format!("codex resume {thread_id}"),
            ProviderResumeCaptureSource::ProviderResponse,
        )
        .context("Cass test provider UUID is not a valid resume identity")?;
        Ok(Some(PreparedProviderLaunch {
            command: format!("codex resume {thread_id}"),
            cleanup_paths: Vec::new(),
            receipt: PreparedProviderReceipt::Captured(provider_resume),
        }))
    }
    #[cfg(not(test))]
    {
        let _ = (cwd, initial_request);
        Ok(None)
    }
}

fn remember_cass_provider_identity(identity: &CassProviderIdentity) {
    #[cfg(test)]
    {
        if let Ok(mut slot) = LAST_CASS_PROVIDER_IDENTITY.lock() {
            *slot = Some(identity.clone());
        }
    }
    let _ = identity;
}

fn remember_cass_command_op(op: CassAdmissionCommandOp) {
    #[cfg(test)]
    {
        if let Ok(mut ops) = LAST_CASS_COMMAND_OPS.lock() {
            ops.push(format!("{op:?}").to_ascii_lowercase());
        }
    }
    let _ = op;
}

#[cfg(test)]
pub(crate) fn take_codex_app_server_launch_calls() -> usize {
    CODEX_APP_SERVER_LAUNCH_CALLS.swap(0, std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn reset_cass_admission_test_hooks() {
    take_codex_app_server_launch_calls();
    if let Ok(mut slot) = LAST_CASS_PROVIDER_IDENTITY.lock() {
        *slot = None;
    }
    if let Ok(mut ops) = LAST_CASS_COMMAND_OPS.lock() {
        ops.clear();
    }
}

#[cfg(test)]
pub(crate) fn take_last_cass_provider_identity() -> Option<CassProviderIdentity> {
    LAST_CASS_PROVIDER_IDENTITY
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

#[cfg(test)]
pub(crate) fn take_cass_command_ops() -> Vec<String> {
    LAST_CASS_COMMAND_OPS
        .lock()
        .map(|mut ops| std::mem::take(&mut *ops))
        .unwrap_or_default()
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::types::{ProviderResumeCapability, PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION};
    use tempfile::tempdir;

    fn resumable_receipt(
        provider: ProviderResumeProvider,
        session_id: &str,
        conversation_id: &str,
    ) -> AuthorizedProviderResumeLaunchReceipt {
        let command = match provider {
            ProviderResumeProvider::Grok => "grok",
            _ => "codex",
        };
        AuthorizedProviderResumeLaunchReceipt::new(
            LaunchReceipt::local("/workspace/swimmers", session_id, false),
            AuthorizedProviderResumeReceipt::resumable(
                provider,
                conversation_id,
                vec![
                    command.to_string(),
                    "resume".to_string(),
                    conversation_id.to_string(),
                ],
                format!("{command} resume {conversation_id}"),
                ProviderResumeCaptureSource::ProviderResponse,
            )
            .expect("valid receipt"),
        )
    }

    #[test]
    fn provider_resume_integration_unknown_matrix_is_explicit_and_identity_free() {
        for (tool, provider) in [
            (Some(SpawnTool::Codex), ProviderResumeProvider::Codex),
            (Some(SpawnTool::Grok), ProviderResumeProvider::Grok),
            (Some(SpawnTool::Claude), ProviderResumeProvider::Claude),
            (None, ProviderResumeProvider::Unknown),
        ] {
            let receipt = unknown_launch_receipt(
                LaunchReceipt::local("/workspace/swimmers", "session-1", false),
                tool,
            );
            assert_eq!(receipt.version(), PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION);
            assert_eq!(receipt.provider_resume().provider(), provider);
            assert_eq!(
                receipt.provider_resume().capability(),
                ProviderResumeCapability::Unknown
            );
            assert_eq!(receipt.provider_resume().conversation_id(), None);
        }
    }

    #[test]
    fn provider_resume_integration_legacy_reader_accepts_versioned_receipt() {
        let receipt = resumable_receipt(ProviderResumeProvider::Codex, "session-7", "thread-exact");
        let encoded = serde_json::to_value(receipt).expect("encode");
        let legacy: LaunchReceipt = serde_json::from_value(encoded).expect("legacy decode");
        assert_eq!(legacy.session_id.as_deref(), Some("session-7"));
    }

    #[tokio::test]
    async fn provider_resume_integration_durable_store_round_trips_exact_correlation() {
        let dir = tempdir().expect("tempdir");
        let store = ProviderReceiptStore::new(dir.path());
        let receipt =
            resumable_receipt(ProviderResumeProvider::Codex, "session-9", "thread-durable");
        store.persist(&receipt).await.expect("persist");
        let loaded = store.load().await.expect("load");
        assert_eq!(loaded, vec![receipt]);
    }

    #[tokio::test]
    async fn provider_resume_integration_partial_persistence_fails_closed() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(RECEIPT_DIR), b"not a directory").expect("block receipt dir");
        let store = ProviderReceiptStore::new(dir.path());
        let receipt = resumable_receipt(ProviderResumeProvider::Grok, "session-10", "grok-durable");
        assert!(store.persist(&receipt).await.is_err());
        assert!(store.load().await.is_err());
    }

    #[tokio::test]
    async fn provider_resume_integration_parallel_store_keeps_exact_pairs() {
        let dir = tempdir().expect("tempdir");
        let store = ProviderReceiptStore::new(dir.path());
        let receipts: Vec<_> = (0..32)
            .map(|index| {
                resumable_receipt(
                    ProviderResumeProvider::Codex,
                    &format!("session-{index}"),
                    &format!("thread-{index}"),
                )
            })
            .collect();
        let writes = receipts.iter().cloned().map(|receipt| {
            let store = store.clone();
            async move { store.persist(&receipt).await }
        });
        for result in futures::future::join_all(writes).await {
            result.expect("parallel persist");
        }
        let loaded = store.load().await.expect("load");
        assert_eq!(loaded.len(), receipts.len());
        for receipt in loaded {
            let session_id = receipt.launch().session_id.as_deref().expect("session id");
            let suffix = session_id.strip_prefix("session-").expect("session prefix");
            assert_eq!(
                receipt.provider_resume().conversation_id(),
                Some(format!("thread-{suffix}").as_str())
            );
        }
    }
}
