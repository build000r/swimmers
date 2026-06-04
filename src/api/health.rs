use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::api::AppState;
use crate::persistence::file_store::{PersistenceHealthSnapshot, PersistenceLoadSnapshot};
use crate::session::overlay::{default_overlay_health, remote_targets_health};
use crate::thought::health::{BridgeHealthSnapshot, BridgeStatus};
use crate::types::{
    DependencyHealthLedger, DependencyHealthSnapshot, DependencyHealthStatus,
    ThoughtPersistenceBackpressureSnapshot,
};

#[derive(Debug, Serialize)]
struct PersistenceHealth {
    available: bool,
    ok: bool,
    consecutive_failures: u64,
    last_success_at: Option<DateTime<Utc>>,
    last_successful_operation: Option<String>,
    last_failure_at: Option<DateTime<Utc>>,
    last_failed_operation: Option<String>,
    last_error: Option<String>,
    load_outcomes: std::collections::BTreeMap<String, PersistenceLoadSnapshot>,
}

impl PersistenceHealth {
    fn unavailable() -> Self {
        Self {
            available: false,
            ok: false,
            consecutive_failures: 0,
            last_success_at: None,
            last_successful_operation: None,
            last_failure_at: None,
            last_failed_operation: None,
            last_error: None,
            load_outcomes: std::collections::BTreeMap::new(),
        }
    }

    fn from_snapshot(snapshot: PersistenceHealthSnapshot) -> Self {
        Self {
            available: true,
            ok: snapshot.ok,
            consecutive_failures: snapshot.consecutive_failures,
            last_success_at: snapshot.last_success_at,
            last_successful_operation: snapshot.last_successful_operation,
            last_failure_at: snapshot.last_failure_at,
            last_failed_operation: snapshot.last_failed_operation,
            last_error: snapshot.last_error,
            load_outcomes: snapshot.load_outcomes,
        }
    }

    fn is_ready(&self) -> bool {
        self.available && self.ok
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: BridgeStatus,
    uptime_secs: u64,
    thought_bridge: BridgeHealthSnapshot,
    thought_persistence: ThoughtPersistenceBackpressureSnapshot,
    persistence: PersistenceHealth,
    dependencies: DependencyHealthLedger,
}

#[derive(Debug, Serialize)]
struct VersionResponse {
    name: &'static str,
    version: &'static str,
}

// ---------------------------------------------------------------------------
// Process start time — captured the first time it is read.
// ---------------------------------------------------------------------------

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

async fn health_response(state: &Arc<AppState>) -> HealthResponse {
    let uptime_secs = process_start().elapsed().as_secs();
    let thought_bridge = state.bridge_health.snapshot();
    let persistence_snapshot = state
        .current_file_store()
        .map(|store| store.health_snapshot());
    let persistence = persistence_snapshot
        .clone()
        .map(PersistenceHealth::from_snapshot)
        .unwrap_or_else(PersistenceHealth::unavailable);
    let thought_persistence = state.supervisor.thought_persistence_backpressure_snapshot();
    let native_app = *state.native_desktop_app.read().await;
    let dependencies = dependency_ledger(
        state,
        &thought_bridge,
        persistence_snapshot,
        &thought_persistence,
        native_app,
    );
    HealthResponse {
        status: thought_bridge.status,
        uptime_secs,
        thought_bridge,
        thought_persistence,
        persistence,
        dependencies,
    }
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(health_response(&state).await)
}

async fn readyz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let response = health_response(&state).await;
    let status = if response.thought_bridge.is_ready() && response.persistence.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response))
}

/// Canonical string label for a dependency-health status.
///
/// Mirrors the `#[serde(rename_all = "snake_case")]` wire encoding of
/// [`DependencyHealthStatus`] so the embedded in-process surface labels each
/// dependency exactly as the HTTP `/health` JSON would, without a second
/// hand-maintained match. Keep this in sync with the enum's serde attribute.
pub fn dependency_status_label(status: DependencyHealthStatus) -> &'static str {
    match status {
        DependencyHealthStatus::Unknown => "unknown",
        DependencyHealthStatus::Healthy => "healthy",
        DependencyHealthStatus::Degraded => "degraded",
        DependencyHealthStatus::Unavailable => "unavailable",
        DependencyHealthStatus::NotConfigured => "not_configured",
    }
}

