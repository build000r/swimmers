use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::future::join_all;
use reqwest::Client;

use crate::config::Config;
use crate::session::overlay::default_overlay;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, CreateSessionsBatchRequest,
    CreateSessionsBatchResponse, ErrorResponse, LaunchPathMapping, LaunchTargetSummary,
    SessionAgentContextResponse, SessionGitDiffResponse, SessionListResponse, SessionSummary,
};

const REMOTE_LIST_TIMEOUT: Duration = Duration::from_millis(900);
const REMOTE_CREATE_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const REMOTE_SESSION_SEPARATOR: &str = "::";
const REMOTE_POLL_FAILURE_BACKOFF_MS: u64 = 10_000;

static REMOTE_POLL_BACKOFF_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct RemoteSessionError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl RemoteSessionError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code.to_string(),
                message: Some(self.message),
            }),
        )
            .into_response()
    }
}

pub fn is_remote_launch_target(target: Option<&str>) -> bool {
    target
        .map(str::trim)
        .is_some_and(|target| !target.is_empty() && target != "local")
}

pub fn split_remote_session_id(session_id: &str) -> Option<(&str, &str)> {
    let (target_id, remote_session_id) = session_id.split_once(REMOTE_SESSION_SEPARATOR)?;
    (!target_id.is_empty() && !remote_session_id.is_empty())
        .then_some((target_id, remote_session_id))
}

pub fn namespace_session_id(target_id: &str, remote_session_id: &str) -> String {
    format!("{target_id}{REMOTE_SESSION_SEPARATOR}{remote_session_id}")
}

pub fn namespace_session_summary(
    target: &LaunchTargetSummary,
    mut session: SessionSummary,
) -> SessionSummary {
    if split_remote_session_id(&session.session_id).is_none() {
        session.session_id = namespace_session_id(&target.id, &session.session_id);
    }
    if !session.tmux_name.starts_with('[') {
        session.tmux_name = format!("[{}] {}", target.label, session.tmux_name);
    }
    session
}

pub fn denamespace_for_target(
    session_id: &str,
) -> Result<Option<(LaunchTargetSummary, &str)>, RemoteSessionError> {
    let Some((target_id, remote_session_id)) = split_remote_session_id(session_id) else {
        return Ok(None);
    };
    let target = resolve_launch_target_by_id(target_id)?;
    Ok(Some((target, remote_session_id)))
}

pub async fn list_remote_sessions() -> Vec<SessionSummary> {
    #[cfg(test)]
    if std::env::var_os("SWIMMERS_TEST_ENABLE_REMOTE_POLLING").is_none() {
        return Vec::new();
    }

    if remote_poll_backoff_active() {
        return Vec::new();
    }

    let Some(overlay) = default_overlay() else {
        return Vec::new();
    };
    let targets = overlay
        .all_launch_targets()
        .into_iter()
        .filter(is_swimmers_api_target)
        .filter(|target| {
            if target_points_at_current_server(target, &Config::from_env()) {
                tracing::debug!(
                    target = %target.id,
                    base_url = ?target.base_url,
                    "skipping self-target remote session polling"
                );
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Vec::new();
    }

    let client = match http_client(REMOTE_LIST_TIMEOUT) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err.message(), "remote session aggregation disabled");
            return Vec::new();
        }
    };

    let results = join_all(
        targets
            .into_iter()
            .filter_map(|target| {
                remote_auth_token_for_polling(&target).map(|token| (target, token))
            })
            .map(|(target, token)| {
                let client = client.clone();
                async move { list_remote_sessions_for_target(&client, target, token).await }
            }),
    )
    .await;

    results
        .into_iter()
        .flat_map(|sessions| match sessions {
            Ok(sessions) => sessions,
            Err(err) => {
                tracing::warn!(error = %err.message(), "remote session list failed");
                record_remote_poll_failure();
                Vec::new()
            }
        })
        .collect()
}

fn remote_poll_backoff_active() -> bool {
    now_ms() < REMOTE_POLL_BACKOFF_UNTIL_MS.load(Ordering::Acquire)
}

