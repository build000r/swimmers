use super::*;
use crate::types::{
    CreateSessionsBatchResult, DirEntry, SessionBatchMembership, SessionGroupInputRequest,
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
        bootstrap_hint: None,
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

fn shared_remote_cache_test_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

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
            session: Some(summary("sess_0")),
            repo_theme: None,
            launch_receipt: None,
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
            session: Some(summary("sess_create")),
            repo_theme: None,
            launch_receipt: None,
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
            launch_receipt: None,
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

fn remote_health_test_config() -> Config {
    let mut config = Config::default();
    config.port = 43210;
    config
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
fn map_path_keeps_first_equal_specificity_mapping() {
    let mappings = vec![
        LaunchPathMapping {
            local_prefix: "/workspace/repos".to_string(),
            remote_prefix: "/primary".to_string(),
        },
        LaunchPathMapping {
            local_prefix: "/workspace/./repos".to_string(),
            remote_prefix: "/duplicate".to_string(),
        },
    ];

    let mapped = map_path_with_mappings("/workspace/repos/swimmers", &mappings).expect("mapped");

    assert_eq!(mapped, "/primary/swimmers");
}

#[test]
fn map_path_ignores_empty_mapping_prefixes() {
    let mappings = vec![
        LaunchPathMapping {
            local_prefix: String::new(),
            remote_prefix: "/remote".to_string(),
        },
        LaunchPathMapping {
            local_prefix: "/workspace/repos".to_string(),
            remote_prefix: "   ".to_string(),
        },
    ];

    assert!(map_path_with_mappings("/workspace/repos/swimmers", &mappings).is_none());
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
fn require_batch_dirs_rejects_oversized_batches() {
    let dirs = (0..=crate::api::service::BATCH_CREATE_MAX_DIRS)
        .map(|index| format!("/workspace/repos/project-{index}"))
        .collect::<Vec<_>>();

    let err = require_batch_dirs(dirs).expect_err("oversized batch dirs should be invalid");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "VALIDATION_FAILED");
    assert_eq!(
        err.message(),
        format!(
            "dirs must include at most {} entries",
            crate::api::service::BATCH_CREATE_MAX_DIRS
        )
    );
}

#[test]
fn require_batch_dirs_rejects_blank_dirs() {
    let err = require_batch_dirs(vec![
        "/workspace/repos/project".to_string(),
        " \t\n".to_string(),
    ])
    .expect_err("blank batch dirs should be invalid");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "VALIDATION_FAILED");
    assert_eq!(err.message(), "dirs must not include blank entries");
}

#[test]
fn map_batch_cwds_for_targets_maps_each_dir() {
    let target = target();
    let mapped = map_batch_cwds_for_targets(
        &[target.clone(), target],
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
fn prepare_remote_sessions_batch_maps_each_dir_with_cwd_scoped_target() {
    let mut first = target();
    first.path_mappings = vec![LaunchPathMapping {
        local_prefix: "/workspace/repos/personal".to_string(),
        remote_prefix: "/monoserver/personal".to_string(),
    }];
    let mut second = target();
    second.path_mappings = vec![LaunchPathMapping {
        local_prefix: "/workspace/repos/opensource".to_string(),
        remote_prefix: "/monoserver/opensource".to_string(),
    }];
    let dirs = vec![
        "/workspace/repos/personal/app".to_string(),
        "/workspace/repos/opensource/swimmers".to_string(),
    ];

    let batch = prepare_remote_sessions_batch_with_resolver(
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(SpawnTool::Codex),
            launch_target: Some("jeremy-skillbox".to_string()),
            initial_request: Some("continue".to_string()),
        },
        |cwd, target_id| {
            assert_eq!(target_id, "jeremy-skillbox");
            if cwd.contains("/personal/") {
                Ok(first.clone())
            } else {
                Ok(second.clone())
            }
        },
    )
    .expect("prepared batch");

    assert_eq!(batch.target.base_url, target().base_url);
    assert_eq!(
        batch.remote_body.dirs,
        vec![
            "/monoserver/personal/app".to_string(),
            "/monoserver/opensource/swimmers".to_string(),
        ]
    );
    assert_eq!(batch.remote_body.launch_target, None);
    assert_eq!(batch.remote_body.spawn_tool, Some(SpawnTool::Codex));
    assert_eq!(
        batch.remote_body.initial_request.as_deref(),
        Some("continue")
    );
}

#[test]
fn prepare_remote_sessions_batch_rejects_cwd_scoped_targets_with_different_endpoints() {
    let first = target();
    let mut second = target();
    second.base_url = Some("http://127.0.0.1:4321".to_string());
    let dirs = vec![
        "/workspace/repos/personal/app".to_string(),
        "/workspace/repos/opensource/swimmers".to_string(),
    ];

    let err = prepare_remote_sessions_batch_with_resolver(
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(SpawnTool::Codex),
            launch_target: Some("jeremy-skillbox".to_string()),
            initial_request: None,
        },
        |cwd, _target_id| {
            if cwd.contains("/personal/") {
                Ok(first.clone())
            } else {
                Ok(second.clone())
            }
        },
    )
    .expect_err("mixed endpoints should fail");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "LAUNCH_TARGET_MISMATCH");
    assert!(err.message().contains("different remote endpoints"));
}

