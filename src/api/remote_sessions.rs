use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use reqwest::Client;

use crate::api::envelope::error_body_msg;
use crate::config::Config;
use crate::session::overlay::default_overlay;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, CreateSessionsBatchRequest,
    CreateSessionsBatchResponse, ErrorResponse, LaunchPathMapping, LaunchTargetSummary,
    SessionAgentContextResponse, SessionGitDiffResponse, SessionListResponse, SessionSummary,
    SessionTimelineResponse, SessionTranscriptResponse,
};

const REMOTE_LIST_TIMEOUT: Duration = Duration::from_millis(900);
const REMOTE_CREATE_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const REMOTE_SESSION_SEPARATOR: &str = "::";
const REMOTE_POLL_FAILURE_BACKOFF_MS: u64 = 10_000;

#[derive(Clone, Debug)]
struct RemoteTargetSessionCache {
    sessions: Vec<SessionSummary>,
    last_seen_at: Option<DateTime<Utc>>,
    backoff_until_ms: u64,
}

static REMOTE_TARGET_SESSION_CACHE: OnceLock<Mutex<HashMap<String, RemoteTargetSessionCache>>> =
    OnceLock::new();

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
        (self.status, Json(error_body_msg(self.code, self.message))).into_response()
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

pub fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                encoded.push('%');
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 0x0F) as usize] as char);
            }
        }
    }
    encoded
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

    list_remote_sessions_for_targets(targets).await
}

async fn list_remote_sessions_for_targets(
    targets: Vec<LaunchTargetSummary>,
) -> Vec<SessionSummary> {
    let client = match http_client(REMOTE_LIST_TIMEOUT) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err.message(), "remote session aggregation disabled");
            return targets
                .into_iter()
                .flat_map(|target| record_remote_poll_failure(&target.id))
                .collect();
        }
    };

    let results = join_all(targets.into_iter().map(|target| {
        let client = client.clone();
        async move { list_remote_sessions_for_poll_target(&client, target).await }
    }))
    .await;

    results.into_iter().flatten().collect()
}

async fn list_remote_sessions_for_poll_target(
    client: &Client,
    target: LaunchTargetSummary,
) -> Vec<SessionSummary> {
    let target_id = target.id.clone();
    if remote_poll_backoff_active(&target_id) {
        return cached_stale_sessions_for_target(&target_id);
    }

    let auth_token = match remote_auth_token(&target) {
        Ok(token) => token,
        Err(err) => {
            tracing::debug!(
                target = %target.id,
                error = %err.message(),
                "skipping remote session polling"
            );
            return record_remote_poll_failure(&target_id);
        }
    };

    match list_remote_sessions_for_target(client, target, auth_token).await {
        Ok(sessions) => {
            record_remote_poll_success(&target_id, &sessions);
            sessions
        }
        Err(err) => {
            tracing::warn!(
                target = %target_id,
                error = %err.message(),
                "remote session list failed"
            );
            record_remote_poll_failure(&target_id)
        }
    }
}

fn remote_poll_backoff_active(target_id: &str) -> bool {
    let now = now_ms();
    with_remote_target_session_cache(|cache| {
        cache
            .get(target_id)
            .is_some_and(|entry| now < entry.backoff_until_ms)
    })
}

fn record_remote_poll_success(target_id: &str, sessions: &[SessionSummary]) {
    let entry = RemoteTargetSessionCache {
        sessions: sessions.to_vec(),
        last_seen_at: Some(Utc::now()),
        backoff_until_ms: 0,
    };
    with_remote_target_session_cache(|cache| {
        cache.insert(target_id.to_string(), entry);
    });
}

fn record_remote_poll_failure(target_id: &str) -> Vec<SessionSummary> {
    let backoff_until_ms = now_ms().saturating_add(REMOTE_POLL_FAILURE_BACKOFF_MS);
    with_remote_target_session_cache(|cache| {
        let entry =
            cache
                .entry(target_id.to_string())
                .or_insert_with(|| RemoteTargetSessionCache {
                    sessions: Vec::new(),
                    last_seen_at: None,
                    backoff_until_ms: 0,
                });
        entry.backoff_until_ms = backoff_until_ms;
        stale_sessions_from_cache(entry)
    })
}