fn record_remote_poll_failure() {
    REMOTE_POLL_BACKOFF_UNTIL_MS.store(
        now_ms().saturating_add(REMOTE_POLL_FAILURE_BACKOFF_MS),
        Ordering::Release,
    );
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn list_remote_sessions_for_target(
    client: &Client,
    target: LaunchTargetSummary,
    auth_token: Option<String>,
) -> Result<Vec<SessionSummary>, RemoteSessionError> {
    let url = remote_url(&target, "/v1/sessions")?;
    let mut request = client.get(url);
    if let Some(token) = auth_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|err| {
        RemoteSessionError::new(
            StatusCode::BAD_GATEWAY,
            "REMOTE_SESSION_LIST_FAILED",
            format!("failed to list sessions from '{}': {err}", target.id),
        )
    })?;

    if !response.status().is_success() {
        return Err(remote_response_error(
            response,
            "REMOTE_SESSION_LIST_FAILED",
            format!("remote target '{}' rejected session listing", target.id),
        )
        .await);
    }

    let body = response
        .json::<SessionListResponse>()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                "REMOTE_SESSION_LIST_FAILED",
                format!("failed to parse session list from '{}': {err}", target.id),
            )
        })?;
    Ok(body
        .sessions
        .into_iter()
        .map(|session| namespace_session_summary(&target, session))
        .collect())
}

pub async fn create_remote_session(
    body: CreateSessionRequest,
) -> Result<CreateSessionResponse, RemoteSessionError> {
    let target_id = required_target_id(body.launch_target.as_deref())?;
    let local_cwd = launch_cwd(body.cwd.as_deref())?;
    let target = resolve_launch_target_for_cwd(&local_cwd, target_id)?;
    let remote_cwd = map_cwd_for_target(&target, &local_cwd)?;

    create_remote_session_on_target(
        &target,
        CreateSessionRequest {
            name: body.name,
            cwd: Some(remote_cwd),
            spawn_tool: body.spawn_tool,
            launch_target: None,
            initial_request: body.initial_request,
        },
    )
    .await
}

pub async fn create_remote_session_on_target(
    target: &LaunchTargetSummary,
    mut body: CreateSessionRequest,
) -> Result<CreateSessionResponse, RemoteSessionError> {
    ensure_swimmers_api_target(target)?;
    body.launch_target = None;
    let client = http_client(REMOTE_CREATE_TIMEOUT)?;
    let url = remote_url(target, "/v1/sessions")?;
    let response = with_remote_auth(client.post(url), target)?
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                "REMOTE_LAUNCH_FAILED",
                format!("failed to create session on '{}': {err}", target.id),
            )
        })?;

    if !response.status().is_success() {
        return Err(remote_response_error(
            response,
            "REMOTE_LAUNCH_FAILED",
            format!("remote target '{}' rejected session creation", target.id),
        )
        .await);
    }

    let mut body = response
        .json::<CreateSessionResponse>()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                "REMOTE_LAUNCH_FAILED",
                format!(
                    "failed to parse create response from '{}': {err}",
                    target.id
                ),
            )
        })?;
    body.session = namespace_session_summary(target, body.session);
    Ok(body)
}

pub async fn create_remote_sessions_batch(
    body: CreateSessionsBatchRequest,
) -> Result<CreateSessionsBatchResponse, RemoteSessionError> {
    let target_id = required_target_id(body.launch_target.as_deref())?;
    let first_cwd = body
        .dirs
        .first()
        .ok_or_else(|| {
            RemoteSessionError::new(
                StatusCode::BAD_REQUEST,
                "VALIDATION_FAILED",
                "dirs must not be empty",
            )
        })?
        .clone();
    let target = resolve_launch_target_for_cwd(&first_cwd, target_id)?;
    let original_dirs = body.dirs;
    let remote_dirs = original_dirs
        .iter()
        .map(|cwd| map_cwd_for_target(&target, cwd))
        .collect::<Result<Vec<_>, _>>()?;

    let mut response = create_remote_sessions_batch_on_target(
        &target,
        CreateSessionsBatchRequest {
            dirs: remote_dirs,
            spawn_tool: body.spawn_tool,
            launch_target: None,
            initial_request: body.initial_request,
        },
    )
    .await?;

    for result in &mut response.results {
        if let Some(original) = original_dirs.get(result.index) {
            result.cwd = original.clone();
        }
    }
    Ok(response)
}