#[test]
fn restore_original_batch_cwds_uses_result_index() {
    let original_dirs = vec![
        "/workspace/repos/first".to_string(),
        "/workspace/repos/second".to_string(),
    ];
    let mut response = CreateSessionsBatchResponse {
        results: vec![
            batch_result(1, "/monoserver/second"),
            batch_result(0, "/monoserver/first"),
        ],
    };

    restore_original_batch_cwds(&mut response, &original_dirs).expect("restore cwds");

    assert_eq!(response.results[0].cwd, "/workspace/repos/second");
    assert_eq!(response.results[1].cwd, "/workspace/repos/first");
}

#[test]
fn restore_original_batch_cwds_updates_nested_session_cwd() {
    let original_dirs = vec!["/workspace/repos/opensource/swimmers".to_string()];
    let mut result = batch_result(0, "/monoserver/opensource/swimmers");
    result.session = Some(namespace_session_summary(&target(), summary("sess_0")));
    let mut response = CreateSessionsBatchResponse {
        results: vec![result],
    };

    restore_original_batch_cwds(&mut response, &original_dirs).expect("restore cwds");

    let result = response.results.first().expect("result");
    assert_eq!(result.cwd, "/workspace/repos/opensource/swimmers");
    assert_eq!(
        result.session.as_ref().expect("session").cwd,
        "/workspace/repos/opensource/swimmers"
    );
    assert_eq!(
        result
            .session
            .as_ref()
            .expect("session")
            .environment
            .local_cwd
            .as_deref(),
        Some("/workspace/repos/opensource/swimmers")
    );
    assert_eq!(
        result
            .session
            .as_ref()
            .expect("session")
            .environment
            .canonical_cwd
            .as_deref(),
        Some("/workspace/repos/opensource/swimmers")
    );
    assert_eq!(
        result
            .session
            .as_ref()
            .expect("session")
            .environment
            .remote_cwd
            .as_deref(),
        Some("/monoserver/opensource/swimmers")
    );
}

#[test]
fn restore_original_batch_cwds_rejects_result_count_mismatch() {
    let original_dirs = vec![
        "/workspace/repos/first".to_string(),
        "/workspace/repos/second".to_string(),
    ];
    let mut response = CreateSessionsBatchResponse {
        results: vec![batch_result(0, "/monoserver/first")],
    };

    let err = restore_original_batch_cwds(&mut response, &original_dirs)
        .expect_err("missing remote result should fail");

    assert_malformed_remote_batch_error(err, "returned 1 results for 2 requested dirs");
}

#[test]
fn restore_original_batch_cwds_rejects_out_of_range_index() {
    let original_dirs = vec![
        "/workspace/repos/first".to_string(),
        "/workspace/repos/second".to_string(),
    ];
    let mut response = CreateSessionsBatchResponse {
        results: vec![
            batch_result(0, "/monoserver/first"),
            batch_result(9, "/monoserver/unmatched"),
        ],
    };

    let err = restore_original_batch_cwds(&mut response, &original_dirs)
        .expect_err("out-of-range remote result should fail");

    assert_malformed_remote_batch_error(err, "out-of-range result index 9");
}

#[test]
fn restore_original_batch_cwds_rejects_duplicate_index() {
    let original_dirs = vec![
        "/workspace/repos/first".to_string(),
        "/workspace/repos/second".to_string(),
    ];
    let mut response = CreateSessionsBatchResponse {
        results: vec![
            batch_result(0, "/monoserver/first"),
            batch_result(0, "/monoserver/duplicate"),
        ],
    };

    let err = restore_original_batch_cwds(&mut response, &original_dirs)
        .expect_err("duplicate remote result should fail");

    assert_malformed_remote_batch_error(err, "duplicate result index 0");
}

fn batch_result(index: usize, cwd: &str) -> CreateSessionsBatchResult {
    CreateSessionsBatchResult {
        index,
        cwd: cwd.to_string(),
        ok: true,
        launch_receipt: None,
        session: None,
        repo_theme: None,
        error: None,
    }
}

