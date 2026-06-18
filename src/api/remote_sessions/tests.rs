use super::*;
use crate::types::{
    CreateSessionsBatchResult, SessionBatchMembership, SessionGroupInputRequest,
    SessionGroupInputResponse, SessionGroupInputResult, SessionInputRequest, SessionInputResponse,
    SessionState, SessionTimelinePinned, SessionTimelineResponse, SpawnTool, ThoughtState,
    TransportHealth, SUMMARY_CAUSE_REMOTE_POLL_DEGRADED,
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
        environments: Vec::new(),
        fleet_lens: Default::default(),
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
        RemoteSmokeScope::Operator | RemoteSmokeScope::Observer => AxumJson(SessionListResponse {
            sessions: vec![summary("sess_list")],
            version: 0,
            repo_themes: Default::default(),
            environments: Vec::new(),
            fleet_lens: Default::default(),
        })
        .into_response(),
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

async fn remote_smoke_send_input(
    axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
    headers: HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    AxumJson(body): AxumJson<SessionInputRequest>,
) -> Response {
    let path = format!("/v1/sessions/{session_id}/input");
    state
        .capture(
            "POST",
            path,
            &headers,
            serde_json::to_value(&body).expect("serialize input body"),
        )
        .await;
    if remote_smoke_scope(&headers) != RemoteSmokeScope::Operator {
        return remote_smoke_auth_error(&headers, "operator");
    }
    AxumJson(SessionInputResponse {
        ok: true,
        session_id,
        delivered: true,
        delivery_method: Some("remote-test".to_string()),
        message: None,
    })
    .into_response()
}

async fn remote_smoke_group_input(
    axum::extract::State(state): axum::extract::State<RemoteSmokeState>,
    headers: HeaderMap,
    AxumJson(body): AxumJson<SessionGroupInputRequest>,
) -> Response {
    state
        .capture(
            "POST",
            "/v1/sessions/group-input",
            &headers,
            serde_json::to_value(&body).expect("serialize group input body"),
        )
        .await;
    if remote_smoke_scope(&headers) != RemoteSmokeScope::Operator {
        return remote_smoke_auth_error(&headers, "operator");
    }
    let results = body
        .session_ids
        .into_iter()
        .map(|session_id| SessionGroupInputResult {
            session_id,
            ok: true,
            error: None,
        })
        .collect();
    AxumJson(SessionGroupInputResponse::from_results(results)).into_response()
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

async fn spawn_remote_smoke_server() -> (String, tokio::task::JoinHandle<()>, RemoteSmokeState) {
    let state = RemoteSmokeState::default();
    let app = Router::new()
        .route(
            "/v1/sessions",
            get(remote_smoke_list_sessions).post(remote_smoke_create_session),
        )
        .route("/v1/sessions/batch", post(remote_smoke_create_batch))
        .route(
            "/v1/sessions/{session_id}/input",
            post(remote_smoke_send_input),
        )
        .route("/v1/sessions/group-input", post(remote_smoke_group_input))
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
fn require_batch_dirs_rejects_empty_dirs() {
    let err = require_batch_dirs(Vec::new()).expect_err("empty batch dirs should be invalid");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "VALIDATION_FAILED");
    assert_eq!(err.message(), "dirs must not be empty");
}

#[test]
fn map_batch_cwds_for_target_maps_each_dir() {
    let mapped = map_batch_cwds_for_target(
        &target(),
        &[
            "/workspace/repos/opensource/swimmers".to_string(),
            "/workspace/repos/tools".to_string(),
        ],
    )
    .expect("mapped batch dirs");

    assert_eq!(
        mapped,
        vec![
            "/monoserver/opensource/swimmers".to_string(),
            "/monoserver/tools".to_string(),
        ]
    );
}

#[test]
fn restore_original_batch_cwds_uses_result_index() {
    let original_dirs = vec![
        "/workspace/repos/first".to_string(),
        "/workspace/repos/second".to_string(),
    ];
    let mut response = CreateSessionsBatchResponse {
        results: vec![
            CreateSessionsBatchResult {
                index: 1,
                cwd: "/monoserver/second".to_string(),
                ok: true,
                session: None,
                repo_theme: None,
                error: None,
            },
            CreateSessionsBatchResult {
                index: 9,
                cwd: "/monoserver/unmatched".to_string(),
                ok: false,
                session: None,
                repo_theme: None,
                error: None,
            },
        ],
    };

    restore_original_batch_cwds(&mut response, &original_dirs);

    assert_eq!(response.results[0].cwd, "/workspace/repos/second");
    assert_eq!(response.results[1].cwd, "/monoserver/unmatched");
}

#[test]
fn remote_transcript_query_omits_blank_turn_and_keeps_numeric_params() {
    assert_eq!(
        remote_transcript_query(Some("  "), Some(42), Some(100)),
        vec![
            ("after".to_string(), "42".to_string()),
            ("limit".to_string(), "100".to_string()),
        ]
    );

    assert_eq!(
        remote_transcript_query(Some("turn-7"), None, Some(20)),
        vec![
            ("turn_id".to_string(), "turn-7".to_string()),
            ("limit".to_string(), "20".to_string()),
        ]
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
    assert_eq!(
        session.environment.scope,
        crate::types::SessionEnvironmentScope::Remote
    );
    assert_eq!(session.environment.target_id, "jeremy-skillbox");
    assert_eq!(session.environment.target_label, "Jeremy Skillbox");
    assert_eq!(
        session.environment.remote_session_id.as_deref(),
        Some("sess_0")
    );
    assert_eq!(
        session.environment.remote_cwd.as_deref(),
        Some("/monoserver/opensource/swimmers")
    );
    assert_eq!(
        session.environment.local_cwd.as_deref(),
        Some("/workspace/repos/opensource/swimmers")
    );
    assert_eq!(
        session.environment.canonical_cwd.as_deref(),
        Some("/workspace/repos/opensource/swimmers")
    );
}

#[test]
fn namespace_session_summary_preserves_current_target_namespace_only() {
    let target = target();

    let same_target = namespace_session_summary(&target, summary("jeremy-skillbox::sess_nested"));
    assert_eq!(same_target.session_id, "jeremy-skillbox::sess_nested");
    assert_eq!(
        same_target.environment.remote_session_id.as_deref(),
        Some("sess_nested")
    );

    let other_target = namespace_session_summary(&target, summary("other-target::sess_nested"));
    assert_eq!(
        other_target.session_id,
        "jeremy-skillbox::other-target::sess_nested"
    );
    assert_eq!(
        split_remote_session_id(&other_target.session_id),
        Some(("jeremy-skillbox", "other-target::sess_nested"))
    );
    assert_eq!(
        other_target.environment.remote_session_id.as_deref(),
        Some("other-target::sess_nested")
    );
}

#[test]
fn namespace_session_summary_matches_full_target_id_with_separator() {
    let mut target = target();
    target.id = "zone::west".to_string();
    target.label = "West Zone".to_string();

    let same_target = namespace_session_summary(&target, summary("zone::west::sess_nested"));
    assert_eq!(same_target.session_id, "zone::west::sess_nested");
    assert_eq!(
        same_target.environment.remote_session_id.as_deref(),
        Some("sess_nested")
    );

    let other_target = namespace_session_summary(&target, summary("zone::east::sess_nested"));
    assert_eq!(
        other_target.session_id,
        "zone::west::zone::east::sess_nested"
    );
    assert_eq!(
        other_target.environment.remote_session_id.as_deref(),
        Some("zone::east::sess_nested")
    );
}

#[test]
fn environment_summary_redacts_token_values_and_credentialed_base_url() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var("SWIMMERS_REMOTE_TEST_TOKEN", "secret-token");
    let mut target = target();
    target.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());
    target.base_url =
        Some("http://secret-token@127.0.0.1:3210/?token=secret-token#secret-token".to_string());

    let environment = environment_summary_for_target(&target);
    let json = serde_json::to_string(&environment).expect("serialize environment");

    assert_eq!(environment.auth.mode, "token_env");
    assert_eq!(environment.auth.token_env_present, Some(true));
    assert_eq!(environment.path_mapping_count, 2);
    assert!(!json.contains("secret-token"));
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
    assert_eq!(
        environment.base_url.as_deref(),
        Some("http://127.0.0.1:3210/")
    );

    std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
}

#[test]
fn remote_targets_health_reports_cached_degraded_target_without_secret_values() {
    reset_remote_target_session_cache_for_tests();
    let remote = target();
    record_remote_poll_success(&remote.id, &[summary("sess_0")]);
    let stale = record_remote_poll_failure(&remote.id, "REMOTE_SESSION_LIST_FAILED");
    assert_eq!(stale.len(), 1);

    let health = remote_targets_health_snapshot_for_targets(vec![
        LaunchTargetSummary::local(),
        remote.clone(),
    ]);
    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(health.details["configured_targets"], "1");
    assert_eq!(health.details["degraded_targets"], "1");
    assert_eq!(health.details["probe"], "session_list_cache");
    assert!(health.last_seen_at.is_some());
    assert_eq!(
        health.last_error.as_deref(),
        Some("REMOTE_SESSION_LIST_FAILED")
    );

    let environment = environment_summary_for_target(&remote);
    assert_eq!(environment.status, DependencyHealthStatus::Degraded);
    assert_eq!(
        environment.last_error.as_deref(),
        Some("REMOTE_SESSION_LIST_FAILED")
    );
}

#[test]
fn remote_targets_health_reports_auth_and_mapping_doctor_without_env_names() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_remote_target_session_cache_for_tests();
    std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
    let mut remote = target();
    remote.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());
    remote.path_mappings = Vec::new();

    let health = remote_targets_health_snapshot_for_targets(vec![remote]);
    assert_eq!(health.status, DependencyHealthStatus::Unavailable);
    assert_eq!(health.details["auth_env_missing"], "1");
    assert_eq!(health.details["targets_without_path_mappings"], "1");
    assert_eq!(health.last_error.as_deref(), Some("auth_env_missing"));

    let json = serde_json::to_string(&health).expect("health json");
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
}