pub async fn create_remote_sessions_batch_on_target(
    target: &LaunchTargetSummary,
    mut body: CreateSessionsBatchRequest,
) -> Result<CreateSessionsBatchResponse, RemoteSessionError> {
    ensure_swimmers_api_target(target)?;
    body.launch_target = None;
    let client = http_client(REMOTE_CREATE_TIMEOUT)?;
    let url = remote_url(target, "/v1/sessions/batch")?;
    let response = with_remote_auth(client.post(url), target)?
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                "REMOTE_LAUNCH_FAILED",
                format!("failed to create sessions on '{}': {err}", target.id),
            )
        })?;

    if !response.status().is_success() {
        return Err(remote_response_error(
            response,
            "REMOTE_LAUNCH_FAILED",
            format!(
                "remote target '{}' rejected batch session creation",
                target.id
            ),
        )
        .await);
    }

    let mut body = response
        .json::<CreateSessionsBatchResponse>()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                "REMOTE_LAUNCH_FAILED",
                format!(
                    "failed to parse batch create response from '{}': {err}",
                    target.id
                ),
            )
        })?;
    for result in &mut body.results {
        if let Some(session) = result.session.take() {
            result.session = Some(namespace_session_summary(target, session));
        }
    }
    Ok(body)
}

pub async fn fetch_remote_mermaid_artifact(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Result<crate::types::MermaidArtifactResponse, RemoteSessionError> {
    get_remote_json(
        target,
        &format!("/v1/sessions/{remote_session_id}/mermaid-artifact"),
    )
    .await
    .map(|mut response: crate::types::MermaidArtifactResponse| {
        response.session_id = namespace_session_id(&target.id, &response.session_id);
        response
    })
}

pub async fn fetch_remote_plan_file(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
    name: &str,
) -> Result<crate::types::PlanFileResponse, RemoteSessionError> {
    let mut response: crate::types::PlanFileResponse = get_remote_json_with_query(
        target,
        &format!("/v1/sessions/{remote_session_id}/plan-file"),
        &[("name", name)],
    )
    .await?;
    response.session_id = namespace_session_id(&target.id, &response.session_id);
    Ok(response)
}

pub async fn fetch_remote_agent_context(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Result<SessionAgentContextResponse, RemoteSessionError> {
    get_remote_json(
        target,
        &format!("/v1/sessions/{remote_session_id}/agent-context"),
    )
    .await
    .map(|mut response: SessionAgentContextResponse| {
        response.session_id = namespace_session_id(&target.id, &response.session_id);
        response
    })
}

pub async fn fetch_remote_git_diff(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Result<SessionGitDiffResponse, RemoteSessionError> {
    get_remote_json(
        target,
        &format!("/v1/sessions/{remote_session_id}/git-diff"),
    )
    .await
    .map(|mut response: SessionGitDiffResponse| {
        response.session_id = namespace_session_id(&target.id, &response.session_id);
        response
    })
}

async fn get_remote_json<T>(
    target: &LaunchTargetSummary,
    path: &str,
) -> Result<T, RemoteSessionError>
where
    T: serde::de::DeserializeOwned,
{
    get_remote_json_with_query(target, path, &[]).await
}

async fn get_remote_json_with_query<T>(
    target: &LaunchTargetSummary,
    path: &str,
    query: &[(&str, &str)],
) -> Result<T, RemoteSessionError>
where
    T: serde::de::DeserializeOwned,
{
    ensure_swimmers_api_target(target)?;
    let client = http_client(REMOTE_CREATE_TIMEOUT)?;
    let url = remote_url(target, path)?;
    let response = with_remote_auth(client.get(url), target)?
        .query(query)
        .send()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                "REMOTE_SESSION_REQUEST_FAILED",
                format!("failed to query remote session on '{}': {err}", target.id),
            )
        })?;
    if !response.status().is_success() {
        return Err(remote_response_error(
            response,
            "REMOTE_SESSION_REQUEST_FAILED",
            format!("remote target '{}' rejected session request", target.id),
        )
        .await);
    }
    response.json::<T>().await.map_err(|err| {
        RemoteSessionError::new(
            StatusCode::BAD_GATEWAY,
            "REMOTE_SESSION_REQUEST_FAILED",
            format!(
                "failed to parse remote session response from '{}': {err}",
                target.id
            ),
        )
    })
}