fn assert_malformed_remote_batch_error(err: RemoteSessionError, detail: &str) {
    assert_eq!(err.status, StatusCode::BAD_GATEWAY);
    assert_eq!(err.code, "REMOTE_LAUNCH_FAILED");
    assert!(
        err.message().contains(detail),
        "expected error to contain {detail:?}, got {:?}",
        err.message()
    );
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
    assert_eq!(session.cwd, "/workspace/repos/opensource/swimmers");
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
fn namespace_session_summary_preserves_unmapped_remote_cwd_as_display_cwd() {
    let target = target();
    let mut remote_only = summary("sess_remote_only");
    remote_only.cwd = "/srv/remote-only/project".to_string();

    let session = namespace_session_summary(&target, remote_only);

    assert_eq!(session.cwd, "/srv/remote-only/project");
    assert_eq!(
        session.environment.remote_cwd.as_deref(),
        Some("/srv/remote-only/project")
    );
    assert_eq!(session.environment.local_cwd, None);
    assert_eq!(
        session.environment.canonical_cwd.as_deref(),
        Some("/srv/remote-only/project")
    );
}

#[test]
fn namespace_session_summary_namespaces_target_prefixed_remote_ids() {
    let target = target();

    let same_target = namespace_session_summary(&target, summary("jeremy-skillbox::sess_nested"));
    assert_eq!(
        same_target.session_id,
        "jeremy-skillbox::jeremy-skillbox::sess_nested"
    );
    assert_eq!(
        split_remote_session_id(&same_target.session_id),
        Some(("jeremy-skillbox", "jeremy-skillbox::sess_nested"))
    );
    assert_eq!(
        same_target.environment.remote_session_id.as_deref(),
        Some("jeremy-skillbox::sess_nested")
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
fn namespace_session_summary_namespaces_full_target_id_with_separator() {
    let mut target = target();
    target.id = "zone::west".to_string();
    target.label = "West Zone".to_string();

    let same_target = namespace_session_summary(&target, summary("zone::west::sess_nested"));
    assert_eq!(
        same_target.session_id,
        "zone::west::zone::west::sess_nested"
    );
    assert_eq!(
        same_target.environment.remote_session_id.as_deref(),
        Some("zone::west::sess_nested")
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
fn namespace_response_session_id_preserves_target_prefixed_remote_id_as_payload() {
    let target = target();

    assert_eq!(
        namespace_response_session_id(&target, "jeremy-skillbox::sess_nested"),
        "jeremy-skillbox::jeremy-skillbox::sess_nested"
    );
}

#[test]
fn environment_summary_redacts_token_values_and_credentialed_base_url() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var("SWIMMERS_REMOTE_TEST_TOKEN", "fixture-token");
    let mut target = target();
    target.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());
    target.base_url =
        Some("http://secret-token@127.0.0.1:3210/?token=secret-token#secret-token".to_string());

    let environment = environment_summary_for_target(&target);
    let json = serde_json::to_string(&environment).expect("serialize environment");

    assert_eq!(environment.auth.mode, "token_env");
    assert_eq!(environment.auth.token_env_present, Some(true));
    assert_eq!(environment.path_mapping_count, 2);
    assert!(!json.contains("fixture-token"));
    assert!(!json.contains("secret-token"));
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
    assert_eq!(
        environment.base_url.as_deref(),
        Some("http://127.0.0.1:3210/")
    );

    std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
}

#[test]
fn environment_summary_surfaces_configured_bootstrap_hint_for_down_swimmers_api_target() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    let mut target = target();
    target.base_url = Some("ftp://127.0.0.1:3210".to_string());
    target.bootstrap_hint =
        Some("ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'".to_string());

    let environment = environment_summary_for_target(&target);

    assert_eq!(environment.kind, "swimmers_api");
    assert_eq!(environment.backend_mode, "remote_swimmers_api");
    assert_eq!(environment.status, DependencyHealthStatus::Unavailable);
    assert_eq!(
        environment.last_error.as_deref(),
        Some("base_url_unavailable")
    );
    assert_eq!(environment.attach_hint, None);
    assert_eq!(
        environment.bootstrap_hint.as_deref(),
        Some("ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'")
    );
    assert_eq!(environment.capabilities.bootstrap_hint, true);
    assert_eq!(environment.capabilities.observe_sessions, false);
}

#[test]
fn configured_bootstrap_hint_suppresses_inline_token_values() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var("SWIMMERS_REMOTE_TEST_TOKEN", "fixture-token");
    let mut target = target();
    target.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());

    target.bootstrap_hint = Some(
        "ssh skillbox-devbox 'SWIMMERS_REMOTE_TEST_TOKEN=$SWIMMERS_REMOTE_TEST_TOKEN swimmers serve'"
            .to_string(),
    );
    let safe = environment_summary_for_target(&target);
    assert_eq!(
        safe.bootstrap_hint.as_deref(),
        Some(
            "ssh skillbox-devbox 'SWIMMERS_REMOTE_TEST_TOKEN=$SWIMMERS_REMOTE_TEST_TOKEN swimmers serve'"
        )
    );

    target.bootstrap_hint = Some(
        "ssh skillbox-devbox 'SWIMMERS_REMOTE_TEST_TOKEN=fixture-token swimmers serve'".to_string(),
    );
    let leaked_value = environment_summary_for_target(&target);
    assert_eq!(leaked_value.bootstrap_hint, None);
    assert_eq!(leaked_value.capabilities.bootstrap_hint, false);

    target.bootstrap_hint =
        Some("ssh skillbox-devbox 'AUTH_TOKEN=literal-token swimmers serve'".to_string());
    let inline_assignment = environment_summary_for_target(&target);
    assert_eq!(inline_assignment.bootstrap_hint, None);

    std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
}