fn cached_stale_sessions_for_target(target_id: &str) -> Vec<SessionSummary> {
    with_remote_target_session_cache(|cache| {
        cache
            .get(target_id)
            .map(stale_sessions_from_cache)
            .unwrap_or_default()
    })
}

fn stale_sessions_from_cache(entry: &RemoteTargetSessionCache) -> Vec<SessionSummary> {
    entry
        .sessions
        .iter()
        .cloned()
        .map(|session| session.into_remote_poll_degraded(entry.last_seen_at))
        .collect()
}

fn with_remote_target_session_cache<R>(
    f: impl FnOnce(&mut HashMap<String, RemoteTargetSessionCache>) -> R,
) -> R {
    let mut cache = REMOTE_TARGET_SESSION_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    f(&mut cache)
}

#[cfg(test)]
fn reset_remote_target_session_cache_for_tests() {
    with_remote_target_session_cache(|cache| cache.clear());
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
            &target,
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
            target,
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
            target,
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
    let session_id = encode_path_segment(remote_session_id);
    get_remote_json(
        target,
        &format!("/v1/sessions/{session_id}/mermaid-artifact"),
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
    let session_id = encode_path_segment(remote_session_id);
    let mut response: crate::types::PlanFileResponse = get_remote_json_with_query(
        target,
        &format!("/v1/sessions/{session_id}/plan-file"),
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
    let session_id = encode_path_segment(remote_session_id);
    get_remote_json(target, &format!("/v1/sessions/{session_id}/agent-context"))
        .await
        .map(|mut response: SessionAgentContextResponse| {
            response.session_id = namespace_session_id(&target.id, &response.session_id);
            response
        })
}

pub async fn fetch_remote_timeline(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Result<SessionTimelineResponse, RemoteSessionError> {
    let session_id = encode_path_segment(remote_session_id);
    get_remote_json(target, &format!("/v1/sessions/{session_id}/timeline"))
        .await
        .map(|mut response: SessionTimelineResponse| {
            response.session_id = namespace_session_id(&target.id, &response.session_id);
            response
        })
}

pub async fn fetch_remote_transcript(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
    turn_id: Option<&str>,
    after: Option<u64>,
    limit: Option<usize>,
) -> Result<SessionTranscriptResponse, RemoteSessionError> {
    let mut query = Vec::new();
    if let Some(turn_id) = turn_id.filter(|turn_id| !turn_id.trim().is_empty()) {
        query.push(("turn_id".to_string(), turn_id.to_string()));
    }
    if let Some(after) = after {
        query.push(("after".to_string(), after.to_string()));
    }
    if let Some(limit) = limit {
        query.push(("limit".to_string(), limit.to_string()));
    }
    let session_id = encode_path_segment(remote_session_id);
    let query_refs = query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let mut response: SessionTranscriptResponse = get_remote_json_with_query(
        target,
        &format!("/v1/sessions/{session_id}/transcript"),
        &query_refs,
    )
    .await?;
    response.session_id = namespace_session_id(&target.id, &response.session_id);
    Ok(response)
}

pub async fn fetch_remote_git_diff(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Result<SessionGitDiffResponse, RemoteSessionError> {
    let session_id = encode_path_segment(remote_session_id);
    get_remote_json(target, &format!("/v1/sessions/{session_id}/git-diff"))
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
            target,
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
    Err(RemoteSessionError::new(
        StatusCode::BAD_REQUEST,
        "VALIDATION_FAILED",
        "cwd is required for remote launch",
    ))
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

async fn remote_response_error(
    target: &LaunchTargetSummary,
    response: reqwest::Response,
    code: &'static str,
    fallback: String,
) -> RemoteSessionError {
    let status = response.status();
    let message = match response.json::<ErrorResponse>().await {
        Ok(body) => body.message.unwrap_or(fallback),
        Err(_) => fallback,
    };
    let message = redact_remote_secret_values(target, message);
    RemoteSessionError::new(
        StatusCode::BAD_GATEWAY,
        code,
        format!("{message} (remote status {status})"),
    )
}

fn redact_remote_secret_values(target: &LaunchTargetSummary, mut message: String) -> String {
    if let Some(env_key) = target
        .auth_token_env
        .as_deref()
        .map(str::trim)
        .filter(|env_key| !env_key.is_empty())
    {
        redact_env_secret(&mut message, env_key);
    }
    for env_key in ["AUTH_TOKEN", "OBSERVER_TOKEN"] {
        redact_env_secret(&mut message, env_key);
    }
    message
}

fn redact_env_secret(message: &mut String, env_key: &str) {
    let Ok(secret) = std::env::var(env_key) else {
        return;
    };
    let secret = secret.trim();
    if !secret.is_empty() && message.contains(secret) {
        *message = message.replace(secret, "[redacted]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CreateSessionsBatchResult, SessionBatchMembership, SessionState, SessionTimelinePinned,
        SessionTimelineResponse, SpawnTool, ThoughtState, TransportHealth,
        SUMMARY_CAUSE_REMOTE_POLL_DEGRADED,
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
        let mut summary = SessionSummary::live(
            session_id,
            "7",
            SessionState::Idle,
            None,
            Default::default(),
            "/monoserver/opensource/swimmers",
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        );
        summary.rest_state =
            crate::types::fallback_rest_state(SessionState::Idle, ThoughtState::Holding);
        summary.batch = None::<SessionBatchMembership>;
        summary
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

    async fn capture_timeline(
        axum::extract::Path(session_id): axum::extract::Path<String>,
    ) -> AxumJson<SessionTimelineResponse> {
        AxumJson(SessionTimelineResponse {
            session_id,
            available: true,
            cwd: "/monoserver/opensource/swimmers".to_string(),
            tool: Some("Codex".to_string()),
            events: Vec::new(),
            pinned: SessionTimelinePinned::default(),
            message: None,
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

    async fn spawn_timeline_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/v1/sessions/{session_id}/timeline", get(capture_timeline));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test api");
        });
        (format!("http://{addr}"), handle)
    }

    const REMOTE_OPERATOR_TOKEN_ENV: &str = "SWIMMERS_REMOTE_SMOKE_OPERATOR_TOKEN";
    const REMOTE_OBSERVER_TOKEN_ENV: &str = "SWIMMERS_REMOTE_SMOKE_OBSERVER_TOKEN";
    const REMOTE_OPERATOR_TOKEN: &str = "operator-token-sensitive-remote-smoke";
    const REMOTE_OBSERVER_TOKEN: &str = "observer-token-sensitive-remote-smoke";

    #[derive(Debug, Clone)]
    struct RemoteSmokeRequest {
        method: &'static str,
        path: String,
        auth: Option<String>,
        body: serde_json::Value,
    }

    #[derive(Clone, Default)]
    struct RemoteSmokeState {
        requests: Arc<Mutex<Vec<RemoteSmokeRequest>>>,
    }

    impl RemoteSmokeState {
        async fn capture(
            &self,
            method: &'static str,
            path: impl Into<String>,
            headers: &HeaderMap,
            body: serde_json::Value,
        ) {
            self.requests.lock().await.push(RemoteSmokeRequest {
                method,
                path: path.into(),
                auth: headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
                body,
            });
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RemoteSmokeScope {
        Operator,
        Observer,
        Unauthenticated,
    }

    fn remote_smoke_scope(headers: &HeaderMap) -> RemoteSmokeScope {
        match remote_smoke_bearer(headers) {
            Some(REMOTE_OPERATOR_TOKEN) => RemoteSmokeScope::Operator,
            Some(REMOTE_OBSERVER_TOKEN) => RemoteSmokeScope::Observer,
            _ => RemoteSmokeScope::Unauthenticated,
        }
    }

    fn remote_smoke_bearer(headers: &HeaderMap) -> Option<&str> {
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
    }

    fn remote_smoke_auth_error(headers: &HeaderMap, required_scope: &str) -> Response {
        let token = remote_smoke_bearer(headers).unwrap_or("<missing>");
        let status = match remote_smoke_scope(headers) {
            RemoteSmokeScope::Unauthenticated => StatusCode::UNAUTHORIZED,
            RemoteSmokeScope::Observer | RemoteSmokeScope::Operator => StatusCode::FORBIDDEN,
        };
        (
            status,
            AxumJson(error_body_msg(
                "REMOTE_AUTH_REJECTED",
                format!("token {token} lacks required {required_scope} scope"),
            )),
        )
            .into_response()
    }

    async fn remote_smoke_list_sessions(
        axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
        headers: HeaderMap,
    ) -> Response {
        state
            .capture("GET", "/v1/sessions", &headers, serde_json::Value::Null)
            .await;
        match remote_smoke_scope(&headers) {
            RemoteSmokeScope::Operator | RemoteSmokeScope::Observer => {
                AxumJson(SessionListResponse {
                    sessions: vec![summary("sess_list")],
                    version: 0,
                    repo_themes: Default::default(),
                })
                .into_response()
            }
            RemoteSmokeScope::Unauthenticated => remote_smoke_auth_error(&headers, "read"),
        }
    }

    async fn remote_smoke_create_session(
        axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
        headers: HeaderMap,
        AxumJson(body): AxumJson<CreateSessionRequest>,
    ) -> Response {
        state
            .capture(
                "POST",
                "/v1/sessions",
                &headers,
                serde_json::to_value(&body).expect("serialize create body"),
            )
            .await;
        if remote_smoke_scope(&headers) != RemoteSmokeScope::Operator {
            return remote_smoke_auth_error(&headers, "operator");
        }
        (
            StatusCode::CREATED,
            AxumJson(CreateSessionResponse {
                session: summary("sess_create"),
                repo_theme: None,
            }),
        )
            .into_response()
    }

    async fn remote_smoke_create_batch(
        axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
        headers: HeaderMap,
        AxumJson(body): AxumJson<CreateSessionsBatchRequest>,
    ) -> Response {
        state
            .capture(
                "POST",
                "/v1/sessions/batch",
                &headers,
                serde_json::to_value(&body).expect("serialize batch body"),
            )
            .await;
        if remote_smoke_scope(&headers) != RemoteSmokeScope::Operator {
            return remote_smoke_auth_error(&headers, "operator");
        }
        let results = body
            .dirs
            .into_iter()
            .enumerate()
            .map(|(index, cwd)| CreateSessionsBatchResult {
                index,
                cwd,
                ok: true,
                session: Some(summary(&format!("sess_batch_{index}"))),
                repo_theme: None,
                error: None,
            })
            .collect();
        (
            StatusCode::CREATED,
            AxumJson(CreateSessionsBatchResponse { results }),
        )
            .into_response()
    }

    async fn remote_smoke_agent_context(
        axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
        headers: HeaderMap,
        axum::extract::Path(session_id): axum::extract::Path<String>,
    ) -> Response {
        let path = format!("/v1/sessions/{session_id}/agent-context");
        state
            .capture("GET", path, &headers, serde_json::Value::Null)
            .await;
        match remote_smoke_scope(&headers) {
            RemoteSmokeScope::Operator | RemoteSmokeScope::Observer => {
                AxumJson(SessionAgentContextResponse {
                    session_id,
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
                })
                .into_response()
            }
            RemoteSmokeScope::Unauthenticated => remote_smoke_auth_error(&headers, "read"),
        }
    }

    async fn remote_smoke_git_diff(
        axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
        headers: HeaderMap,
        axum::extract::Path(session_id): axum::extract::Path<String>,
    ) -> Response {
        let path = format!("/v1/sessions/{session_id}/git-diff");
        state
            .capture("GET", path, &headers, serde_json::Value::Null)
            .await;
        match remote_smoke_scope(&headers) {
            RemoteSmokeScope::Operator | RemoteSmokeScope::Observer => {
                AxumJson(SessionGitDiffResponse {
                    session_id,
                    available: true,
                    cwd: "/monoserver/opensource/swimmers".to_string(),
                    repo_root: Some("/monoserver/opensource/swimmers".to_string()),
                    status_short: " M src/lib.rs\n".to_string(),
                    unstaged_diff: String::new(),
                    staged_diff: String::new(),
                    truncated: false,
                    message: None,
                    files: Vec::new(),
                })
                .into_response()
            }
            RemoteSmokeScope::Unauthenticated => remote_smoke_auth_error(&headers, "read"),
        }
    }

    async fn spawn_remote_smoke_server() -> (String, tokio::task::JoinHandle<()>, RemoteSmokeState)
    {
        let state = RemoteSmokeState::default();
        let app = Router::new()
            .route(
                "/v1/sessions",
                get(remote_smoke_list_sessions).post(remote_smoke_create_session),
            )
            .route("/v1/sessions/batch", post(remote_smoke_create_batch))
            .route(
                "/v1/sessions/{session_id}/agent-context",
                get(remote_smoke_agent_context),
            )
            .route(
                "/v1/sessions/{session_id}/git-diff",
                get(remote_smoke_git_diff),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind remote smoke server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve remote smoke api");
        });
        (format!("http://{addr}"), handle, state)
    }

    fn remote_smoke_target(base_url: &str, auth_token_env: &str) -> LaunchTargetSummary {
        let mut target = target();
        target.base_url = Some(base_url.to_string());
        target.auth_token_env = Some(auth_token_env.to_string());
        target
    }

    fn has_request(requests: &[RemoteSmokeRequest], method: &str, path: &str) -> bool {
        requests
            .iter()
            .any(|request| request.method == method && request.path == path)
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
    fn launch_cwd_rejects_missing_cwd() {
        let err = launch_cwd(Some("   ")).expect_err("blank cwd should be invalid");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_FAILED");
        assert!(err.message().contains("cwd is required"));

        let err = launch_cwd(None).expect_err("missing cwd should be invalid");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_FAILED");
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
    fn encode_path_segment_escapes_reserved_url_characters() {
        assert_eq!(
            encode_path_segment("target::sess/weird?x#frag"),
            "target%3A%3Asess%2Fweird%3Fx%23frag"
        );
        assert_eq!(encode_path_segment("sess_2-okay.~"), "sess_2-okay.~");
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

    #[tokio::test]
    async fn remote_poll_failure_returns_cached_stale_sessions_with_degraded_metadata() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        reset_remote_target_session_cache_for_tests();
        let client = http_client(REMOTE_LIST_TIMEOUT).expect("http client");

        let (base_url, handle) = spawn_list_server().await;
        let mut target = target();
        target.base_url = Some(base_url);

        let initial = list_remote_sessions_for_poll_target(&client, target.clone()).await;
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].transport_health, TransportHealth::Healthy);
        assert!(!initial[0].is_stale);
        handle.abort();

        let (bad_base_url, bad_handle) = spawn_timeline_server().await;
        target.base_url = Some(bad_base_url);
        let stale = list_remote_sessions_for_poll_target(&client, target).await;

        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].session_id,
            namespace_session_id("jeremy-skillbox", "sess_1")
        );
        assert!(stale[0].is_stale);
        assert_eq!(stale[0].transport_health, TransportHealth::Degraded);
        assert_eq!(
            stale[0].state_evidence.cause,
            SUMMARY_CAUSE_REMOTE_POLL_DEGRADED
        );
        assert!(stale[0].state_evidence.observed_at.is_some());

        bad_handle.abort();
        reset_remote_target_session_cache_for_tests();
    }

    #[tokio::test]
    async fn one_failed_remote_target_does_not_hide_other_targets() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        reset_remote_target_session_cache_for_tests();
        let client = http_client(REMOTE_LIST_TIMEOUT).expect("http client");

        let (alpha_base_url, alpha_handle) = spawn_list_server().await;
        let mut alpha = target();
        alpha.id = "alpha".to_string();
        alpha.label = "Alpha".to_string();
        alpha.base_url = Some(alpha_base_url);

        let initial_alpha = list_remote_sessions_for_poll_target(&client, alpha.clone()).await;
        assert_eq!(
            initial_alpha[0].session_id,
            namespace_session_id("alpha", "sess_1")
        );
        alpha_handle.abort();

        let (bad_base_url, bad_handle) = spawn_timeline_server().await;
        let mut failed_alpha = alpha;
        failed_alpha.base_url = Some(bad_base_url);

        let (beta_base_url, beta_handle) = spawn_list_server().await;
        let mut beta = target();
        beta.id = "beta".to_string();
        beta.label = "Beta".to_string();
        beta.base_url = Some(beta_base_url);

        let sessions = list_remote_sessions_for_targets(vec![failed_alpha, beta]).await;
        assert_eq!(sessions.len(), 2);
        let alpha = sessions
            .iter()
            .find(|session| session.session_id == namespace_session_id("alpha", "sess_1"))
            .expect("stale alpha session remains visible");
        let beta = sessions
            .iter()
            .find(|session| session.session_id == namespace_session_id("beta", "sess_1"))
            .expect("healthy beta session remains visible");

        assert!(alpha.is_stale);
        assert_eq!(alpha.transport_health, TransportHealth::Degraded);
        assert!(!beta.is_stale);
        assert_eq!(beta.transport_health, TransportHealth::Healthy);

        bad_handle.abort();
        beta_handle.abort();
        reset_remote_target_session_cache_for_tests();
    }

    #[tokio::test]
    async fn fetch_remote_timeline_namespaces_response_session_id() {
        let (base_url, handle) = spawn_timeline_server().await;
        let mut target = target();
        target.base_url = Some(base_url);

        let response = fetch_remote_timeline(&target, "sess_2")
            .await
            .expect("remote timeline");

        assert_eq!(
            response.session_id,
            namespace_session_id("jeremy-skillbox", "sess_2")
        );
        assert_eq!(response.available, true);
        handle.abort();
    }

    #[tokio::test]
    async fn fetch_remote_timeline_encodes_reserved_session_id_path_segment() {
        let (base_url, handle) = spawn_timeline_server().await;
        let mut target = target();
        target.base_url = Some(base_url);

        let response = fetch_remote_timeline(&target, "sess/weird?x#frag")
            .await
            .expect("remote timeline with reserved characters");

        assert_eq!(
            response.session_id,
            namespace_session_id("jeremy-skillbox", "sess/weird?x#frag")
        );
        handle.abort();
    }

    #[tokio::test]
    async fn remote_api_smoke_matrix_covers_launch_reads_scopes_and_redaction() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var(REMOTE_OPERATOR_TOKEN_ENV, REMOTE_OPERATOR_TOKEN);
        std::env::set_var(REMOTE_OBSERVER_TOKEN_ENV, REMOTE_OBSERVER_TOKEN);

        let (base_url, handle, state) = spawn_remote_smoke_server().await;
        let operator_target = remote_smoke_target(&base_url, REMOTE_OPERATOR_TOKEN_ENV);
        let observer_target = remote_smoke_target(&base_url, REMOTE_OBSERVER_TOKEN_ENV);
        let client = http_client(REMOTE_LIST_TIMEOUT).expect("http client");

        let listed = list_remote_sessions_for_target(
            &client,
            operator_target.clone(),
            remote_auth_token(&operator_target).expect("operator token"),
        )
        .await
        .expect("remote list with operator token");
        assert_eq!(
            listed[0].session_id,
            namespace_session_id("jeremy-skillbox", "sess_list")
        );

        let created = create_remote_session_on_target(
            &operator_target,
            CreateSessionRequest {
                name: Some("remote create".to_string()),
                cwd: Some("/monoserver/opensource/swimmers".to_string()),
                spawn_tool: Some(SpawnTool::Codex),
                launch_target: Some("jeremy-skillbox".to_string()),
                initial_request: Some("run remote tests".to_string()),
            },
        )
        .await
        .expect("remote create with operator token");
        assert_eq!(
            created.session.session_id,
            namespace_session_id("jeremy-skillbox", "sess_create")
        );

        let batch = create_remote_sessions_batch_on_target(
            &operator_target,
            CreateSessionsBatchRequest {
                dirs: vec![
                    "/monoserver/opensource/swimmers".to_string(),
                    "/monoserver/opensource/skillbox".to_string(),
                ],
                spawn_tool: Some(SpawnTool::Codex),
                launch_target: Some("jeremy-skillbox".to_string()),
                initial_request: Some("fan out".to_string()),
            },
        )
        .await
        .expect("remote batch with operator token");
        assert_eq!(batch.success_count(), 2);
        assert_eq!(
            batch.results[1]
                .session
                .as_ref()
                .expect("batch session")
                .session_id,
            namespace_session_id("jeremy-skillbox", "sess_batch_1")
        );

        let observer_listed = list_remote_sessions_for_target(
            &client,
            observer_target.clone(),
            remote_auth_token(&observer_target).expect("observer token"),
        )
        .await
        .expect("remote list with observer token");
        assert_eq!(observer_listed.len(), 1);

        let context = fetch_remote_agent_context(&observer_target, "sess_agent")
            .await
            .expect("observer can read agent context");
        assert_eq!(
            context.session_id,
            namespace_session_id("jeremy-skillbox", "sess_agent")
        );
        assert_eq!(context.user_task.as_deref(), Some("remote task"));

        let diff = fetch_remote_git_diff(&observer_target, "sess_diff")
            .await
            .expect("observer can read git diff");
        assert_eq!(
            diff.session_id,
            namespace_session_id("jeremy-skillbox", "sess_diff")
        );

        let observer_create = create_remote_session_on_target(
            &observer_target,
            CreateSessionRequest {
                name: None,
                cwd: Some("/monoserver/opensource/swimmers".to_string()),
                spawn_tool: Some(SpawnTool::Codex),
                launch_target: None,
                initial_request: None,
            },
        )
        .await
        .expect_err("observer token must not create sessions");
        assert_eq!(observer_create.status, StatusCode::BAD_GATEWAY);
        assert_eq!(observer_create.code, "REMOTE_LAUNCH_FAILED");
        assert!(observer_create.message().contains("[redacted]"));
        assert!(!observer_create.message().contains(REMOTE_OBSERVER_TOKEN));
        assert!(observer_create
            .message()
            .contains("remote status 403 Forbidden"));

        let observer_batch = create_remote_sessions_batch_on_target(
            &observer_target,
            CreateSessionsBatchRequest {
                dirs: vec!["/monoserver/opensource/swimmers".to_string()],
                spawn_tool: Some(SpawnTool::Codex),
                launch_target: None,
                initial_request: None,
            },
        )
        .await
        .expect_err("observer token must not batch-create sessions");
        assert!(observer_batch.message().contains("[redacted]"));
        assert!(!observer_batch.message().contains(REMOTE_OBSERVER_TOKEN));

        let requests = state.requests.lock().await;
        assert!(has_request(&requests, "GET", "/v1/sessions"));
        assert!(has_request(&requests, "POST", "/v1/sessions"));
        assert!(has_request(&requests, "POST", "/v1/sessions/batch"));
        assert!(has_request(
            &requests,
            "GET",
            "/v1/sessions/sess_agent/agent-context"
        ));
        assert!(has_request(
            &requests,
            "GET",
            "/v1/sessions/sess_diff/git-diff"
        ));

        let operator_create = requests
            .iter()
            .find(|request| {
                request.method == "POST"
                    && request.path == "/v1/sessions"
                    && request.auth.as_deref()
                        == Some(format!("Bearer {REMOTE_OPERATOR_TOKEN}").as_str())
            })
            .expect("operator create request");
        assert!(operator_create.body.get("launch_target").is_none());
        assert_eq!(
            operator_create.body["initial_request"].as_str(),
            Some("run remote tests")
        );

        let operator_batch = requests
            .iter()
            .find(|request| {
                request.method == "POST"
                    && request.path == "/v1/sessions/batch"
                    && request.auth.as_deref()
                        == Some(format!("Bearer {REMOTE_OPERATOR_TOKEN}").as_str())
            })
            .expect("operator batch request");
        assert!(operator_batch.body.get("launch_target").is_none());
        assert_eq!(
            operator_batch.body["dirs"].as_array().expect("dirs").len(),
            2
        );

        drop(requests);
        handle.abort();
        std::env::remove_var(REMOTE_OPERATOR_TOKEN_ENV);
        std::env::remove_var(REMOTE_OBSERVER_TOKEN_ENV);
    }
}