fn required_target_id(target: Option<&str>) -> Result<&str, RemoteSessionError> {
    let target = target.map(str::trim).filter(|target| !target.is_empty());
    match target {
        Some("local") | None => Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            "remote launch requested without a non-local launch target",
        )),
        Some(target) => Ok(target),
    }
}

fn launch_cwd(cwd: Option<&str>) -> Result<String, RemoteSessionError> {
    if let Some(cwd) = cwd.map(str::trim).filter(|cwd| !cwd.is_empty()) {
        return Ok(cwd.to_string());
    }
    std::env::current_dir()
        .map(|cwd| cwd.to_string_lossy().into_owned())
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_REQUEST,
                "VALIDATION_FAILED",
                format!("cwd is required for remote launch and current dir is unavailable: {err}"),
            )
        })
}

fn resolve_launch_target_for_cwd(
    cwd: &str,
    target_id: &str,
) -> Result<LaunchTargetSummary, RemoteSessionError> {
    let Some(overlay) = default_overlay() else {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_UNKNOWN",
            "no skillbox-config overlay is available for remote launch targets",
        ));
    };
    let target = overlay
        .launch_target_for_cwd(cwd, target_id)
        .or_else(|| overlay.launch_target_by_id(target_id))
        .ok_or_else(|| {
            RemoteSessionError::new(
                StatusCode::BAD_REQUEST,
                "LAUNCH_TARGET_UNKNOWN",
                format!("launch target '{target_id}' is not configured"),
            )
        })?;
    ensure_swimmers_api_target(&target)?;
    Ok(target)
}

fn resolve_launch_target_by_id(target_id: &str) -> Result<LaunchTargetSummary, RemoteSessionError> {
    let Some(overlay) = default_overlay() else {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_UNKNOWN",
            "no skillbox-config overlay is available for remote session targets",
        ));
    };
    let target = overlay.launch_target_by_id(target_id).ok_or_else(|| {
        RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_UNKNOWN",
            format!("remote session target '{target_id}' is not configured"),
        )
    })?;
    ensure_swimmers_api_target(&target)?;
    Ok(target)
}

fn ensure_swimmers_api_target(target: &LaunchTargetSummary) -> Result<(), RemoteSessionError> {
    if !is_swimmers_api_target(target) {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_UNSUPPORTED",
            format!(
                "launch target '{}' has kind '{}' but only 'swimmers_api' remote targets are supported",
                target.id, target.kind
            ),
        ));
    }
    if target
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .is_none()
    {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            format!("launch target '{}' is missing base_url", target.id),
        ));
    }
    Ok(())
}

fn is_swimmers_api_target(target: &LaunchTargetSummary) -> bool {
    target.kind == "swimmers_api"
}

fn target_points_at_current_server(target: &LaunchTargetSummary, config: &Config) -> bool {
    let Some(base_url) = target
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let url_port = url.port_or_known_default().unwrap_or(80);
    if url_port != config.port {
        return false;
    }
    let bind_host = crate::cli::bind_host(&config.bind);
    host.eq_ignore_ascii_case(bind_host)
        || (crate::cli::is_loopback_bind(&config.bind)
            && matches!(host, "127.0.0.1" | "localhost" | "::1"))
}

pub fn map_cwd_for_target(
    target: &LaunchTargetSummary,
    cwd: &str,
) -> Result<String, RemoteSessionError> {
    map_path_with_mappings(cwd, &target.path_mappings).ok_or_else(|| {
        RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_PATH_UNMAPPED",
            format!(
                "cwd '{cwd}' is not covered by path_mappings for launch target '{}'",
                target.id
            ),
        )
    })
}

pub fn map_path_with_mappings(cwd: &str, mappings: &[LaunchPathMapping]) -> Option<String> {
    let cwd = lexical_path(cwd);
    mappings
        .iter()
        .filter_map(|mapping| {
            let local_prefix = lexical_path(&mapping.local_prefix);
            let rel = cwd.strip_prefix(&local_prefix).ok()?;
            let score = local_prefix.components().count();
            let mut remote = PathBuf::from(&mapping.remote_prefix);
            remote.push(rel);
            Some((score, remote.to_string_lossy().into_owned()))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, path)| path)
}