/// Assemble the complete dependency-health ledger from live state.
///
/// This is the single source of truth for the dependency ledger. Both the HTTP
/// `/health` surface and the embedded in-process TUI adapter call it so the two
/// surfaces never drift in which dependencies they report or how each
/// dependency's status is derived. It fetches every input itself (bridge,
/// persistence, thought-persistence backpressure, native app) so callers that
/// only need the ledger have a single entry point; `health_response` keeps its
/// already-fetched snapshots and calls the lower-level `dependency_ledger`
/// directly to avoid re-reading them.
pub async fn build_dependency_ledger(state: &Arc<AppState>) -> DependencyHealthLedger {
    let thought_bridge = state.bridge_health.snapshot();
    let persistence = state
        .current_file_store()
        .map(|store| store.health_snapshot());
    let thought_persistence = state.supervisor.thought_persistence_backpressure_snapshot();
    let native_app = *state.native_desktop_app.read().await;
    dependency_ledger(
        state,
        &thought_bridge,
        persistence,
        &thought_persistence,
        native_app,
    )
}

fn dependency_ledger(
    state: &Arc<AppState>,
    thought_bridge: &BridgeHealthSnapshot,
    persistence: Option<PersistenceHealthSnapshot>,
    thought_persistence: &ThoughtPersistenceBackpressureSnapshot,
    native_app: crate::types::NativeDesktopApp,
) -> DependencyHealthLedger {
    let tmux = state.supervisor.tmux_dependency_health_snapshot();
    DependencyHealthLedger {
        tmux_discovery: tmux.discovery,
        tmux_capture: tmux.capture,
        persistence: persistence_dependency_health(persistence, thought_persistence),
        thought_bridge: thought_bridge_dependency_health(thought_bridge),
        native_scripts: crate::native::script_dependency_health(native_app),
        overlay: default_overlay_health(),
        remote_targets: remote_targets_health(),
    }
}

fn persistence_dependency_health(
    snapshot: Option<PersistenceHealthSnapshot>,
    thought_persistence: &ThoughtPersistenceBackpressureSnapshot,
) -> DependencyHealthSnapshot {
    let now = Utc::now();
    let mut health = match snapshot {
        Some(snapshot) if snapshot.ok => DependencyHealthSnapshot::healthy(now)
            .with_detail("available", "true")
            .with_detail("load_outcomes", snapshot.load_outcomes.len().to_string()),
        Some(snapshot) => {
            let mut health = DependencyHealthSnapshot::degraded(
                now,
                sanitize_health_error(snapshot.last_error.as_deref().unwrap_or("degraded")),
            )
            .with_detail("available", "true")
            .with_detail("load_outcomes", snapshot.load_outcomes.len().to_string())
            .with_detail(
                "consecutive_failures",
                snapshot.consecutive_failures.to_string(),
            );
            health.last_seen_at = snapshot.last_success_at;
            health.freshness_ms = snapshot
                .last_success_at
                .and_then(|seen| now.signed_duration_since(seen).to_std().ok())
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64);
            health.last_error_at = snapshot.last_failure_at;
            health
        }
        None => DependencyHealthSnapshot::unavailable(now, "file store unavailable")
            .with_detail("available", "false"),
    };

    health = health
        .with_detail(
            "thought_queue_capacity",
            thought_persistence.queue_capacity.to_string(),
        )
        .with_detail(
            "thought_queue_depth",
            thought_persistence.queue_depth.to_string(),
        )
        .with_detail(
            "thought_pending_count",
            thought_persistence.pending_count.to_string(),
        )
        .with_detail(
            "thought_overflow_slots",
            thought_persistence.overflow_slots.to_string(),
        )
        .with_detail(
            "thought_queue_full_count",
            thought_persistence.queue_full_count.to_string(),
        )
        .with_detail(
            "thought_coalesced_count",
            thought_persistence.coalesced_count.to_string(),
        )
        .with_detail(
            "thought_dropped_count",
            thought_persistence.dropped_count.to_string(),
        );
    if thought_persistence.overflow_slots > 0
        && matches!(health.status, DependencyHealthStatus::Healthy)
    {
        health.status = DependencyHealthStatus::Degraded;
        health.last_error_at = Some(now);
        health.last_error = Some("thought persistence backpressure".to_string());
    }
    health
}