#[test]
fn environment_summary_classifies_ssh_only_as_handoff_without_live_capabilities() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::set_var("SWIMMERS_REMOTE_TEST_TOKEN", "secret-token");
    let mut target = target();
    target.id = "skillbox-devbox".to_string();
    target.label = "Skillbox devbox".to_string();
    target.kind = "ssh_only".to_string();
    target.base_url =
        Some("http://secret-token@127.0.0.1:3210/?token=secret-token#secret-token".to_string());
    target.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());

    let environment = environment_summary_for_target(&target);
    let json = serde_json::to_string(&environment).expect("serialize environment");

    assert_eq!(environment.kind, "ssh_only");
    assert_eq!(environment.backend_mode, "ssh_handoff");
    assert_eq!(environment.display_host, "Skillbox devbox");
    assert_eq!(environment.status, DependencyHealthStatus::NotConfigured);
    assert_eq!(environment.auth.mode, "none");
    assert_eq!(environment.auth.token_env_present, None);
    assert_eq!(environment.base_url, None);
    assert_eq!(environment.ssh_alias.as_deref(), Some("skillbox-devbox"));
    assert_eq!(
        environment.attach_hint.as_deref(),
        Some("ssh skillbox-devbox")
    );
    assert_eq!(
        environment.bootstrap_hint.as_deref(),
        Some("ssh skillbox-devbox 'swimmers serve'")
    );
    assert!(!environment.capabilities.observe_sessions);
    assert!(!environment.capabilities.launch_session);
    assert!(!environment.capabilities.send_input);
    assert!(!environment.capabilities.group_input);
    assert!(!environment.capabilities.remote_dir_inventory);
    assert!(environment.capabilities.ssh_attach_hint);
    assert!(environment.capabilities.bootstrap_hint);
    assert!(environment.capabilities.advisory_metadata);
    assert!(!environment.capabilities.health_probe);
    assert!(!json.contains("secret-token"));
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));

    std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
}

#[test]
fn environment_summary_refuses_command_hints_for_unsafe_ssh_alias() {
    let mut target = target();
    target.id = "devbox;rm -rf".to_string();
    target.label = "Unsafe SSH alias".to_string();
    target.kind = "ssh".to_string();

    let environment = environment_summary_for_target(&target);

    assert_eq!(environment.kind, "ssh_only");
    assert_eq!(environment.backend_mode, "ssh_handoff");
    assert_eq!(environment.ssh_alias, None);
    assert_eq!(environment.attach_hint, None);
    assert_eq!(environment.bootstrap_hint, None);
    assert!(!environment.capabilities.ssh_attach_hint);
    assert!(!environment.capabilities.bootstrap_hint);
    assert!(!environment.capabilities.observe_sessions);
}