#[test]
fn unmapped_launch_target_cwd_returns_stable_guidance() {
    let err = map_cwd_for_target(&target(), "/elsewhere/swimmers").expect_err("unmapped cwd");
    assert_eq!(err.code(), "LAUNCH_TARGET_PATH_UNMAPPED");
    assert!(err.message().contains("add a path_mappings entry"));
}

#[test]
fn denamespace_uses_longest_configured_target_prefix() {
    let mut short = target();
    short.id = "zone".to_string();
    short.label = "Zone".to_string();

    let mut long = target();
    long.id = "zone::west".to_string();
    long.label = "West Zone".to_string();

    let (target, remote_session_id) =
        denamespace_for_configured_targets("zone::west::other::sess", &[short, long])
            .expect("denamespace succeeds")
            .expect("remote session");

    assert_eq!(target.id, "zone::west");
    assert_eq!(remote_session_id, "other::sess");
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

#[test]
fn remote_polling_test_gate_requires_explicit_enablement() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("SWIMMERS_TEST_ENABLE_REMOTE_POLLING");
    assert!(!remote_polling_enabled_for_environment());

    std::env::set_var("SWIMMERS_TEST_ENABLE_REMOTE_POLLING", "1");
    assert!(remote_polling_enabled_for_environment());
    std::env::remove_var("SWIMMERS_TEST_ENABLE_REMOTE_POLLING");
}

