use std::collections::HashMap;
use std::net::IpAddr;
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
use serde::Serialize;

use crate::api::envelope::error_body_msg;
use crate::api::service::validate_sessions_batch_dirs;
use crate::config::Config;
use crate::session::overlay::default_overlay;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, CreateSessionsBatchRequest,
    CreateSessionsBatchResponse, DependencyHealthStatus, DirListResponse, EnvironmentAuthSummary,
    EnvironmentSummary, ErrorResponse, LaunchPathMapping, LaunchTargetSummary,
    SessionAgentContextResponse, SessionEnvironmentSummary, SessionGitDiffResponse,
    SessionGroupInputRequest, SessionGroupInputResponse, SessionInputRequest, SessionInputResponse,
    SessionListResponse, SessionPaneTailResponse, SessionSummary, SessionTimelineResponse,
    SessionTranscriptResponse,
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
    last_error_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
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

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub fn display_message(&self, status: impl std::fmt::Display) -> String {
        ErrorResponse::with_message(self.code, &self.message).display_message(status)
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

fn namespace_response_session_id(target: &LaunchTargetSummary, session_id: &str) -> String {
    namespace_session_id(&target.id, session_id)
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
    let remote_session_id = session.session_id.clone();
    let remote_cwd = session.cwd.clone();
    let local_cwd = map_remote_cwd_to_local(target, &remote_cwd);
    session.cwd = local_cwd.clone().unwrap_or_else(|| remote_cwd.clone());
    session.session_id = namespace_session_id(&target.id, &session.session_id);
    if !session.tmux_name.starts_with('[') {
        session.tmux_name = format!("[{}] {}", target.label, session.tmux_name);
    }
    session.environment = SessionEnvironmentSummary::remote(
        target,
        remote_session_id,
        remote_cwd,
        local_cwd,
        "remote_swimmers_api",
    );
    session
}

pub fn environment_summaries(include_remote: bool) -> Vec<EnvironmentSummary> {
    let mut local = EnvironmentSummary::local();
    local.advisory = crate::advisory::advisory_for_target("local", None);
    let mut environments = vec![local];
    if !include_remote {
        return environments;
    }

    let Some(overlay) = default_overlay() else {
        return environments;
    };
    environments.extend(
        overlay
            .all_launch_targets()
            .into_iter()
            .filter(is_swimmers_api_target)
            .map(|target| environment_summary_for_target(&target)),
    );
    environments
}

fn environment_summary_for_target(target: &LaunchTargetSummary) -> EnvironmentSummary {
    let health = remote_target_environment_health(target);
    EnvironmentSummary {
        id: target.id.clone(),
        label: target.label.clone(),
        kind: target.kind.clone(),
        backend_mode: "remote_swimmers_api".to_string(),
        base_url: sanitized_target_base_url(target),
        auth: environment_auth_summary(target),
        path_mapping_count: target.path_mappings.len(),
        status: health.status,
        last_seen_at: health.last_seen_at,
        last_error_at: health.last_error_at,
        last_error: health.last_error,
        freshness_ms: health.freshness_ms,
        advisory: crate::advisory::advisory_for_target(&target.id, None),
    }
}

fn sanitized_target_base_url(target: &LaunchTargetSummary) -> Option<String> {
    let raw = target.base_url.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let mut url = reqwest::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if !url.username().is_empty() {
        let _ = url.set_username("");
    }
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn environment_auth_summary(target: &LaunchTargetSummary) -> EnvironmentAuthSummary {
    target
        .auth_token_env
        .as_deref()
        .map(str::trim)
        .filter(|env_key| !env_key.is_empty())
        .map(|env_key| EnvironmentAuthSummary {
            mode: "token_env".to_string(),
            token_env_present: Some(auth_env_value_present(env_key)),
        })
        .unwrap_or_else(|| EnvironmentAuthSummary {
            mode: "none".to_string(),
            token_env_present: None,
        })
}

fn auth_env_value_present(env_key: &str) -> bool {
    matches!(std::env::var(env_key), Ok(value) if !value.trim().is_empty())
}

#[derive(Debug, Clone)]
struct RemoteTargetHealth {
    status: DependencyHealthStatus,
    last_seen_at: Option<DateTime<Utc>>,
    last_error_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    freshness_ms: Option<u64>,
}

fn remote_target_environment_health(target: &LaunchTargetSummary) -> RemoteTargetHealth {
    let now = Utc::now();
    let config_error = remote_target_config_error(target);
    with_remote_target_session_cache(|cache| {
        let Some(entry) = cache.get(&target.id) else {
            return RemoteTargetHealth {
                status: config_error
                    .as_ref()
                    .map(|_| DependencyHealthStatus::Unavailable)
                    .unwrap_or(DependencyHealthStatus::Unknown),
                last_seen_at: None,
                last_error_at: config_error.as_ref().map(|_| now),
                last_error: config_error,
                freshness_ms: None,
            };
        };
        let current_error = cached_poll_error_is_current(entry);
        let mut status = if current_error {
            if entry.last_seen_at.is_some() {
                DependencyHealthStatus::Degraded
            } else {
                DependencyHealthStatus::Unavailable
            }
        } else if now_ms() < entry.backoff_until_ms {
            if entry.last_seen_at.is_some() {
                DependencyHealthStatus::Degraded
            } else {
                DependencyHealthStatus::Unavailable
            }
        } else if entry.last_seen_at.is_some() {
            DependencyHealthStatus::Healthy
        } else {
            DependencyHealthStatus::Unknown
        };
        let mut last_error_at = entry.last_error_at;
        let mut last_error = entry.last_error.clone();
        if let Some(error) = config_error {
            status = DependencyHealthStatus::Unavailable;
            last_error_at = Some(last_error_at.unwrap_or(now));
            last_error = Some(error);
        }
        let freshness_ms = entry.last_seen_at.and_then(|seen| {
            now.signed_duration_since(seen)
                .to_std()
                .ok()
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        });
        RemoteTargetHealth {
            status,
            last_seen_at: entry.last_seen_at,
            last_error_at,
            last_error,
            freshness_ms,
        }
    })
}

fn cached_poll_error_is_current(entry: &RemoteTargetSessionCache) -> bool {
    match (entry.last_error_at, entry.last_seen_at) {
        (Some(error_at), Some(seen_at)) => error_at >= seen_at,
        (Some(_), None) => true,
        _ => false,
    }
}

fn remote_target_config_error(target: &LaunchTargetSummary) -> Option<String> {
    remote_base_url_config_error(target).or_else(|| {
        target
            .auth_token_env
            .as_deref()
            .map(str::trim)
            .filter(|env_key| !env_key.is_empty())
            .filter(|env_key| !auth_env_value_present(env_key))
            .map(|_| "auth_env_missing".to_string())
    })
}

fn remote_base_url_config_error(target: &LaunchTargetSummary) -> Option<String> {
    parse_remote_base_url(target)
        .err()
        .map(|_| "base_url_unavailable".to_string())
}

pub fn remote_targets_health_snapshot() -> crate::types::DependencyHealthSnapshot {
    let now = Utc::now();
    let Some(overlay) = default_overlay() else {
        return crate::types::DependencyHealthSnapshot::unknown(now)
            .with_detail("configured_targets", "unknown")
            .with_detail("probe", "overlay_unavailable");
    };
    remote_targets_health_snapshot_for_targets(overlay.all_launch_targets(), &Config::from_env())
}

fn remote_targets_health_snapshot_for_targets(
    targets: Vec<LaunchTargetSummary>,
    config: &Config,
) -> crate::types::DependencyHealthSnapshot {
    let now = Utc::now();
    let mut skipped_current_server_targets = 0usize;
    let targets = targets
        .into_iter()
        .filter(is_swimmers_api_target)
        .filter(|target| {
            let is_current = target_points_at_current_server(target, config);
            if is_current {
                skipped_current_server_targets += 1;
            }
            !is_current
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return crate::types::DependencyHealthSnapshot::not_configured(now)
            .with_detail("configured_targets", "0")
            .with_detail(
                "skipped_current_server_targets",
                skipped_current_server_targets.to_string(),
            );
    }

    let mut rollup = RemoteTargetHealthRollup::default();
    for target in &targets {
        rollup.observe_target(target);
    }
    rollup.into_snapshot(now, targets.len(), skipped_current_server_targets)
}

#[derive(Default)]
struct RemoteTargetHealthRollup {
    healthy: usize,
    degraded: usize,
    unavailable: usize,
    unknown: usize,
    auth_present: usize,
    auth_missing: usize,
    mappings_total: usize,
    missing_mappings: usize,
    missing_base_url: usize,
    last_seen_at: Option<DateTime<Utc>>,
    last_error_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

impl RemoteTargetHealthRollup {
    fn observe_target(&mut self, target: &LaunchTargetSummary) {
        self.observe_mapping_config(target);
        self.observe_auth_config(target);
        self.observe_health(remote_target_environment_health(target));
    }

    fn observe_mapping_config(&mut self, target: &LaunchTargetSummary) {
        self.mappings_total += target.path_mappings.len();
        if target.path_mappings.is_empty() {
            self.missing_mappings += 1;
        }
        if remote_base_url_config_error(target).is_some() {
            self.missing_base_url += 1;
        }
    }

    fn observe_auth_config(&mut self, target: &LaunchTargetSummary) {
        let Some(env_key) = target
            .auth_token_env
            .as_deref()
            .map(str::trim)
            .filter(|env_key| !env_key.is_empty())
        else {
            return;
        };
        if auth_env_value_present(env_key) {
            self.auth_present += 1;
        } else {
            self.auth_missing += 1;
        }
    }

    fn observe_health(&mut self, health: RemoteTargetHealth) {
        match health.status {
            DependencyHealthStatus::Healthy => self.healthy += 1,
            DependencyHealthStatus::Degraded => self.degraded += 1,
            DependencyHealthStatus::Unavailable => self.unavailable += 1,
            DependencyHealthStatus::Unknown | DependencyHealthStatus::NotConfigured => {
                self.unknown += 1
            }
        }
        if health.last_seen_at > self.last_seen_at {
            self.last_seen_at = health.last_seen_at;
        }
        if health.last_error_at > self.last_error_at {
            self.last_error_at = health.last_error_at;
            self.last_error = health.last_error;
        }
    }

    fn status(&self) -> DependencyHealthStatus {
        aggregate_remote_target_status(
            self.healthy,
            self.degraded,
            self.unavailable,
            self.unknown,
            self.missing_mappings > 0,
        )
    }

    fn into_snapshot(
        self,
        now: DateTime<Utc>,
        configured_targets: usize,
        skipped_current_server_targets: usize,
    ) -> crate::types::DependencyHealthSnapshot {
        let status = self.status();
        let mut snapshot = match status {
            DependencyHealthStatus::Healthy => crate::types::DependencyHealthSnapshot::healthy(now),
            DependencyHealthStatus::Degraded => crate::types::DependencyHealthSnapshot::degraded(
                now,
                self.last_error
                    .clone()
                    .unwrap_or_else(|| "remote target path mapping doctor warning".to_string()),
            ),
            DependencyHealthStatus::Unavailable => {
                crate::types::DependencyHealthSnapshot::unavailable(
                    now,
                    self.last_error
                        .clone()
                        .unwrap_or_else(|| "remote target unavailable".to_string()),
                )
            }
            DependencyHealthStatus::Unknown => crate::types::DependencyHealthSnapshot::unknown(now),
            DependencyHealthStatus::NotConfigured => {
                crate::types::DependencyHealthSnapshot::not_configured(now)
            }
        }
        .with_detail("configured_targets", configured_targets.to_string())
        .with_detail("healthy_targets", self.healthy.to_string())
        .with_detail("degraded_targets", self.degraded.to_string())
        .with_detail("unavailable_targets", self.unavailable.to_string())
        .with_detail("unknown_targets", self.unknown.to_string())
        .with_detail("auth_env_present", self.auth_present.to_string())
        .with_detail("auth_env_missing", self.auth_missing.to_string())
        .with_detail("path_mappings_total", self.mappings_total.to_string())
        .with_detail(
            "targets_without_path_mappings",
            self.missing_mappings.to_string(),
        )
        .with_detail(
            "targets_without_base_url",
            self.missing_base_url.to_string(),
        )
        .with_detail(
            "skipped_current_server_targets",
            skipped_current_server_targets.to_string(),
        )
        .with_detail("probe", "session_list_cache");
        snapshot.last_seen_at = self.last_seen_at;
        snapshot.last_error_at = self.last_error_at.or(snapshot.last_error_at);
        if snapshot.last_error.is_none() {
            snapshot.last_error = self.last_error;
        }
        snapshot.freshness_ms = self.last_seen_at.and_then(|seen| {
            now.signed_duration_since(seen)
                .to_std()
                .ok()
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        });
        snapshot
    }
}

fn aggregate_remote_target_status(
    healthy: usize,
    degraded: usize,
    unavailable: usize,
    unknown: usize,
    doctor_degraded: bool,
) -> DependencyHealthStatus {
    let has_healthy = healthy > 0;
    let has_degraded = degraded > 0;
    let has_unavailable = unavailable > 0;
    let has_unknown = unknown > 0;
    match (
        has_unavailable,
        has_healthy,
        has_degraded,
        has_unknown,
        doctor_degraded,
    ) {
        (true, false, false, _, _) => DependencyHealthStatus::Unavailable,
        (true, _, _, _, _) | (_, _, true, _, _) | (_, _, _, _, true) => {
            DependencyHealthStatus::Degraded
        }
        (false, true, false, false, false) => DependencyHealthStatus::Healthy,
        _ => DependencyHealthStatus::Unknown,
    }
}

pub fn denamespace_for_target(
    session_id: &str,
) -> Result<Option<(LaunchTargetSummary, &str)>, RemoteSessionError> {
    let Some((target, remote_session_id)) = denamespace_for_configured_target(session_id)? else {
        return Ok(None);
    };
    Ok(Some((target, remote_session_id)))
}

fn denamespace_for_configured_target(
    session_id: &str,
) -> Result<Option<(LaunchTargetSummary, &str)>, RemoteSessionError> {
    let Some(overlay) = default_overlay() else {
        return split_remote_session_id(session_id)
            .map(|(target_id, _)| {
                Err(RemoteSessionError::new(
                    StatusCode::BAD_REQUEST,
                    "LAUNCH_TARGET_UNKNOWN",
                    format!("remote session target '{target_id}' is not configured"),
                ))
            })
            .transpose();
    };

    denamespace_for_configured_targets(session_id, &overlay.all_launch_targets())
}

pub(crate) fn denamespace_for_configured_targets<'a>(
    session_id: &'a str,
    targets: &[LaunchTargetSummary],
) -> Result<Option<(LaunchTargetSummary, &'a str)>, RemoteSessionError> {
    let mut targets = targets
        .iter()
        .cloned()
        .filter_map(|target| {
            session_id
                .strip_prefix(&target.id)
                .and_then(|suffix| suffix.strip_prefix(REMOTE_SESSION_SEPARATOR))
                .filter(|remote_session_id| !remote_session_id.is_empty())
                .map(|remote_session_id| (target, remote_session_id))
        })
        .collect::<Vec<_>>();
    targets.sort_by_key(|(target, _)| std::cmp::Reverse(target.id.len()));
    if let Some((target, remote_session_id)) = targets.into_iter().next() {
        ensure_swimmers_api_target(&target)?;
        return Ok(Some((target, remote_session_id)));
    }

    let Some((target_id, _)) = split_remote_session_id(session_id) else {
        return Ok(None);
    };
    Err(RemoteSessionError::new(
        StatusCode::BAD_REQUEST,
        "LAUNCH_TARGET_UNKNOWN",
        format!("remote session target '{target_id}' is not configured"),
    ))
}

pub async fn list_remote_sessions() -> Vec<SessionSummary> {
    if !remote_polling_enabled_for_environment() {
        return Vec::new();
    }

    let Some(overlay) = default_overlay() else {
        return Vec::new();
    };
    let targets = remote_poll_targets(overlay.all_launch_targets(), &Config::from_env());
    if targets.is_empty() {
        return Vec::new();
    }

    list_remote_sessions_for_targets(targets).await
}

fn remote_polling_enabled_for_environment() -> bool {
    #[cfg(test)]
    {
        std::env::var_os("SWIMMERS_TEST_ENABLE_REMOTE_POLLING").is_some()
    }

    #[cfg(not(test))]
    {
        true
    }
}

fn remote_poll_targets(
    targets: Vec<LaunchTargetSummary>,
    config: &Config,
) -> Vec<LaunchTargetSummary> {
    targets
        .into_iter()
        .filter(is_swimmers_api_target)
        .filter(|target| !is_current_server_poll_target(target, config))
        .collect()
}

fn is_current_server_poll_target(target: &LaunchTargetSummary, config: &Config) -> bool {
    if target_points_at_current_server(target, config) {
        tracing::debug!(
            target = %target.id,
            base_url = ?sanitized_target_base_url(target),
            "skipping self-target remote session polling"
        );
        true
    } else {
        false
    }
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
                .flat_map(|target| record_remote_poll_failure(&target.id, err.code()))
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
            return record_remote_poll_failure(&target_id, err.code());
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
            record_remote_poll_failure(&target_id, err.code())
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
        last_error_at: None,
        last_error: None,
        backoff_until_ms: 0,
    };
    with_remote_target_session_cache(|cache| {
        cache.insert(target_id.to_string(), entry);
    });
}

fn record_remote_poll_failure(target_id: &str, error: impl Into<String>) -> Vec<SessionSummary> {
    let now = Utc::now();
    let backoff_until_ms = now_ms().saturating_add(REMOTE_POLL_FAILURE_BACKOFF_MS);
    with_remote_target_session_cache(|cache| {
        let entry =
            cache
                .entry(target_id.to_string())
                .or_insert_with(|| RemoteTargetSessionCache {
                    sessions: Vec::new(),
                    last_seen_at: None,
                    last_error_at: None,
                    last_error: None,
                    backoff_until_ms: 0,
                });
        entry.last_error_at = Some(now);
        entry.last_error = Some(error.into());
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
    let batch = prepare_remote_sessions_batch(body)?;
    let mut response =
        create_remote_sessions_batch_on_target(&batch.target, batch.remote_body).await?;
    restore_original_batch_cwds(&mut response, &batch.original_dirs)?;
    Ok(response)
}

struct PreparedRemoteSessionsBatch {
    target: LaunchTargetSummary,
    original_dirs: Vec<String>,
    remote_body: CreateSessionsBatchRequest,
}

fn prepare_remote_sessions_batch(
    body: CreateSessionsBatchRequest,
) -> Result<PreparedRemoteSessionsBatch, RemoteSessionError> {
    let target_id = required_target_id(body.launch_target.as_deref())?;
    let original_dirs = require_batch_dirs(body.dirs)?;
    let target = resolve_launch_target_for_cwd(&original_dirs[0], target_id)?;
    let remote_dirs = map_batch_cwds_for_target(&target, &original_dirs)?;

    Ok(PreparedRemoteSessionsBatch {
        target,
        original_dirs,
        remote_body: CreateSessionsBatchRequest {
            dirs: remote_dirs,
            spawn_tool: body.spawn_tool,
            launch_target: None,
            initial_request: body.initial_request,
        },
    })
}

fn require_batch_dirs(dirs: Vec<String>) -> Result<Vec<String>, RemoteSessionError> {
    validate_sessions_batch_dirs(&dirs).map_err(|error| {
        RemoteSessionError::new(error.status(), error.code(), error.message().to_string())
    })?;
    Ok(dirs)
}

fn map_batch_cwds_for_target(
    target: &LaunchTargetSummary,
    dirs: &[String],
) -> Result<Vec<String>, RemoteSessionError> {
    dirs.iter()
        .map(|cwd| map_cwd_for_target(target, cwd))
        .collect()
}

fn restore_original_batch_cwds(
    response: &mut CreateSessionsBatchResponse,
    original_dirs: &[String],
) -> Result<(), RemoteSessionError> {
    validate_remote_batch_result_indexes(&response.results, original_dirs.len())?;
    for result in &mut response.results {
        result.cwd = original_dirs[result.index].clone();
    }

    Ok(())
}

fn validate_remote_batch_result_indexes(
    results: &[crate::types::CreateSessionsBatchResult],
    expected_len: usize,
) -> Result<(), RemoteSessionError> {
    if results.len() != expected_len {
        return Err(remote_batch_result_count_error(results.len(), expected_len));
    }

    let mut seen = vec![false; expected_len];
    for index in results.iter().map(|result| result.index) {
        mark_remote_batch_result_index(&mut seen, index)?;
    }
    Ok(())
}

fn mark_remote_batch_result_index(
    seen: &mut [bool],
    index: usize,
) -> Result<(), RemoteSessionError> {
    let Some(slot) = seen.get_mut(index) else {
        return Err(malformed_remote_batch_response(format!(
            "remote batch response included out-of-range result index {index}"
        )));
    };
    if *slot {
        return Err(malformed_remote_batch_response(format!(
            "remote batch response included duplicate result index {index}"
        )));
    }
    *slot = true;
    Ok(())
}

fn remote_batch_result_count_error(actual: usize, expected: usize) -> RemoteSessionError {
    malformed_remote_batch_response(format!(
        "remote batch response returned {actual} results for {expected} requested dirs"
    ))
}

fn malformed_remote_batch_response(message: impl Into<String>) -> RemoteSessionError {
    RemoteSessionError::new(
        StatusCode::BAD_GATEWAY,
        "REMOTE_LAUNCH_FAILED",
        message.into(),
    )
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

pub async fn fetch_remote_pane_tail(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Result<SessionPaneTailResponse, RemoteSessionError> {
    let session_id = encode_path_segment(remote_session_id);
    get_remote_json(target, &format!("/v1/sessions/{session_id}/pane-tail"))
        .await
        .map(|mut response: SessionPaneTailResponse| {
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
    let session_id = encode_path_segment(remote_session_id);
    let query = remote_transcript_query(turn_id, after, limit);
    let query_refs = query_string_refs(&query);
    let mut response: SessionTranscriptResponse = get_remote_json_with_query(
        target,
        &format!("/v1/sessions/{session_id}/transcript"),
        &query_refs,
    )
    .await?;
    response.session_id = namespace_session_id(&target.id, &response.session_id);
    Ok(response)
}

fn remote_transcript_query(
    turn_id: Option<&str>,
    after: Option<u64>,
    limit: Option<usize>,
) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(turn_id) = turn_id.filter(|turn_id| !turn_id.trim().is_empty()) {
        query.push(("turn_id".to_string(), turn_id.to_string()));
    }
    query.extend(after.map(|after| ("after".to_string(), after.to_string())));
    query.extend(limit.map(|limit| ("limit".to_string(), limit.to_string())));
    query
}

fn query_string_refs(query: &[(String, String)]) -> Vec<(&str, &str)> {
    query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
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

pub async fn send_remote_input(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
    body: SessionInputRequest,
) -> Result<SessionInputResponse, RemoteSessionError> {
    let session_id = encode_path_segment(remote_session_id);
    let mut response: SessionInputResponse = post_remote_json(
        target,
        &format!("/v1/sessions/{session_id}/input"),
        &body,
        "REMOTE_INPUT_FAILED",
        "send input",
    )
    .await?;
    response.session_id = namespace_response_session_id(target, &response.session_id);
    Ok(response)
}

pub async fn send_remote_group_input(
    target: &LaunchTargetSummary,
    remote_session_ids: Vec<String>,
    text: String,
) -> Result<SessionGroupInputResponse, RemoteSessionError> {
    let request = SessionGroupInputRequest {
        session_ids: remote_session_ids,
        text,
    };
    let mut response: SessionGroupInputResponse = post_remote_json(
        target,
        "/v1/sessions/group-input",
        &request,
        "REMOTE_GROUP_INPUT_FAILED",
        "send group input",
    )
    .await?;
    for result in &mut response.results {
        result.session_id = namespace_response_session_id(target, &result.session_id);
    }
    Ok(SessionGroupInputResponse::from_results(response.results))
}

pub async fn list_remote_dirs(
    target_id: &str,
    path: Option<&str>,
    managed_only: bool,
    group: Option<&str>,
) -> Result<DirListResponse, RemoteSessionError> {
    let target = resolve_dir_inventory_target(target_id)?;
    let local_path = path.map(str::trim).filter(|path| !path.is_empty());
    let remote_path = remote_dir_inventory_path(&target, local_path)?;
    let managed = managed_only.to_string();
    let mut query = vec![
        ("path", remote_path.as_str()),
        ("managed_only", managed.as_str()),
    ];
    if let Some(group) = group.map(str::trim).filter(|group| !group.is_empty()) {
        query.push(("group", group));
    }
    let response = get_remote_json_with_query(&target, "/v1/dirs", &query).await?;
    Ok(remote_dir_response_for_local_cockpit(
        &target, response, local_path,
    ))
}

fn resolve_dir_inventory_target(
    target_id: &str,
) -> Result<LaunchTargetSummary, RemoteSessionError> {
    let target_id = target_id.trim();
    if target_id.is_empty() || target_id == "local" {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            "remote directory inventory requires a non-local launch target",
        ));
    }
    let Some(overlay) = default_overlay() else {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_UNKNOWN",
            "no skillbox-config overlay is available for remote directory inventory",
        ));
    };
    let target = overlay.launch_target_by_id(target_id).ok_or_else(|| {
        RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_UNKNOWN",
            format!("launch target '{target_id}' is not configured"),
        )
    })?;
    ensure_swimmers_api_target(&target)?;
    Ok(target)
}

fn remote_dir_inventory_path(
    target: &LaunchTargetSummary,
    local_path: Option<&str>,
) -> Result<String, RemoteSessionError> {
    if let Some(path) = local_path {
        return map_cwd_for_target(target, path);
    }
    target
        .path_mappings
        .first()
        .map(|mapping| mapping.remote_prefix.clone())
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            RemoteSessionError::new(
                StatusCode::BAD_REQUEST,
                "LAUNCH_TARGET_PATH_UNMAPPED",
                format!(
                    "launch target '{}' has no path_mappings remote prefix for directory inventory",
                    target.id
                ),
            )
        })
}

fn remote_dir_response_for_local_cockpit(
    target: &LaunchTargetSummary,
    mut response: DirListResponse,
    fallback_local_path: Option<&str>,
) -> DirListResponse {
    response.path = map_remote_cwd_to_local(target, &response.path)
        .or_else(|| fallback_local_path.map(str::to_string))
        .unwrap_or(response.path);
    for entry in &mut response.entries {
        if let Some(full_path) = entry.full_path.as_mut() {
            if let Some(local_path) = map_remote_cwd_to_local(target, full_path) {
                *full_path = local_path;
            }
        }
    }
    response.launch_targets = default_overlay()
        .map(|overlay| overlay.all_launch_targets())
        .filter(|targets| !targets.is_empty())
        .unwrap_or_else(|| vec![LaunchTargetSummary::local()]);
    response.default_launch_target = Some(target.id.clone());
    response
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

async fn post_remote_json<B, T>(
    target: &LaunchTargetSummary,
    path: &str,
    body: &B,
    code: &'static str,
    action: &'static str,
) -> Result<T, RemoteSessionError>
where
    B: Serialize + ?Sized,
    T: serde::de::DeserializeOwned,
{
    ensure_swimmers_api_target(target)?;
    let client = http_client(REMOTE_CREATE_TIMEOUT)?;
    let url = remote_url(target, path)?;
    let response = with_remote_auth(client.post(url), target)?
        .json(body)
        .send()
        .await
        .map_err(|err| {
            RemoteSessionError::new(
                StatusCode::BAD_GATEWAY,
                code,
                format!("failed to {action} on '{}': {err}", target.id),
            )
        })?;
    if !response.status().is_success() {
        return Err(remote_response_error(
            target,
            response,
            code,
            format!("remote target '{}' rejected {action}", target.id),
        )
        .await);
    }
    response.json::<T>().await.map_err(|err| {
        RemoteSessionError::new(
            StatusCode::BAD_GATEWAY,
            code,
            format!(
                "failed to parse remote {action} response from '{}': {err}",
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
    let Some((host, url_port)) = current_server_candidate_host_and_port(target) else {
        return false;
    };
    url_port == config.port && host_matches_current_server(&host, &config.bind)
}

fn current_server_candidate_host_and_port(target: &LaunchTargetSummary) -> Option<(String, u16)> {
    let url = parse_remote_base_url(target).ok()?;
    is_root_base_url_path(url.path()).then_some(())?;
    let host = url.host_str()?.to_string();
    Some((host, url.port_or_known_default().unwrap_or(80)))
}

fn host_matches_current_server(host: &str, bind: &str) -> bool {
    let bind_host = crate::cli::bind_host(bind);
    let loopback_url_host = is_loopback_url_host(host);
    [
        host.eq_ignore_ascii_case(bind_host),
        crate::cli::is_loopback_bind(bind) && loopback_url_host,
        is_unspecified_bind_host(bind_host) && loopback_url_host,
    ]
    .contains(&true)
}

fn is_root_base_url_path(path: &str) -> bool {
    matches!(path, "" | "/")
}

fn is_loopback_url_host(host: &str) -> bool {
    let host = unbracketed_url_host(host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn unbracketed_url_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host)
}

fn is_unspecified_bind_host(host: &str) -> bool {
    host.parse::<IpAddr>()
        .map(|ip| ip.is_unspecified())
        .unwrap_or(false)
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
                "cwd '{cwd}' is not covered by path_mappings for launch target '{}'; add a path_mappings entry from a local prefix that contains this cwd to the matching remote prefix",
                target.id
            ),
        )
    })
}

fn map_remote_cwd_to_local(target: &LaunchTargetSummary, remote_cwd: &str) -> Option<String> {
    let reverse_mappings = target
        .path_mappings
        .iter()
        .map(|mapping| LaunchPathMapping {
            local_prefix: mapping.remote_prefix.clone(),
            remote_prefix: mapping.local_prefix.clone(),
        })
        .collect::<Vec<_>>();
    map_path_with_mappings(remote_cwd, &reverse_mappings)
}

pub fn map_path_with_mappings(cwd: &str, mappings: &[LaunchPathMapping]) -> Option<String> {
    let cwd = lexical_path(cwd);
    mappings
        .iter()
        .filter_map(|mapping| {
            let local_prefix_raw = mapping.local_prefix.trim();
            let remote_prefix_raw = mapping.remote_prefix.trim();
            if local_prefix_raw.is_empty() || remote_prefix_raw.is_empty() {
                return None;
            }
            let local_prefix = lexical_path(local_prefix_raw);
            let rel = cwd.strip_prefix(&local_prefix).ok()?;
            let score = local_prefix.components().count();
            let mut remote = PathBuf::from(remote_prefix_raw);
            if !rel.as_os_str().is_empty() {
                remote.push(rel);
            }
            Some((score, remote.to_string_lossy().into_owned()))
        })
        .fold(
            None,
            |best: Option<(usize, String)>, candidate| match best {
                Some((best_score, best_path)) if best_score >= candidate.0 => {
                    Some((best_score, best_path))
                }
                _ => Some(candidate),
            },
        )
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
    let url = parse_remote_base_url(target)?;
    Ok(format!(
        "{}/{}",
        url.as_str().trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
}

fn parse_remote_base_url(target: &LaunchTargetSummary) -> Result<reqwest::Url, RemoteSessionError> {
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
    let url = reqwest::Url::parse(base_url).map_err(|err| {
        RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            format!("launch target '{}' has invalid base_url: {err}", target.id),
        )
    })?;
    if url.host_str().is_none() {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            format!("launch target '{}' base_url must include a host", target.id),
        ));
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            format!(
                "launch target '{}' base_url must use http or https",
                target.id
            ),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(RemoteSessionError::new(
            StatusCode::BAD_REQUEST,
            "LAUNCH_TARGET_INVALID",
            format!(
                "launch target '{}' base_url must not include credentials, query, or fragment",
                target.id
            ),
        ));
    }
    Ok(url)
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
mod tests;