#[test]
fn remote_targets_health_reports_cached_degraded_target_without_secret_values() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    let remote = target();
    record_remote_poll_success(&remote.id, &[summary("sess_0")]);
    let stale = record_remote_poll_failure(&remote.id, "REMOTE_SESSION_LIST_FAILED");
    assert_eq!(stale.len(), 1);

    let health = remote_targets_health_snapshot_for_targets(
        vec![LaunchTargetSummary::local(), remote.clone()],
        &remote_health_test_config(),
    );
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
fn aggregate_remote_target_status_matrix_preserves_fleet_policy() {
    use DependencyHealthStatus::{Degraded, Healthy, Unavailable, Unknown};

    for (healthy, degraded, unavailable, unknown, doctor_degraded, expected) in [
        (0, 0, 1, 0, false, Unavailable),
        (0, 0, 2, 1, false, Unavailable),
        (0, 0, 1, 1, true, Unavailable),
        (1, 0, 0, 0, false, Healthy),
        (1, 0, 0, 1, false, Unknown),
        (1, 0, 1, 0, false, Degraded),
        (1, 1, 0, 0, false, Degraded),
        (0, 1, 0, 1, false, Degraded),
        (0, 0, 0, 1, false, Unknown),
        (1, 0, 0, 0, true, Degraded),
        (0, 0, 0, 1, true, Degraded),
    ] {
        assert_eq!(
            aggregate_remote_target_status(healthy, degraded, unavailable, unknown, doctor_degraded),
            expected,
            "healthy={healthy} degraded={degraded} unavailable={unavailable} unknown={unknown} doctor_degraded={doctor_degraded}",
        );
    }
}

#[test]
fn remote_targets_health_snapshot_characterizes_mixed_fleets() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();

    let mut healthy = target();
    healthy.id = "healthy".to_string();
    healthy.label = "Healthy".to_string();
    record_remote_poll_success(&healthy.id, &[summary("sess_healthy")]);

    let mut unknown = target();
    unknown.id = "unknown".to_string();
    unknown.label = "Unknown".to_string();

    let health = remote_targets_health_snapshot_for_targets(
        vec![healthy.clone(), unknown.clone()],
        &remote_health_test_config(),
    );
    assert_eq!(health.status, DependencyHealthStatus::Unknown);
    assert_eq!(health.details["healthy_targets"], "1");
    assert_eq!(health.details["unknown_targets"], "1");

    let mut unavailable = target();
    unavailable.id = "unavailable".to_string();
    unavailable.label = "Unavailable".to_string();
    unavailable.base_url = Some("ftp://127.0.0.1:3210".to_string());

    let health = remote_targets_health_snapshot_for_targets(
        vec![healthy.clone(), unavailable],
        &remote_health_test_config(),
    );
    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(health.details["healthy_targets"], "1");
    assert_eq!(health.details["unavailable_targets"], "1");
    assert_eq!(health.last_error.as_deref(), Some("base_url_unavailable"));

    record_remote_poll_failure(&healthy.id, "REMOTE_SESSION_LIST_FAILED");
    let health = remote_targets_health_snapshot_for_targets(
        vec![healthy.clone(), unknown.clone()],
        &remote_health_test_config(),
    );
    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(health.details["degraded_targets"], "1");
    assert_eq!(health.details["unknown_targets"], "1");
    assert_eq!(
        health.last_error.as_deref(),
        Some("REMOTE_SESSION_LIST_FAILED")
    );

    let mut missing_mapping = target();
    missing_mapping.id = "missing-mapping".to_string();
    missing_mapping.label = "Missing Mapping".to_string();
    missing_mapping.path_mappings = Vec::new();

    let health = remote_targets_health_snapshot_for_targets(
        vec![unknown, missing_mapping],
        &remote_health_test_config(),
    );
    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(health.details["unknown_targets"], "2");
    assert_eq!(health.details["targets_without_path_mappings"], "1");
    assert_eq!(
        health.last_error.as_deref(),
        Some("remote target path mapping doctor warning")
    );
}

#[test]
fn remote_targets_health_snapshot_reports_ssh_only_inventory_without_probe() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();

    let mut remote_api = target();
    remote_api.id = "devbox-api".to_string();
    remote_api.label = "Devbox API".to_string();
    record_remote_poll_success(&remote_api.id, &[summary("sess_api")]);

    let mut ssh_only = target();
    ssh_only.id = "skillbox-devbox".to_string();
    ssh_only.label = "Skillbox devbox".to_string();
    ssh_only.kind = "ssh_only".to_string();
    ssh_only.base_url = None;
    ssh_only.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());
    ssh_only.path_mappings = Vec::new();

    let health = remote_targets_health_snapshot_for_targets(
        vec![LaunchTargetSummary::local(), remote_api, ssh_only],
        &remote_health_test_config(),
    );
    let json = serde_json::to_string(&health).expect("health json");

    assert_eq!(health.status, DependencyHealthStatus::Healthy);
    assert_eq!(health.details["configured_targets"], "2");
    assert_eq!(health.details["swimmers_api_targets"], "1");
    assert_eq!(health.details["ssh_only_targets"], "1");
    assert_eq!(health.details["handoff_targets"], "1");
    assert_eq!(health.details["probed_targets"], "1");
    assert_eq!(health.details["attach_hint_missing"], "0");
    assert_eq!(health.details["path_mappings_total"], "2");
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
}