fn lexical_path(path: &str) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn http_client(timeout: Duration) -> Result<Client, RemoteSessionError> {
    Client::builder()
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "REMOTE_HTTP_CLIENT_FAILED",
                format!("failed to build remote HTTP client: {err}"),
            )
        })
}

fn remote_url(target: &LaunchTargetSummary, path: &str) -> Result<String, RemoteSessionError> {
    let base_url = target
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            RemoteSessionError::new(
                StatusCode::BAD_REQUEST,
                "LAUNCH_TARGET_INVALID",
                format!("launch target '{}' is missing base_url", target.id),
            )
        })?;
    Ok(format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
}

fn with_remote_auth(
    builder: reqwest::RequestBuilder,
    target: &LaunchTargetSummary,
) -> Result<reqwest::RequestBuilder, RemoteSessionError> {
    match remote_auth_token(target)? {
        Some(token) => Ok(builder.bearer_auth(token)),
        None => Ok(builder),
    }
}

fn remote_auth_token(target: &LaunchTargetSummary) -> Result<Option<String>, RemoteSessionError> {
    let Some(env_key) = target
        .auth_token_env
        .as_deref()
        .map(str::trim)
        .filter(|env_key| !env_key.is_empty())
    else {
        return Ok(None);
    };
    let token = std::env::var(env_key).map_err(|_| {
        RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_AUTH_TOKEN_MISSING",
            format!(
                "launch target '{}' requires auth token env {env_key}, but it is not set",
                target.id
            ),
        )
    })?;
    if token.trim().is_empty() {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_AUTH_TOKEN_MISSING",
            format!(
                "launch target '{}' requires auth token env {env_key}, but it is empty",
                target.id
            ),
        ));
    }
    Ok(Some(token))
}

fn remote_auth_token_for_polling(target: &LaunchTargetSummary) -> Option<Option<String>> {
    match remote_auth_token(target) {
        Ok(token) => Some(token),
        Err(err) => {
            tracing::debug!(
                target = %target.id,
                error = %err.message(),
                "skipping remote session polling"
            );
            None
        }
    }
}