#[test]
fn remote_poll_targets_keeps_only_swimmers_api_targets() {
    let config = Config::default();
    let mut swimmers = target();
    swimmers.id = "swimmers".to_string();
    swimmers.base_url = Some("http://remote.example:3210".to_string());

    let mut unsupported = target();
    unsupported.id = "ssh".to_string();
    unsupported.kind = "ssh".to_string();
    unsupported.base_url = Some("http://other.example:3210".to_string());

    let targets = remote_poll_targets(vec![unsupported, swimmers], &config);

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "swimmers");
}

#[test]
fn remote_poll_targets_skips_current_server_target() {
    let mut config = Config::default();
    config.bind = "127.0.0.1".to_string();
    config.port = 3210;

    let mut self_target = target();
    self_target.id = "self".to_string();
    self_target.base_url = Some("http://localhost:3210".to_string());

    let mut remote_target = target();
    remote_target.id = "remote".to_string();
    remote_target.base_url = Some("http://remote.example:3210".to_string());

    let targets = remote_poll_targets(vec![self_target, remote_target], &config);

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "remote");
}

#[test]
fn remote_poll_targets_returns_empty_when_no_pollable_targets_remain() {
    let mut config = Config::default();
    config.bind = "127.0.0.1".to_string();
    config.port = 3210;

    let mut self_target = target();
    self_target.base_url = Some("http://localhost:3210".to_string());

    let mut unsupported = target();
    unsupported.kind = "ssh".to_string();

    let targets = remote_poll_targets(vec![self_target, unsupported], &config);

    assert!(targets.is_empty());
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
async fn send_remote_input_posts_denamespaced_session_and_namespaces_response() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var(REMOTE_OPERATOR_TOKEN_ENV, REMOTE_OPERATOR_TOKEN);
    let (base_url, handle, state) = spawn_remote_smoke_server().await;
    let target = remote_smoke_target(&base_url, REMOTE_OPERATOR_TOKEN_ENV);

    let response = send_remote_input(
        &target,
        "sess/input?x#frag",
        SessionInputRequest {
            text: "status".to_string(),
            submit: true,
        },
    )
    .await
    .expect("remote input");

    assert_eq!(
        response.session_id,
        namespace_session_id("jeremy-skillbox", "sess/input?x#frag")
    );
    assert!(response.delivered);
    assert_eq!(response.delivery_method.as_deref(), Some("remote-test"));
    let requests = state.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| {
            request.method == "POST" && request.path == "/v1/sessions/sess/input?x#frag/input"
        })
        .expect("remote input request");
    assert_eq!(
        request.auth.as_deref(),
        Some(format!("Bearer {REMOTE_OPERATOR_TOKEN}").as_str())
    );
    assert_eq!(request.body["text"], "status");
    assert_eq!(request.body["submit"], true);
    drop(requests);
    handle.abort();
    std::env::remove_var(REMOTE_OPERATOR_TOKEN_ENV);
}