#[test]
fn remote_targets_health_snapshot_warns_when_ssh_only_cannot_make_attach_hint() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();

    let mut ssh_only = target();
    ssh_only.id = "devbox;rm -rf".to_string();
    ssh_only.label = "Unsafe SSH alias".to_string();
    ssh_only.kind = "ssh_only".to_string();
    ssh_only.base_url = Some("http://secret-token@127.0.0.1:3210/?token=secret-token".to_string());
    ssh_only.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());

    let health =
        remote_targets_health_snapshot_for_targets(vec![ssh_only], &remote_health_test_config());
    let json = serde_json::to_string(&health).expect("health json");

    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(
        health.last_error.as_deref(),
        Some("ssh_only_attach_hint_unavailable")
    );
    assert_eq!(health.details["configured_targets"], "1");
    assert_eq!(health.details["ssh_only_targets"], "1");
    assert_eq!(health.details["handoff_targets"], "1");
    assert_eq!(health.details["probed_targets"], "0");
    assert_eq!(health.details["attach_hint_missing"], "1");
    assert_eq!(health.details["auth_env_missing"], "0");
    assert_eq!(health.details["path_mappings_total"], "0");
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
    assert!(!json.contains("secret-token"));
}

#[test]
fn remote_targets_health_snapshot_keeps_first_error_on_equal_timestamps() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();

    let mut first = target();
    first.id = "first-degraded".to_string();
    first.label = "First Degraded".to_string();
    let mut second = target();
    second.id = "second-degraded".to_string();
    second.label = "Second Degraded".to_string();

    record_remote_poll_success(&first.id, &[summary("sess_first")]);
    record_remote_poll_success(&second.id, &[summary("sess_second")]);
    record_remote_poll_failure(&first.id, "FIRST_FAILURE");
    record_remote_poll_failure(&second.id, "SECOND_FAILURE");

    let error_at = Utc::now();
    with_remote_target_session_cache(|cache| {
        let first_entry = cache.get_mut(&first.id).expect("first cache entry");
        first_entry.last_error_at = Some(error_at);
        first_entry.last_error = Some("FIRST_FAILURE".to_string());
        let second_entry = cache.get_mut(&second.id).expect("second cache entry");
        second_entry.last_error_at = Some(error_at);
        second_entry.last_error = Some("SECOND_FAILURE".to_string());
    });

    let health = remote_targets_health_snapshot_for_targets(
        vec![first, second],
        &remote_health_test_config(),
    );
    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(health.last_error.as_deref(), Some("FIRST_FAILURE"));
    assert_eq!(health.last_error_at, Some(error_at));
}

#[test]
fn remote_targets_health_keeps_cached_failure_degraded_after_backoff_expires() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    let remote = target();
    record_remote_poll_success(&remote.id, &[summary("sess_0")]);
    record_remote_poll_failure(&remote.id, "REMOTE_SESSION_LIST_FAILED");
    with_remote_target_session_cache(|cache| {
        cache
            .get_mut(&remote.id)
            .expect("cached target")
            .backoff_until_ms = 0;
    });

    let health = remote_target_environment_health(&remote);

    assert_eq!(health.status, DependencyHealthStatus::Degraded);
    assert_eq!(
        health.last_error.as_deref(),
        Some("REMOTE_SESSION_LIST_FAILED")
    );
}

#[test]
fn remote_targets_health_reports_auth_and_mapping_doctor_without_env_names() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    std::env::remove_var("SWIMMERS_REMOTE_TEST_TOKEN");
    let mut remote = target();
    remote.auth_token_env = Some("SWIMMERS_REMOTE_TEST_TOKEN".to_string());
    remote.path_mappings = Vec::new();

    let health =
        remote_targets_health_snapshot_for_targets(vec![remote], &remote_health_test_config());
    assert_eq!(health.status, DependencyHealthStatus::Unavailable);
    assert_eq!(health.details["auth_env_missing"], "1");
    assert_eq!(health.details["targets_without_path_mappings"], "1");
    assert_eq!(health.last_error.as_deref(), Some("auth_env_missing"));

    let json = serde_json::to_string(&health).expect("health json");
    assert!(!json.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
}