async fn remote_response_error(
    response: reqwest::Response,
    code: &'static str,
    fallback: String,
) -> RemoteSessionError {
    let status = response.status();
    let message = match response.json::<ErrorResponse>().await {
        Ok(body) => body.message.unwrap_or(fallback),
        Err(_) => fallback,
    };
    RemoteSessionError::new(
        StatusCode::BAD_GATEWAY,
        code,
        format!("{message} (remote status {status})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        SessionBatchMembership, SessionState, SpawnTool, ThoughtSource, ThoughtState,
        TransportHealth,
    };
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use axum::{Json as AxumJson, Router};
    use chrono::Utc;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn target() -> LaunchTargetSummary {
        LaunchTargetSummary {
            id: "jeremy-skillbox".to_string(),
            label: "Jeremy Skillbox".to_string(),
            kind: "swimmers_api".to_string(),
            base_url: Some("http://127.0.0.1:3210".to_string()),
            auth_token_env: None,
            path_mappings: vec![
                LaunchPathMapping {
                    local_prefix: "/workspace/repos".to_string(),
                    remote_prefix: "/monoserver".to_string(),
                },
                LaunchPathMapping {
                    local_prefix: "/workspace/repos/opensource".to_string(),
                    remote_prefix: "/monoserver/opensource".to_string(),
                },
            ],
        }
    }

    fn summary(session_id: &str) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Idle,
            current_command: None,
            state_evidence: Default::default(),
            cwd: "/monoserver/opensource/swimmers".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: crate::types::fallback_rest_state(
                SessionState::Idle,
                ThoughtState::Holding,
            ),
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
            batch: None::<SessionBatchMembership>,
        }
    }

    #[derive(Clone, Default)]
    struct CaptureState {
        requests: Arc<Mutex<CapturedRequests>>,
    }

    type CapturedRequests = Vec<(Option<String>, CreateSessionRequest)>;

    async fn capture_create_session(
        axum::extract::State(state): axum::extract::State<CaptureState>,
        headers: HeaderMap,
        AxumJson(body): AxumJson<CreateSessionRequest>,
    ) -> (StatusCode, AxumJson<CreateSessionResponse>) {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        state.requests.lock().await.push((auth, body));
        (
            StatusCode::CREATED,
            AxumJson(CreateSessionResponse {
                session: summary("sess_0"),
                repo_theme: None,
            }),
        )
    }

    async fn capture_list_sessions() -> AxumJson<SessionListResponse> {
        AxumJson(SessionListResponse {
            sessions: vec![summary("sess_1")],
            version: 0,
            repo_themes: Default::default(),
        })
    }

    async fn spawn_create_server() -> (String, tokio::task::JoinHandle<()>, CaptureState) {
        let state = CaptureState::default();
        let app = Router::new()
            .route("/v1/sessions", post(capture_create_session))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test api");
        });
        (format!("http://{addr}"), handle, state)
    }

    async fn spawn_list_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/v1/sessions", get(capture_list_sessions));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test api");
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn map_path_uses_longest_matching_prefix() {
        let mapped = map_path_with_mappings(
            "/workspace/repos/opensource/swimmers",
            &target().path_mappings,
        )
        .expect("mapped");
        assert_eq!(mapped, "/monoserver/opensource/swimmers");
    }

    #[test]
    fn map_path_respects_component_boundaries() {
        assert!(
            map_path_with_mappings("/workspace/repos2/swimmers", &target().path_mappings).is_none()
        );
    }

    #[test]
    fn namespaces_remote_session_summary() {
        let target = target();
        let session = namespace_session_summary(&target, summary("sess_0"));
        assert_eq!(
            session.session_id,
            namespace_session_id("jeremy-skillbox", "sess_0")
        );
        assert_eq!(session.tmux_name, "[Jeremy Skillbox] 7");
    }

    #[test]
    fn target_points_at_current_server_matches_active_tailnet_bind_and_port() {
        let mut config = Config::default();
        config.bind = "100.86.253.9".to_string();
        config.port = 3210;
        let mut target = target();
        target.base_url = Some("http://100.86.253.9:3210".to_string());

        assert!(target_points_at_current_server(&target, &config));

        target.base_url = Some("http://100.86.253.9:3211".to_string());
        assert!(!target_points_at_current_server(&target, &config));
    }

    #[test]
    fn target_points_at_current_server_matches_loopback_aliases() {
        let mut config = Config::default();
        config.bind = "127.0.0.1".to_string();
        config.port = 3210;
        let mut target = target();
        target.base_url = Some("http://localhost:3210".to_string());

        assert!(target_points_at_current_server(&target, &config));
    }

    #[tokio::test]
    async fn create_remote_session_posts_without_recursive_launch_target_and_namespaces_response() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("SWIMMERS_REMOTE_TEST_TOKEN", "secret-token");
        let (base_url, handle, state) = spawn_create_server().await;
        let mut target = target();
        target.base_url = Some(base_url);
        target.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());

        let response = create_remote_session_on_target(
            &target,
            CreateSessionRequest {
                name: None,
                cwd: Some("/monoserver/opensource/swimmers".to_string()),
                spawn_tool: Some(SpawnTool::Codex),
                launch_target: Some("jeremy-skillbox".to_string()),
                initial_request: Some("run tests".to_string()),
            },
        )
        .await
        .expect("remote create");

        assert_eq!(
            response.session.session_id,
            namespace_session_id("jeremy-skillbox", "sess_0")
        );
        let requests = state.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0.as_deref(), Some("Bearer secret-token"));
        assert_eq!(
            requests[0].1.cwd.as_deref(),
            Some("/monoserver/opensource/swimmers")
        );
        assert_eq!(requests[0].1.launch_target, None);
        assert_eq!(requests[0].1.initial_request.as_deref(), Some("run tests"));
        drop(requests);
        handle.abort();
        std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
    }

    #[tokio::test]
    async fn list_remote_sessions_for_target_namespaces_returned_sessions() {
        let (base_url, handle) = spawn_list_server().await;
        let mut target = target();
        target.base_url = Some(base_url);
        let client = http_client(REMOTE_LIST_TIMEOUT).expect("http client");

        let sessions = list_remote_sessions_for_target(&client, target, None)
            .await
            .expect("remote list");

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].session_id,
            namespace_session_id("jeremy-skillbox", "sess_1")
        );
        assert_eq!(sessions[0].tmux_name, "[Jeremy Skillbox] 7");
        handle.abort();
    }
}