#[tokio::test]
async fn send_remote_group_input_denamespaces_request_and_namespaces_results() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var(REMOTE_OPERATOR_TOKEN_ENV, REMOTE_OPERATOR_TOKEN);
    let (base_url, handle, state) = spawn_remote_smoke_server().await;
    let target = remote_smoke_target(&base_url, REMOTE_OPERATOR_TOKEN_ENV);

    let response = send_remote_group_input(
        &target,
        vec!["sess-a".to_string(), "sess-b".to_string()],
        "continue".to_string(),
    )
    .await
    .expect("remote group input");

    assert_eq!(response.delivered, 2);
    assert_eq!(response.skipped, 0);
    assert_eq!(
        response.results[0].session_id,
        namespace_session_id("jeremy-skillbox", "sess-a")
    );
    assert_eq!(
        response.results[1].session_id,
        namespace_session_id("jeremy-skillbox", "sess-b")
    );
    let requests = state.requests.lock().await;
    let request = requests
        .iter()
        .find(|request| request.method == "POST" && request.path == "/v1/sessions/group-input")
        .expect("remote group input request");
    assert_eq!(
        request.body["session_ids"],
        serde_json::json!(["sess-a", "sess-b"])
    );
    assert_eq!(request.body["text"], "continue");
    drop(requests);
    handle.abort();
    std::env::remove_var(REMOTE_OPERATOR_TOKEN_ENV);
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
    assert_eq!(stale[0].environment.target_id, "jeremy-skillbox");
    assert_eq!(
        stale[0].environment.remote_session_id.as_deref(),
        Some("sess_1")
    );
    assert_eq!(
        stale[0].environment.scope,
        crate::types::SessionEnvironmentScope::Remote
    );

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