#[test]
fn remote_targets_health_reports_non_http_base_url_as_unavailable() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    let mut remote = target();
    remote.base_url = Some("ftp://127.0.0.1:3210".to_string());

    let health = remote_targets_health_snapshot_for_targets(
        vec![remote.clone()],
        &remote_health_test_config(),
    );

    assert_eq!(health.status, DependencyHealthStatus::Unavailable);
    assert_eq!(health.details["targets_without_base_url"], "1");
    assert_eq!(health.last_error.as_deref(), Some("base_url_unavailable"));

    let environment = environment_summary_for_target(&remote);
    assert_eq!(environment.status, DependencyHealthStatus::Unavailable);
    assert_eq!(environment.base_url, None);
    assert_eq!(
        environment.last_error.as_deref(),
        Some("base_url_unavailable")
    );
}

#[test]
fn remote_targets_health_reports_unsafe_base_url_without_secret_values() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    let mut remote = target();
    remote.base_url =
        Some("http://secret-token@127.0.0.1:3210/?token=secret-token#secret-token".to_string());

    let health =
        remote_targets_health_snapshot_for_targets(vec![remote], &remote_health_test_config());

    assert_eq!(health.status, DependencyHealthStatus::Unavailable);
    assert_eq!(health.details["targets_without_base_url"], "1");
    assert_eq!(health.last_error.as_deref(), Some("base_url_unavailable"));

    let json = serde_json::to_string(&health).expect("health json");
    assert!(!json.contains("secret-token"));
}