fn thought_bridge_dependency_health(snapshot: &BridgeHealthSnapshot) -> DependencyHealthSnapshot {
    let now = Utc::now();
    let status = match snapshot.status {
        BridgeStatus::Starting => DependencyHealthStatus::Unknown,
        BridgeStatus::Healthy => DependencyHealthStatus::Healthy,
        BridgeStatus::Degraded => DependencyHealthStatus::Degraded,
        BridgeStatus::Unhealthy => DependencyHealthStatus::Unavailable,
    };
    let mut health = match status {
        DependencyHealthStatus::Healthy => DependencyHealthSnapshot::healthy(now),
        DependencyHealthStatus::Degraded => DependencyHealthSnapshot::degraded(
            now,
            sanitize_health_error(
                snapshot
                    .last_backend_error
                    .as_deref()
                    .or(snapshot.last_error.as_deref())
                    .unwrap_or("degraded"),
            ),
        ),
        DependencyHealthStatus::Unavailable => DependencyHealthSnapshot::unavailable(
            now,
            sanitize_health_error(snapshot.last_error.as_deref().unwrap_or("unhealthy")),
        ),
        _ => DependencyHealthSnapshot::unknown(now),
    }
    .with_detail(
        "consecutive_failures",
        snapshot.consecutive_failures.to_string(),
    )
    .with_detail("tick_ms", snapshot.tick_ms.to_string());
    health.last_seen_at = snapshot.last_success_at;
    health.freshness_ms = snapshot
        .last_success_at
        .and_then(|seen| now.signed_duration_since(seen).to_std().ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64);
    health.last_error_at = snapshot.last_failure_at;
    health
}

const MAX_HEALTH_ERROR_LEN: usize = 512;