#[test]
fn remote_targets_health_skips_current_server_targets() {
    let _guard = shared_remote_cache_test_guard();
    reset_remote_target_session_cache_for_tests();
    let mut config = Config::default();
    config.bind = "127.0.0.1".to_string();
    config.port = 3210;
    let mut self_target = target();
    self_target.id = "self".to_string();
    self_target.base_url = Some("http://localhost:3210".to_string());

    let health = remote_targets_health_snapshot_for_targets(vec![self_target], &config);

    assert_eq!(health.status, DependencyHealthStatus::NotConfigured);
    assert_eq!(health.details["configured_targets"], "0");
    assert_eq!(health.details["skipped_current_server_targets"], "1");
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
fn remote_url_rejects_credentialed_query_or_fragment_base_url() {
    for base_url in [
        "http://token@127.0.0.1:3210",
        "http://user:token@127.0.0.1:3210",
        "http://127.0.0.1:3210?token=secret",
        "http://127.0.0.1:3210#secret",
    ] {
        let mut target = target();
        target.base_url = Some(base_url.to_string());

        let err = remote_url(&target, "/v1/sessions").expect_err("unsafe base_url rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code(), "LAUNCH_TARGET_INVALID");
        assert!(err.message().contains("must not include credentials"));
    }
}

#[test]
fn remote_url_rejects_non_http_base_url() {
    let mut target = target();
    target.base_url = Some("ftp://127.0.0.1:3210".to_string());

    let err = remote_url(&target, "/v1/sessions").expect_err("ftp base_url rejected");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code(), "LAUNCH_TARGET_INVALID");
    assert!(err.message().contains("must use http or https"));
}

#[test]
fn remote_url_preserves_clean_base_path() {
    let mut target = target();
    target.base_url = Some("  http://127.0.0.1:3210/swimmers-api/  ".to_string());

    assert_eq!(
        remote_url(&target, "/v1/sessions/sess%2Fweird/pane-tail").expect("remote url"),
        "http://127.0.0.1:3210/swimmers-api/v1/sessions/sess%2Fweird/pane-tail"
    );
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
fn target_points_at_current_server_ignores_prefixed_base_paths() {
    let mut config = Config::default();
    config.bind = "127.0.0.1".to_string();
    config.port = 3210;
    let mut target = target();
    target.base_url = Some("http://localhost:3210/swimmers-api".to_string());

    assert!(!target_points_at_current_server(&target, &config));
    assert_eq!(
        remote_poll_targets(vec![target], &config)
            .into_iter()
            .map(|target| target.id)
            .collect::<Vec<_>>(),
        vec!["jeremy-skillbox".to_string()]
    );
}

#[test]
fn target_points_at_current_server_matches_wildcard_loopback_aliases() {
    let mut config = Config::default();
    config.bind = "0.0.0.0".to_string();
    config.port = 3210;
    let mut target = target();
    target.base_url = Some("http://127.0.0.1:3210".to_string());

    assert!(target_points_at_current_server(&target, &config));

    target.base_url = Some("http://localhost:3210".to_string());
    assert!(target_points_at_current_server(&target, &config));

    config.bind = "::".to_string();
    target.base_url = Some("http://[::1]:3210".to_string());
    assert!(target_points_at_current_server(&target, &config));
}

#[test]
fn target_points_at_current_server_matches_wildcard_local_interface_ip() {
    let mut config = Config::default();
    config.bind = "0.0.0.0".to_string();
    config.port = 3210;
    let local_ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(100, 86, 253, 9));
    let mut target = target();
    target.base_url = Some("http://100.86.253.9:3210".to_string());

    assert!(target_points_at_current_server_with_local_ips(
        &target,
        &config,
        &[local_ip]
    ));

    target.base_url = Some("http://100.86.253.10:3210".to_string());
    assert!(!target_points_at_current_server_with_local_ips(
        &target,
        &config,
        &[local_ip]
    ));

    target.base_url = Some("http://100.86.253.9:3211".to_string());
    assert!(!target_points_at_current_server_with_local_ips(
        &target,
        &config,
        &[local_ip]
    ));
}

#[test]
fn target_points_at_current_server_matches_wildcard_local_ipv6_interface() {
    let mut config = Config::default();
    config.bind = "::".to_string();
    config.port = 3210;
    let local_ip = std::net::IpAddr::V6("fd7a:115c:a1e0::1".parse().expect("test IPv6 address"));
    let mut target = target();
    target.base_url = Some("http://[fd7a:115c:a1e0::1]:3210".to_string());

    assert!(target_points_at_current_server_with_local_ips(
        &target,
        &config,
        &[local_ip]
    ));
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
    swimmers.kind = " swimmers_api ".to_string();
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
fn remote_poll_targets_skips_wildcard_bind_self_target() {
    let mut config = Config::default();
    config.bind = "0.0.0.0".to_string();
    config.port = 3210;

    let mut self_target = target();
    self_target.id = "self".to_string();
    self_target.base_url = Some("http://127.0.0.1:3210".to_string());

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
        response
            .session
            .as_ref()
            .expect("created session")
            .session_id,
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
        created
            .session
            .as_ref()
            .expect("created session")
            .session_id,
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

#[test]
fn remote_dir_inventory_path_maps_local_paths_and_defaults_to_first_remote_prefix() {
    let target = target();

    assert_eq!(
        remote_dir_inventory_path(&target, None).expect("default remote path"),
        "/monoserver"
    );
    assert_eq!(
        remote_dir_inventory_path(&target, Some("/workspace/repos/opensource/swimmers"))
            .expect("mapped remote path"),
        "/monoserver/opensource/swimmers"
    );

    let err = remote_dir_inventory_path(&target, Some("/tmp/outside"))
        .expect_err("outside paths should be rejected before remote listing");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "LAUNCH_TARGET_PATH_UNMAPPED");
}

#[test]
fn remote_dir_inventory_target_prefers_cwd_scoped_duplicate_id() {
    let mut global = target();
    global.id = "devbox".to_string();
    global.label = "Global Devbox".to_string();
    global.base_url = Some("http://global.example:3210".to_string());

    let mut cwd_scoped = target();
    cwd_scoped.id = "devbox".to_string();
    cwd_scoped.label = "Cwd Scoped Devbox".to_string();
    cwd_scoped.base_url = Some("http://scoped.example:3210".to_string());

    let selected = choose_dir_inventory_target("devbox", Some(cwd_scoped.clone()), Some(global))
        .expect("target selected");

    assert_eq!(selected.label, "Cwd Scoped Devbox");
    assert_eq!(selected.base_url, cwd_scoped.base_url);
}

#[test]
fn remote_dir_response_maps_remote_paths_back_to_local_cockpit() {
    let target = target();
    let response = DirListResponse {
        path: "/monoserver/opensource".to_string(),
        entries: vec![
            DirEntry {
                name: "swimmers".to_string(),
                has_children: false,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: vec!["core".to_string()],
                full_path: Some("/monoserver/opensource/swimmers".to_string()),
                has_restart: None,
                open_url: None,
            },
            DirEntry {
                name: "outside".to_string(),
                has_children: false,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: Some("/outside/unmapped".to_string()),
                has_restart: None,
                open_url: None,
            },
        ],
        overlay_label: Some("Remote".to_string()),
        groups: vec!["core".to_string()],
        launch_targets: Vec::new(),
        default_launch_target: None,
    };

    let mapped = remote_dir_response_for_local_cockpit(
        &target,
        response,
        Some("/workspace/repos/opensource"),
    );

    assert_eq!(mapped.path, "/workspace/repos/opensource");
    assert_eq!(
        mapped.entries[0].full_path.as_deref(),
        Some("/workspace/repos/opensource/swimmers")
    );
    assert_eq!(
        mapped.entries[1].full_path.as_deref(),
        Some("/outside/unmapped")
    );
    assert_eq!(
        mapped.default_launch_target.as_deref(),
        Some("jeremy-skillbox")
    );
    assert!(!mapped.launch_targets.is_empty());
}