fn sanitize_health_error(error: &str) -> String {
    truncate_health_error(redact_health_error_values(
        error,
        ["AUTH_TOKEN", "OBSERVER_TOKEN"]
            .into_iter()
            .filter_map(env_value),
    ))
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn redact_health_error_values<I, S>(error: &str, values: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    values
        .into_iter()
        .fold(error.to_string(), |sanitized, value| {
            let value = value.as_ref();
            if value.is_empty() {
                sanitized
            } else {
                sanitized.replace(value, "<redacted>")
            }
        })
}

fn truncate_health_error(mut error: String) -> String {
    if error.len() > MAX_HEALTH_ERROR_LEN {
        error.truncate(MAX_HEALTH_ERROR_LEN);
        error.push_str("...");
    }
    error
}

// ---------------------------------------------------------------------------
// GET /version
// ---------------------------------------------------------------------------

async fn version() -> Json<VersionResponse> {
    Json(VersionResponse {
        name: "swimmers",
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn health_router() -> Router<Arc<AppState>> {
    // Ensure the start time is captured at router construction so /health
    // reports a sensible uptime even on the very first request.
    let _ = process_start();
    Router::new()
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route("/version", get(version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;

    use crate::config::Config;
    use crate::host_actions::RepoActionTracker;
    use crate::persistence::file_store::FileStore;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::health::BridgeHealthState;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    fn test_state_with_store(
        bridge_health: Arc<BridgeHealthState>,
        file_store: Option<Arc<FileStore>>,
    ) -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(file_store),
            bridge_health,
            published_selection: Arc::new(RwLock::new(crate::api::PublishedSelectionState {
                session_id: None,
                published_at: Some(Utc::now()),
            })),
            repo_actions: RepoActionTracker::default(),
        })
    }

    fn test_state(bridge_health: Arc<BridgeHealthState>) -> Arc<AppState> {
        test_state_with_store(bridge_health, None)
    }

    #[tokio::test]
    async fn health_returns_bridge_snapshot_and_uptime() {
        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);
        let response = health(State(test_state(bridge_health)))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["status"], "healthy");
        assert!(
            json["uptime_secs"].is_u64(),
            "uptime_secs should be a u64, got {:?}",
            json["uptime_secs"]
        );
        assert_eq!(json["thought_bridge"]["status"], "healthy");
        assert_eq!(json["thought_bridge"]["tick_ms"], 15_000);
        assert!(
            json["persistence"].is_object(),
            "persistence field should be present"
        );
        assert_eq!(
            json["persistence"]["ok"], false,
            "persistence.ok should be false without a file store"
        );
        assert_eq!(json["persistence"]["available"], false);
        assert_eq!(json["persistence"]["consecutive_failures"], 0);
        assert!(
            json["dependencies"].is_object(),
            "dependencies ledger should be present"
        );
        assert_eq!(json["dependencies"]["persistence"]["status"], "unavailable");
        assert_eq!(json["thought_persistence"]["queue_depth"], 0);
        assert_eq!(json["thought_persistence"]["overflow_slots"], 0);
        assert_eq!(
            json["dependencies"]["persistence"]["details"]["thought_queue_depth"],
            "0"
        );
        assert_eq!(json["dependencies"]["thought_bridge"]["status"], "healthy");
        assert!(json["dependencies"]["native_scripts"]["last_checked_at"].is_string());
    }

    #[tokio::test]
    async fn health_reports_persistence_ok_when_file_store_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");

        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);

        let state = test_state_with_store(bridge_health, Some(store));

        let response = health(State(state)).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["persistence"]["available"], true);
        assert_eq!(json["persistence"]["ok"], true);
        assert_eq!(json["persistence"]["consecutive_failures"], 0);
        assert_eq!(
            json["persistence"]["load_outcomes"]["session_registry"]["status"],
            "missing"
        );
        assert_eq!(json["dependencies"]["persistence"]["status"], "healthy");
        assert_eq!(
            json["dependencies"]["persistence"]["details"]["thought_queue_full_count"],
            "0"
        );
    }

    #[tokio::test]
    async fn health_marks_persistence_dependency_degraded_for_thought_backpressure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");

        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);

        let state = test_state_with_store(bridge_health, Some(store));
        state
            .supervisor
            .set_thought_persistence_backpressure_for_test(1, 1, 3, 2, 0);

        let response = health(State(state)).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;

        assert_eq!(json["persistence"]["ok"], true);
        assert_eq!(json["thought_persistence"]["queue_depth"], 1);
        assert_eq!(json["thought_persistence"]["overflow_slots"], 1);
        assert_eq!(json["thought_persistence"]["queue_full_count"], 3);
        assert_eq!(json["thought_persistence"]["coalesced_count"], 2);
        assert_eq!(json["dependencies"]["persistence"]["status"], "degraded");
        assert_eq!(
            json["dependencies"]["persistence"]["last_error"],
            "thought persistence backpressure"
        );
        assert_eq!(
            json["dependencies"]["persistence"]["details"]["thought_overflow_slots"],
            "1"
        );
        assert_eq!(
            json["dependencies"]["persistence"]["details"]["thought_coalesced_count"],
            "2"
        );
    }

    #[tokio::test]
    async fn health_reports_persistence_write_failure_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");
        std::fs::create_dir(dir.path().join("session_registry.json"))
            .expect("create directory at registry path");
        store.save_sessions(&[]).await;

        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);
        let state = test_state_with_store(bridge_health, Some(store));

        let response = health(State(state.clone())).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["persistence"]["available"], true);
        assert_eq!(json["persistence"]["ok"], false);
        assert_eq!(json["persistence"]["consecutive_failures"], 1);
        assert_eq!(
            json["persistence"]["last_failed_operation"],
            "session_registry"
        );
        assert!(json["persistence"]["last_failure_at"].is_string());
        assert!(json["persistence"]["last_error"].is_string());
        assert_eq!(json["dependencies"]["persistence"]["status"], "degraded");

        let readyz_response = readyz(State(state)).await.into_response();
        assert_eq!(readyz_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn health_reports_persistence_load_decode_failure_without_erasing_outcome() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::write(dir.path().join("session_registry.json"), "{not json")
            .await
            .expect("write corrupt registry");
        let store = FileStore::new(dir.path()).await.expect("file store");

        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);
        let state = test_state_with_store(bridge_health, Some(store));

        let response = health(State(state.clone())).await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["persistence"]["available"], true);
        assert_eq!(json["persistence"]["ok"], false);
        assert_eq!(
            json["persistence"]["load_outcomes"]["session_registry"]["status"],
            "decode_failed"
        );
        assert!(json["persistence"]["load_outcomes"]["session_registry"]["last_error"].is_string());
        assert_eq!(json["dependencies"]["persistence"]["status"], "degraded");

        let readyz_response = readyz(State(state)).await.into_response();
        assert_eq!(readyz_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn dependency_status_label_matches_serde_wire_encoding() {
        for status in [
            DependencyHealthStatus::Unknown,
            DependencyHealthStatus::Healthy,
            DependencyHealthStatus::Degraded,
            DependencyHealthStatus::Unavailable,
            DependencyHealthStatus::NotConfigured,
        ] {
            let serialized = serde_json::to_value(status).expect("serialize status");
            assert_eq!(
                Value::String(dependency_status_label(status).to_string()),
                serialized,
                "label must match the snake_case serde encoding for {status:?}"
            );
        }
    }

    #[test]
    fn sanitize_health_error_redacts_nonempty_token_values() {
        let error = "AUTH_TOKEN=writer-token OBSERVER_TOKEN=observer-token";

        assert_eq!(
            redact_health_error_values(error, ["writer-token", "", "observer-token"]),
            "AUTH_TOKEN=<redacted> OBSERVER_TOKEN=<redacted>"
        );
    }

    #[test]
    fn sanitize_health_error_truncates_long_messages() {
        let error = "x".repeat(MAX_HEALTH_ERROR_LEN + 8);
        let sanitized = truncate_health_error(error);

        assert_eq!(sanitized.len(), MAX_HEALTH_ERROR_LEN + 3);
        assert!(sanitized.ends_with("..."));
    }

    #[tokio::test]
    async fn build_dependency_ledger_reports_complete_set() {
        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);
        let state = test_state(bridge_health);

        let ledger = build_dependency_ledger(&state).await;
        // The unified builder must populate the full ledger — including the
        // persistence/thought_bridge/overlay entries the embedded surface used
        // to omit — so both surfaces report the same dependency set.
        let json = serde_json::to_value(&ledger).expect("serialize ledger");
        for key in [
            "tmux_discovery",
            "tmux_capture",
            "persistence",
            "thought_bridge",
            "native_scripts",
            "overlay",
            "remote_targets",
        ] {
            assert!(json.get(key).is_some(), "ledger should expose {key}");
        }
        assert_eq!(
            json["thought_bridge"]["status"],
            dependency_status_label(ledger.thought_bridge.status)
        );
    }

    #[tokio::test]
    async fn version_returns_name_and_cargo_pkg_version() {
        let response = version().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["name"], "swimmers");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn readyz_returns_service_unavailable_when_bridge_is_starting() {
        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        let response = readyz(State(test_state(bridge_health)))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json = response_json(response).await;
        assert_eq!(json["status"], "starting");
        assert_eq!(json["thought_bridge"]["status"], "starting");
    }

    #[tokio::test]
    async fn health_stays_available_while_readyz_reports_unhealthy_bridge() {
        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_failure("timeout 1", Duration::from_secs(1));
        bridge_health.record_failure("timeout 2", Duration::from_secs(1));
        bridge_health.record_failure("timeout 3", Duration::from_secs(1));
        let state = test_state(bridge_health);

        let health_response = health(State(state.clone())).await.into_response();
        assert_eq!(health_response.status(), StatusCode::OK);
        let health_json = response_json(health_response).await;
        assert_eq!(health_json["status"], "unhealthy");
        assert_eq!(health_json["thought_bridge"]["status"], "unhealthy");

        let readyz_response = readyz(State(state)).await.into_response();
        assert_eq!(readyz_response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let readyz_json = response_json(readyz_response).await;
        assert_eq!(readyz_json["status"], "unhealthy");
        assert_eq!(readyz_json["thought_bridge"]["status"], "unhealthy");
    }
}
