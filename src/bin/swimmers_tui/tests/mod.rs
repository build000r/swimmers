use super::*;
use std::cell::Cell as TestCell;
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use chrono::Utc;
use proptest::prelude::*;
use swimmers::openrouter_models::default_openrouter_candidates;
use swimmers::types::{GhosttyOpenMode, ThoughtSource, ThoughtState, TransportHealth};
use tempfile::tempdir;

const EXPECTED_TERMINAL_ENTRY: &str = concat!(
    "\u{1b}[?1049h",
    "\u{1b}[?1000h",
    "\u{1b}[?1002h",
    "\u{1b}[?1003h",
    "\u{1b}[?1015h",
    "\u{1b}[?1006h",
    "\u{1b}[?2004h",
    "\u{1b}[?25l",
    "\u{1b}[2J",
);

const EXPECTED_TERMINAL_TEARDOWN: &str = concat!(
    "\u{1b}[?2004l",
    "\u{1b}[?1006l",
    "\u{1b}[?1015l",
    "\u{1b}[?1003l",
    "\u{1b}[?1002l",
    "\u{1b}[?1000l",
    "\u{1b}[?1049l",
    "\u{1b}[?25h",
    "\u{1b}[0m",
);

static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Default)]
struct MockApiState {
    fetch_sessions_results: VecDeque<Result<Vec<SessionSummary>, String>>,
    fetch_thought_config_results: VecDeque<Result<ThoughtConfigResponse, String>>,
    update_thought_config_results: VecDeque<Result<ThoughtConfig, String>>,
    test_thought_config_results: VecDeque<Result<ThoughtConfigTestResponse, String>>,
    refresh_openrouter_candidates_results: VecDeque<Result<Vec<String>, String>>,
    mermaid_artifact_results: VecDeque<Result<MermaidArtifactResponse, String>>,
    plan_file_results: VecDeque<Result<PlanFileResponse, String>>,
    native_status_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    set_native_app_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    set_native_mode_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    publish_selection_results: VecDeque<Result<(), String>>,
    open_session_results: VecDeque<Result<NativeDesktopOpenResponse, String>>,
    list_dirs_results: VecDeque<Result<DirListResponse, String>>,
    create_session_results: VecDeque<Result<CreateSessionResponse, String>>,
    update_thought_config_calls: Vec<ThoughtConfig>,
    test_thought_config_calls: Vec<ThoughtConfig>,
    native_status_calls: usize,
    set_native_app_calls: Vec<NativeDesktopApp>,
    set_native_mode_calls: Vec<GhosttyOpenMode>,
    publish_calls: Vec<Option<String>>,
    open_calls: Vec<String>,
    list_calls: Vec<(Option<String>, bool)>,
    create_calls: Vec<(String, SpawnTool, Option<String>)>,
}

#[derive(Clone, Default)]
struct MockApi {
    state: Arc<Mutex<MockApiState>>,
}

impl MockApi {
    fn new() -> Self {
        Self::default()
    }

    fn push_fetch_sessions(&self, result: Result<Vec<SessionSummary>, String>) {
        self.state
            .lock()
            .unwrap()
            .fetch_sessions_results
            .push_back(result);
    }

    fn push_mermaid_artifact(&self, result: Result<MermaidArtifactResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .mermaid_artifact_results
            .push_back(result);
    }

    fn push_fetch_thought_config(&self, result: Result<ThoughtConfigResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .fetch_thought_config_results
            .push_back(result);
    }

    fn push_update_thought_config(&self, result: Result<ThoughtConfig, String>) {
        self.state
            .lock()
            .unwrap()
            .update_thought_config_results
            .push_back(result);
    }

    fn push_test_thought_config(&self, result: Result<ThoughtConfigTestResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .test_thought_config_results
            .push_back(result);
    }

    fn push_refresh_openrouter_candidates(&self, result: Result<Vec<String>, String>) {
        self.state
            .lock()
            .unwrap()
            .refresh_openrouter_candidates_results
            .push_back(result);
    }

    fn push_plan_file(&self, result: Result<PlanFileResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .plan_file_results
            .push_back(result);
    }

    fn push_native_status(&self, result: Result<NativeDesktopStatusResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .native_status_results
            .push_back(result);
    }

    fn push_set_native_app(&self, result: Result<NativeDesktopStatusResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .set_native_app_results
            .push_back(result);
    }

    fn push_set_native_mode(&self, result: Result<NativeDesktopStatusResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .set_native_mode_results
            .push_back(result);
    }

    fn push_list_dirs(&self, result: Result<DirListResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .list_dirs_results
            .push_back(result);
    }

    fn push_create_session(&self, result: Result<CreateSessionResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .create_session_results
            .push_back(result);
    }

    fn push_open_session(&self, result: Result<NativeDesktopOpenResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .open_session_results
            .push_back(result);
    }

    fn list_calls(&self) -> Vec<(Option<String>, bool)> {
        self.state.lock().unwrap().list_calls.clone()
    }

    fn create_calls(&self) -> Vec<(String, SpawnTool, Option<String>)> {
        self.state.lock().unwrap().create_calls.clone()
    }

    fn publish_calls(&self) -> Vec<Option<String>> {
        self.state.lock().unwrap().publish_calls.clone()
    }

    fn open_calls(&self) -> Vec<String> {
        self.state.lock().unwrap().open_calls.clone()
    }

    fn update_thought_config_calls(&self) -> Vec<ThoughtConfig> {
        self.state
            .lock()
            .unwrap()
            .update_thought_config_calls
            .clone()
    }

    fn test_thought_config_calls(&self) -> Vec<ThoughtConfig> {
        self.state.lock().unwrap().test_thought_config_calls.clone()
    }

    fn native_status_calls(&self) -> usize {
        self.state.lock().unwrap().native_status_calls
    }

    fn set_native_app_calls(&self) -> Vec<NativeDesktopApp> {
        self.state.lock().unwrap().set_native_app_calls.clone()
    }

    fn set_native_mode_calls(&self) -> Vec<GhosttyOpenMode> {
        self.state.lock().unwrap().set_native_mode_calls.clone()
    }
}

impl TuiApi for MockApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .fetch_sessions_results
                .pop_front()
                .unwrap_or_else(|| Ok(Vec::new()))
        })
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .fetch_thought_config_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(ThoughtConfigResponse {
                        config: ThoughtConfig::default(),
                        daemon_defaults: None,
                        ui: swimmers::types::ThoughtConfigUiMetadata::default(),
                    })
                })
        })
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.update_thought_config_calls.push(config.clone());
            state
                .update_thought_config_results
                .pop_front()
                .unwrap_or(Ok(config))
        })
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.test_thought_config_calls.push(config.clone());
            state
                .test_thought_config_results
                .pop_front()
                .unwrap_or(Ok(ThoughtConfigTestResponse {
                    ok: true,
                    message: "probe succeeded".to_string(),
                    last_backend_error: None,
                    llm_calls: 1,
                }))
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .refresh_openrouter_candidates_results
                .pop_front()
                .unwrap_or_else(|| Ok(default_openrouter_candidates()))
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .mermaid_artifact_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(MermaidArtifactResponse {
                        session_id,
                        available: false,
                        path: None,
                        updated_at: None,
                        source: None,
                        error: None,
                        slice_name: None,
                        plan_files: None,
                    })
                })
        })
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .plan_file_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(PlanFileResponse {
                        session_id,
                        name,
                        content: None,
                        error: Some("no mock result configured".to_string()),
                    })
                })
        })
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut locked = state.lock().unwrap();
            locked.native_status_calls += 1;
            locked
                .native_status_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(NativeDesktopStatusResponse {
                        supported: true,
                        platform: Some("test".to_string()),
                        app_id: Some(NativeDesktopApp::Iterm),
                        ghostty_mode: None,
                        app: Some(NativeDesktopApp::Iterm.display_name().to_string()),
                        reason: None,
                    })
                })
        })
    }

    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.set_native_app_calls.push(app);
            state.set_native_app_results.pop_front().unwrap_or_else(|| {
                Ok(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("test".to_string()),
                    app_id: Some(app),
                    ghostty_mode: (app == NativeDesktopApp::Ghostty)
                        .then_some(GhosttyOpenMode::Swap),
                    app: Some(app.display_name().to_string()),
                    reason: None,
                })
            })
        })
    }

    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.set_native_mode_calls.push(mode);
            state
                .set_native_mode_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(NativeDesktopStatusResponse {
                        supported: true,
                        platform: Some("test".to_string()),
                        app_id: Some(NativeDesktopApp::Ghostty),
                        ghostty_mode: Some(mode),
                        app: Some(NativeDesktopApp::Ghostty.display_name().to_string()),
                        reason: None,
                    })
                })
        })
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        let state = self.state.clone();
        let session_id = session_id.map(|value| value.to_string());
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.publish_calls.push(session_id);
            state
                .publish_selection_results
                .pop_front()
                .unwrap_or(Ok(()))
        })
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.open_calls.push(session_id);
            state
                .open_session_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected open_session".to_string()))
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let state = self.state.clone();
        let path = path.map(|value| value.to_string());
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.list_calls.push((path, managed_only));
            state
                .list_dirs_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected list_dirs".to_string()))
        })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        let state = self.state.clone();
        let cwd = cwd.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.create_calls.push((cwd, spawn_tool, initial_request));
            state
                .create_session_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected create_session".to_string()))
        })
    }
}

fn test_runtime() -> Runtime {
    Runtime::new().expect("test runtime")
}

fn test_field() -> Rect {
    Rect {
        x: 1,
        y: 3,
        width: 78,
        height: 14,
    }
}

fn test_layout(width: u16, height: u16) -> WorkspaceLayout {
    WorkspaceLayout::for_terminal(width, height)
}

fn test_layout_with_ratio(width: u16, height: u16, thought_ratio: f32) -> WorkspaceLayout {
    WorkspaceLayout::for_terminal_with_ratio(width, height, thought_ratio)
}

const TEST_REPOS_ROOT: &str = "/tmp/repos";
const TEST_REPO_ALPHA: &str = "/tmp/repos/alpha";
const TEST_REPO_BETA: &str = "/tmp/repos/beta";
const TEST_REPO_BUILDOOOR: &str = "/tmp/repos/buildooor";
const TEST_REPO_DEV: &str = "/tmp/repos/dev";
const TEST_REPO_GAMMA: &str = "/tmp/repos/gamma";
const TEST_REPO_OPENSOURCE: &str = "/tmp/repos/opensource";
const TEST_REPO_SKILLS: &str = "/tmp/repos/opensource/skills";
const TEST_REPO_SWIMMERS: &str = "/tmp/repos/swimmers";

#[derive(Default)]
struct MockArtifactOpenerState {
    calls: Vec<String>,
    error: Option<String>,
}

#[derive(Clone, Default)]
struct MockArtifactOpener {
    state: Arc<Mutex<MockArtifactOpenerState>>,
}

impl MockArtifactOpener {
    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    fn fail_with(&self, message: &str) {
        self.state.lock().unwrap().error = Some(message.to_string());
    }
}

impl ArtifactOpener for MockArtifactOpener {
    fn open(&self, path: &str) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(path.to_string());
        if let Some(message) = state.error.clone() {
            return Err(io::Error::other(message));
        }
        Ok(())
    }
}

#[derive(Default)]
struct MockCommitLauncherState {
    calls: Vec<SessionSummary>,
    result: Option<CommitCodexLaunch>,
    error: Option<String>,
}

#[derive(Clone, Default)]
struct MockCommitLauncher {
    state: Arc<Mutex<MockCommitLauncherState>>,
}

impl MockCommitLauncher {
    fn calls(&self) -> Vec<SessionSummary> {
        self.state.lock().unwrap().calls.clone()
    }

    fn fail_with(&self, message: &str) {
        self.state.lock().unwrap().error = Some(message.to_string());
    }
}

impl CommitLauncher for MockCommitLauncher {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitCodexLaunch> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(session.clone());
        if let Some(message) = state.error.clone() {
            return Err(io::Error::other(message));
        }
        Ok(state.result.clone().unwrap_or(CommitCodexLaunch {
            session_name: "commit-7-123".to_string(),
            watch_command: "tmux a -t commit-7-123".to_string(),
        }))
    }
}

fn make_app(api: MockApi) -> App<MockApi> {
    App::new(test_runtime(), api)
}

fn make_app_with_artifact_opener(
    api: MockApi,
    artifact_opener: Arc<dyn ArtifactOpener>,
) -> App<MockApi> {
    App::with_artifact_opener(test_runtime(), api, artifact_opener)
}

fn make_app_with_commit_launcher(
    api: MockApi,
    commit_launcher: Arc<dyn CommitLauncher>,
) -> App<MockApi> {
    App::with_helpers(
        test_runtime(),
        api,
        Arc::new(SystemArtifactOpener),
        commit_launcher,
    )
}

fn test_api_client(base_url: String, auth_token: Option<&str>) -> ApiClient {
    ApiClient {
        http: Client::builder()
            .connect_timeout(Duration::from_millis(50))
            .timeout(Duration::from_millis(100))
            .build()
            .expect("http client"),
        base_url,
        auth_token: auth_token.map(str::to_string),
    }
}

fn restore_env_var(key: &str, value: Option<String>) {
    match value {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
}

fn write_fake_clawgs_script(args_log: &Path, input_log: &Path, dir: &Path) -> std::path::PathBuf {
    let script_path = dir.join("fake-clawgs.sh");
    let script = r#"#!/bin/sh
printf '%s\n' "$*" >> "__ARGS_LOG__"
if [ "$1" = "defaults" ]; then
  printf '%s\n' '{"model":"test-model","agent_prompt":"You are a status reporter for a coding agent session.","terminal_prompt":"Terminal session status reporter."}'
  exit 0
fi
printf '%s\n' '{"type":"hello","protocol":"clawgs.emit.v1","engine_version":"0.1.0"}'
count=1
while IFS= read -r line; do
  printf '%s\n' "$line" >> "__INPUT_LOG__"
  printf '%s\n' '{"type":"sync_result","id":"'"$count"'","stream_instance_id":"stream-a","updates":[],"metrics":{"sessions_seen":1,"llm_calls":1,"suppressed":0}}'
  count=$((count + 1))
done
sleep 5
"#
    .replace("__ARGS_LOG__", &args_log.display().to_string())
    .replace("__INPUT_LOG__", &input_log.display().to_string());
    fs::write(&script_path, script).expect("write fake clawgs");
    let mut perms = fs::metadata(&script_path)
        .expect("fake clawgs metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("mark fake clawgs executable");
    script_path
}

#[test]
fn thought_config_response_deserializes_flattened_api_shape() {
    let value = serde_json::json!({
        "enabled": true,
        "model": "haiku",
        "backend": "claude",
        "cadence_hot_ms": 15000,
        "cadence_warm_ms": 45000,
        "cadence_cold_ms": 120000,
        "daemon_defaults": {
            "model": "haiku",
            "backend": "claude",
            "agent_prompt": "agent",
            "terminal_prompt": "terminal"
        }
    });

    let response: ThoughtConfigResponse =
        serde_json::from_value(value).expect("flattened thought config response");

    assert_eq!(response.config.backend, "claude");
    assert_eq!(response.config.model, "haiku");
    assert_eq!(
        response
            .daemon_defaults
            .as_ref()
            .map(|defaults| defaults.backend.as_str()),
        Some("claude")
    );
}

async fn spawn_guarded_startup_server(
    expected_token: &str,
    selection_status: axum::http::StatusCode,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, put};
    use axum::Router;

    let expected_sessions_auth = format!("Bearer {expected_token}");
    let expected_selection_auth = expected_sessions_auth.clone();

    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move |headers: HeaderMap| {
                let expected_auth = expected_sessions_auth.clone();
                async move {
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        == Some(expected_auth.as_str())
                    {
                        StatusCode::OK
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }
            }),
        )
        .route(
            "/v1/selection",
            put(move |headers: HeaderMap| {
                let expected_auth = expected_selection_auth.clone();
                async move {
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        == Some(expected_auth.as_str())
                    {
                        selection_status
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn api_client_transport_errors_are_actionable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let client = test_api_client(format!("http://127.0.0.1:{port}"), None);

    let error = client
        .fetch_sessions()
        .await
        .expect_err("closed localhost port should fail");
    assert!(error.contains("swimmers API unavailable at"));
    assert!(error.contains("Start `swimmers` or set SWIMMERS_TUI_URL."));
    assert!(!error.contains("error sending request for url"));
}

#[tokio::test]
async fn api_client_test_thought_config_falls_back_when_local_backend_is_unreachable() {
    let _lock = TEST_ENV_LOCK.lock().expect("env lock");
    let original = env::var("CLAWGS_BIN").ok();
    let temp = tempdir().expect("tempdir");
    let args_log = temp.path().join("args.log");
    let input_log = temp.path().join("input.log");
    let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());
    env::set_var("CLAWGS_BIN", fake_bin.as_os_str());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let client = test_api_client(format!("http://127.0.0.1:{port}"), None);
    let response = client
        .test_thought_config(ThoughtConfig::default())
        .await
        .expect("local transport error should fall back to local probe");

    restore_env_var("CLAWGS_BIN", original);

    assert!(response.ok);
    assert_eq!(response.message, "probe succeeded");
    assert_eq!(response.llm_calls, 1);
}

#[tokio::test]
async fn api_client_test_thought_config_falls_back_when_backend_route_is_missing() {
    use axum::Router;

    let _lock = TEST_ENV_LOCK.lock().expect("env lock");
    let original = env::var("CLAWGS_BIN").ok();
    let temp = tempdir().expect("tempdir");
    let args_log = temp.path().join("args.log");
    let input_log = temp.path().join("input.log");
    let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());
    env::set_var("CLAWGS_BIN", fake_bin.as_os_str());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve empty test api");
    });

    let client = test_api_client(format!("http://{addr}"), None);
    let response = client
        .test_thought_config(ThoughtConfig::default())
        .await
        .expect("404 fallback should return local probe result");

    handle.abort();
    restore_env_var("CLAWGS_BIN", original);

    assert!(response.ok);
    assert_eq!(response.message, "probe succeeded");
    assert_eq!(response.llm_calls, 1);
}

async fn spawn_delayed_api_server(
    sessions_delay: Option<Duration>,
    native_open_delay: Option<Duration>,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::routing::{get, post, put};
    use axum::{Json, Router};

    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move || async move {
                if let Some(delay) = sessions_delay {
                    tokio::time::sleep(delay).await;
                }
                Json(SessionListResponse {
                    sessions: vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
                    version: 1,
                    repo_themes: HashMap::new(),
                })
            }),
        )
        .route(
            "/v1/native/open",
            post(move || async move {
                if let Some(delay) = native_open_delay {
                    tokio::time::sleep(delay).await;
                }
                Json(NativeDesktopOpenResponse {
                    session_id: "sess-1".to_string(),
                    status: "focused".to_string(),
                    pane_id: Some("pane-1".to_string()),
                })
            }),
        )
        .route(
            "/v1/native/app",
            put(|Json(body): Json<NativeDesktopConfigRequest>| async move {
                Json(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("macos".to_string()),
                    app_id: Some(body.app),
                    ghostty_mode: (body.app == NativeDesktopApp::Ghostty)
                        .then_some(GhosttyOpenMode::Swap),
                    app: Some(body.app.display_name().to_string()),
                    reason: None,
                })
            }),
        )
        .route(
            "/v1/native/mode",
            put(|Json(body): Json<NativeDesktopModeRequest>| async move {
                Json(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("macos".to_string()),
                    app_id: Some(NativeDesktopApp::Ghostty),
                    ghostty_mode: Some(body.mode),
                    app: Some(NativeDesktopApp::Ghostty.display_name().to_string()),
                    reason: None,
                })
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn api_client_open_session_allows_slower_native_open_responses() {
    let (base_url, handle) = spawn_delayed_api_server(None, Some(Duration::from_millis(150))).await;
    let client = test_api_client(base_url, None);

    let response = client
        .open_session("sess-1")
        .await
        .expect("native open should outlive the default polling timeout");

    handle.abort();
    assert_eq!(response.session_id, "sess-1");
    assert_eq!(response.status, "focused");
    assert_eq!(response.pane_id.as_deref(), Some("pane-1"));
}

#[tokio::test]
async fn api_client_can_switch_native_app_without_restart() {
    let (base_url, handle) = spawn_delayed_api_server(None, None).await;
    let client = test_api_client(base_url, None);

    let response = client
        .set_native_app(NativeDesktopApp::Ghostty)
        .await
        .expect("native app switch should succeed");

    handle.abort();
    assert_eq!(response.app_id, Some(NativeDesktopApp::Ghostty));
    assert_eq!(response.ghostty_mode, Some(GhosttyOpenMode::Swap));
    assert_eq!(response.app.as_deref(), Some("Ghostty"));
}

#[tokio::test]
async fn api_client_can_switch_ghostty_mode_without_restart() {
    let (base_url, handle) = spawn_delayed_api_server(None, None).await;
    let client = test_api_client(base_url, None);

    let response = client
        .set_native_mode(GhosttyOpenMode::Add)
        .await
        .expect("native mode switch should succeed");

    handle.abort();
    assert_eq!(response.app_id, Some(NativeDesktopApp::Ghostty));
    assert_eq!(response.ghostty_mode, Some(GhosttyOpenMode::Add));
}

#[tokio::test]
async fn api_client_set_native_app_reports_restart_hint_on_404() {
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), None);

    let error = client
        .set_native_app(NativeDesktopApp::Ghostty)
        .await
        .expect_err("missing route should surface restart hint");

    handle.abort();
    assert!(error.contains("does not support runtime native target switching yet"));
    assert!(error.contains("restart `swimmers`"));
}

#[tokio::test]
async fn api_client_set_native_mode_reports_restart_hint_on_404() {
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), None);

    let error = client
        .set_native_mode(GhosttyOpenMode::Add)
        .await
        .expect_err("missing route should surface restart hint");

    handle.abort();
    assert!(error.contains("does not support runtime Ghostty preview mode switching yet"));
    assert!(error.contains("restart `swimmers`"));
}

#[tokio::test]
async fn api_client_fetch_sessions_keeps_short_timeout_for_refresh() {
    let (base_url, handle) = spawn_delayed_api_server(Some(Duration::from_millis(150)), None).await;
    let client = test_api_client(base_url.clone(), None);

    let error = client
        .fetch_sessions()
        .await
        .expect_err("refresh should keep the short polling timeout");

    handle.abort();
    assert!(error.contains(&base_url));
    assert!(error.contains("timed out while trying to refresh sessions"));
}

#[tokio::test]
async fn startup_preflight_accepts_matching_bearer_token() {
    let (base_url, handle) =
        spawn_guarded_startup_server("testtoken", axum::http::StatusCode::OK).await;
    let client = test_api_client(base_url, Some("testtoken"));

    let result = client.preflight_startup_access().await;

    handle.abort();
    assert!(
        result.is_ok(),
        "matching token should pass startup preflight"
    );
}

#[tokio::test]
async fn startup_preflight_requires_matching_auth_for_sessions() {
    let (base_url, handle) =
        spawn_guarded_startup_server("testtoken", axum::http::StatusCode::OK).await;
    let client = test_api_client(base_url.clone(), None);

    let error = client
        .preflight_startup_access()
        .await
        .expect_err("missing auth should fail startup preflight");

    handle.abort();
    assert!(error.contains(&base_url));
    assert!(error.contains("/v1/sessions"));
    assert!(error.contains("AUTH_MODE=token"));
    assert!(error.contains("AUTH_TOKEN"));
}

#[tokio::test]
async fn startup_preflight_requires_selection_scope() {
    let (base_url, handle) =
        spawn_guarded_startup_server("testtoken", axum::http::StatusCode::FORBIDDEN).await;
    let client = test_api_client(base_url.clone(), Some("testtoken"));

    let error = client
        .preflight_startup_access()
        .await
        .expect_err("selection auth failure should fail startup preflight");

    handle.abort();
    assert!(error.contains(&base_url));
    assert!(error.contains("/v1/selection"));
    assert!(error.contains("required session scope"));
}

#[test]
fn set_message_deduplicates_repeated_errors() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.set_message("backend unavailable");
    let first = app.message.as_ref().expect("message").1;

    std::thread::sleep(Duration::from_millis(5));
    app.set_message("backend unavailable");

    let second = app.message.as_ref().expect("message").1;
    assert_eq!(first, second);
}

#[test]
fn auto_refresh_keeps_existing_footer_message() {
    let api = MockApi::new();
    let layout = test_layout(160, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-7", "7", TEST_REPO_SWIMMERS)]));
    let mut app = make_app(api);
    app.set_message("sticky status");

    app.refresh(layout);

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("sticky status")
    );
}

#[test]
fn manual_refresh_reports_session_count() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary("sess-7", "7", TEST_REPO_SWIMMERS),
        session_summary("sess-8", "8", TEST_REPO_OPENSOURCE),
    ]));
    let mut app = make_app(api);

    app.manual_refresh(layout);

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("refreshed 2 sessions")
    );
}

#[test]
fn refresh_skips_native_status_when_sessions_fail() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Err("timed out while trying to refresh sessions".to_string()));
    let mut app = make_app(api.clone());

    app.refresh(layout);

    assert_eq!(
        api.native_status_calls(),
        0,
        "native status should not be called when sessions failed"
    );
    assert!(
        app.message
            .as_ref()
            .map(|(m, _)| m.contains("refresh sessions"))
            .unwrap_or(false),
        "sessions error should be in message"
    );
}

#[test]
fn refresh_calls_native_status_when_sessions_succeed() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    }));
    let mut app = make_app(api.clone());

    app.refresh(layout);

    assert_eq!(
        api.native_status_calls(),
        1,
        "native status should be called when sessions succeeded"
    );
    assert!(app.native_status.is_some());
}

#[test]
fn refresh_sessions_error_not_overwritten_by_native_status_error() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Err("timed out while trying to refresh sessions".to_string()));
    let mut app = make_app(api.clone());

    app.refresh(layout);

    let msg = app
        .message
        .as_ref()
        .map(|(m, _)| m.as_str())
        .unwrap_or("");
    assert!(
        msg.contains("refresh sessions"),
        "expected sessions error, got: {msg}"
    );
    assert!(
        !msg.contains("native desktop status"),
        "native-status error must not overwrite sessions error: {msg}"
    );
}

#[test]
fn refresh_retains_cached_native_status_when_sessions_fail() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let cached = NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    };
    api.push_fetch_sessions(Ok(vec![]));
    api.push_native_status(Ok(cached.clone()));
    let mut app = make_app(api.clone());
    app.refresh(layout);
    assert!(app.native_status.is_some(), "setup: native_status should be populated");

    api.push_fetch_sessions(Err("backend down".to_string()));
    app.refresh(layout);

    assert!(
        app.native_status.is_some(),
        "cached native_status should be retained after a failed refresh"
    );
    assert_eq!(
        app.native_status.as_ref().unwrap().app.as_deref(),
        Some("iTerm"),
        "cached value should match what was last successfully fetched"
    );
}

fn test_renderer(width: u16, height: u16) -> Renderer {
    let buffer_size = (width as usize) * (height as usize);
    Renderer {
        stdout: BufWriter::new(io::stdout()),
        width,
        height,
        buffer: vec![Cell::default(); buffer_size],
        last_buffer: vec![Cell::default(); buffer_size],
        terminal_state: TerminalState::default(),
    }
}

#[test]
fn enter_terminal_ui_enables_bracketed_paste_with_mouse_capture() {
    let mut output = Vec::new();

    enter_terminal_ui(&mut output).expect("enter terminal UI should write ANSI codes");

    assert_eq!(
        String::from_utf8(output).expect("terminal startup output should be valid utf-8"),
        EXPECTED_TERMINAL_ENTRY
    );
}

#[test]
fn leave_terminal_ui_disables_bracketed_paste_before_leaving_alt_screen() {
    let mut output = Vec::new();

    leave_terminal_ui(&mut output).expect("leave terminal UI should write ANSI codes");

    assert_eq!(
        String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
        EXPECTED_TERMINAL_TEARDOWN
    );
}

#[test]
fn cleanup_is_noop_when_renderer_is_inactive() {
    let mut renderer = test_renderer(80, 24);

    renderer.cleanup().expect("inactive cleanup should succeed");

    assert!(!renderer.terminal_state.raw_mode_enabled);
    assert!(!renderer.terminal_state.terminal_ui_active);
}

#[test]
fn cleanup_after_runtime_error_restores_terminal_in_reverse_order() {
    let mut terminal_state = TerminalState::default();
    let mut output = Vec::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    terminal_state
        .init_with(
            &mut output,
            {
                let events = Arc::clone(&events);
                move || {
                    events.lock().unwrap().push("enable_raw_mode");
                    Ok(())
                }
            },
            {
                let events = Arc::clone(&events);
                move |_writer| {
                    events.lock().unwrap().push("enter_terminal_ui");
                    Ok(())
                }
            },
        )
        .expect("terminal init should succeed");

    terminal_state
        .cleanup_with(
            &mut output,
            {
                let events = Arc::clone(&events);
                move |writer| {
                    events.lock().unwrap().push("leave_terminal_ui");
                    leave_terminal_ui(writer)
                }
            },
            {
                let events = Arc::clone(&events);
                move || {
                    events.lock().unwrap().push("disable_raw_mode");
                    Ok(())
                }
            },
        )
        .expect("cleanup should succeed after a runtime error");

    assert_eq!(
        String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
        EXPECTED_TERMINAL_TEARDOWN
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "enable_raw_mode",
            "enter_terminal_ui",
            "leave_terminal_ui",
            "disable_raw_mode",
        ]
    );
}

#[test]
fn failed_init_still_runs_full_cleanup_once() {
    let mut terminal_state = TerminalState::default();
    let mut output = Vec::new();
    let leave_calls = TestCell::new(0usize);
    let disable_calls = TestCell::new(0usize);

    let err = terminal_state
        .init_with(
            &mut output,
            || Ok(()),
            |_writer| Err(io::Error::other("forced init failure")),
        )
        .expect_err("init should surface the forced failure");
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(err.to_string(), "forced init failure");

    terminal_state
        .cleanup_with(
            &mut output,
            |writer| {
                leave_calls.set(leave_calls.get() + 1);
                leave_terminal_ui(writer)
            },
            || {
                disable_calls.set(disable_calls.get() + 1);
                Ok(())
            },
        )
        .expect("cleanup should restore the terminal after init failure");

    terminal_state
        .cleanup_with(
            &mut output,
            |writer| {
                leave_calls.set(leave_calls.get() + 1);
                leave_terminal_ui(writer)
            },
            || {
                disable_calls.set(disable_calls.get() + 1);
                Ok(())
            },
        )
        .expect("second cleanup should be a no-op");

    assert_eq!(
        String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
        EXPECTED_TERMINAL_TEARDOWN
    );
    assert_eq!(leave_calls.get(), 1);
    assert_eq!(disable_calls.get(), 1);
    assert!(!terminal_state.raw_mode_enabled);
    assert!(!terminal_state.terminal_ui_active);
}

fn cell_at(renderer: &Renderer, x: u16, y: u16) -> Cell {
    renderer.buffer[(y as usize) * (renderer.width as usize) + (x as usize)]
}

fn row_text(renderer: &Renderer, y: u16) -> String {
    (0..renderer.width)
        .map(|x| cell_at(renderer, x, y).ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn find_text_position(renderer: &Renderer, needle: &str) -> Option<(u16, u16)> {
    for y in 0..renderer.height {
        let row = row_text(renderer, y);
        if let Some(byte_index) = row.find(needle) {
            let char_index = row[..byte_index].chars().count() as u16;
            return Some((char_index, y));
        }
    }
    None
}

fn find_blank_position(renderer: &Renderer, rect: Rect) -> Option<(u16, u16)> {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if cell_at(renderer, x, y).ch == ' ' {
                return Some((x, y));
            }
        }
    }
    None
}

fn open_mermaid_test_viewer(
    source: &str,
    width: u16,
    height: u16,
) -> (App<MockApi>, Renderer, WorkspaceLayout) {
    let api = MockApi::new();
    let layout = test_layout(width, height);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            source,
        ),
    );
    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = None;
    (app, test_renderer(width, height), layout)
}

#[derive(Clone, Copy, Debug)]
enum MermaidMetamorphicOp {
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MermaidSemanticSnapshot {
    source_index: usize,
    text: String,
    rel_x: u16,
    rel_y: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MermaidMetamorphicSnapshot {
    view_state: MermaidViewState,
    focused_source_index: Option<usize>,
    cached_lines: Vec<String>,
    semantic_lines: Vec<MermaidSemanticSnapshot>,
}

fn mermaid_flowchart_source_strategy() -> impl Strategy<Value = String> {
    let words = prop::sample::select(vec![
        "Alpha", "Beta", "Gamma", "Delta", "Producer", "Consumer", "Queue", "Worker", "Client",
        "Server", "Stream", "Buffer",
    ]);
    let edges = prop::sample::select(vec!["ships", "queues", "sends", "loads", "syncs", "pushes"]);

    (
        0u8..3,
        words.clone(),
        words.clone(),
        words.clone(),
        words.clone(),
        edges,
    )
        .prop_map(
            |(template, left, right, extra, group, edge)| match template {
                0 => format!("graph TD\nA[{left}] -->|{edge}| B[{right}]\n"),
                1 => format!(
                    "graph TD\nsubgraph {group}\nA[{left}]\nB[{right}]\nend\nA -->|{edge}| B\n"
                ),
                _ => format!("graph TD\nA[{left}] -->|{edge}| B[{right}]\nA --> C[{extra}]\n"),
            },
        )
}

fn mermaid_anchorable_source_strategy() -> impl Strategy<Value = String> {
    let words = prop::sample::select(vec![
        "Alpha", "Beta", "Gamma", "Delta", "Producer", "Consumer", "Stream", "Buffer",
    ]);
    let edges = prop::sample::select(vec!["ships", "queues", "sends", "syncs"]);

    (words.clone(), words, edges).prop_map(|(left, right, edge)| {
        format!("graph TD\nA[{left} Node] -->|{edge}| B[{right} Node]\n")
    })
}

fn mermaid_metamorphic_ops_strategy() -> impl Strategy<Value = Vec<MermaidMetamorphicOp>> {
    proptest::collection::vec(0u8..6, 0..8).prop_map(|ops| {
        ops.into_iter()
            .map(|op| match op {
                0 => MermaidMetamorphicOp::ZoomIn,
                1 => MermaidMetamorphicOp::ZoomOut,
                2 => MermaidMetamorphicOp::PanLeft,
                3 => MermaidMetamorphicOp::PanRight,
                4 => MermaidMetamorphicOp::PanUp,
                _ => MermaidMetamorphicOp::PanDown,
            })
            .collect()
    })
}

fn mermaid_snapshot(viewer: &MermaidViewerState) -> MermaidMetamorphicSnapshot {
    let content_rect = viewer.content_rect.expect("content rect");
    let mut semantic_lines = viewer
        .cached_semantic_lines
        .iter()
        .map(|line| MermaidSemanticSnapshot {
            source_index: line.source_index,
            text: line.text.clone(),
            rel_x: line.x.saturating_sub(content_rect.x),
            rel_y: line.y.saturating_sub(content_rect.y),
        })
        .collect::<Vec<_>>();
    semantic_lines.sort();

    MermaidMetamorphicSnapshot {
        view_state: mermaid_view_state_for_view(viewer, content_rect),
        focused_source_index: viewer.focused_source_index,
        cached_lines: viewer.cached_lines.clone(),
        semantic_lines,
    }
}

fn render_mermaid_snapshot(
    app: &mut App<MockApi>,
    renderer: &mut Renderer,
    layout: WorkspaceLayout,
) -> MermaidMetamorphicSnapshot {
    app.render(renderer, layout);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    mermaid_snapshot(viewer)
}

fn mermaid_content_rect_for_layout(layout: WorkspaceLayout) -> Rect {
    mermaid_content_rect(layout.overview_field)
}

fn mermaid_pan_headroom(viewer: &MermaidViewerState, content_rect: Rect) -> (f32, f32, f32, f32) {
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    let base_scale = mermaid_fit_scale(
        viewer.diagram_width,
        viewer.diagram_height,
        sample_width as f32,
        sample_height as f32,
    );
    let scale = (base_scale * viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM)).max(0.000_1);
    let visible_width = sample_width as f32 / scale;
    let visible_height = sample_height as f32 / scale;

    let min_center_x = if visible_width >= viewer.diagram_width {
        viewer.diagram_width / 2.0
    } else {
        visible_width / 2.0
    };
    let max_center_x = if visible_width >= viewer.diagram_width {
        viewer.diagram_width / 2.0
    } else {
        viewer.diagram_width - visible_width / 2.0
    };
    let min_center_y = if visible_height >= viewer.diagram_height {
        viewer.diagram_height / 2.0
    } else {
        visible_height / 2.0
    };
    let max_center_y = if visible_height >= viewer.diagram_height {
        viewer.diagram_height / 2.0
    } else {
        viewer.diagram_height - visible_height / 2.0
    };

    (
        (viewer.center_x - min_center_x).max(0.0),
        (max_center_x - viewer.center_x).max(0.0),
        (viewer.center_y - min_center_y).max(0.0),
        (max_center_y - viewer.center_y).max(0.0),
    )
}

fn mermaid_safe_pan_distance(
    ratio_percent: i16,
    negative_headroom: f32,
    positive_headroom: f32,
) -> f32 {
    if ratio_percent < 0 {
        -negative_headroom * f32::from(-ratio_percent) / 100.0
    } else {
        positive_headroom * f32::from(ratio_percent) / 100.0
    }
}

fn apply_mermaid_metamorphic_ops(
    app: &mut App<MockApi>,
    layout: WorkspaceLayout,
    ops: &[MermaidMetamorphicOp],
) {
    let content_rect = mermaid_content_rect_for_layout(layout);
    for op in ops {
        match op {
            MermaidMetamorphicOp::ZoomIn => {
                app.zoom_mermaid_viewer(MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
            }
            MermaidMetamorphicOp::ZoomOut => {
                app.zoom_mermaid_viewer(-MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
            }
            MermaidMetamorphicOp::PanLeft => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).0,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(-step, 0.0);
            }
            MermaidMetamorphicOp::PanRight => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).0,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(step, 0.0);
            }
            MermaidMetamorphicOp::PanUp => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).1,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(0.0, -step);
            }
            MermaidMetamorphicOp::PanDown => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).1,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(0.0, step);
            }
        }
    }
}

fn find_cached_semantic_line(viewer: &MermaidViewerState, needle: &str) -> Option<(u16, u16)> {
    viewer
        .cached_semantic_lines
        .iter()
        .find(|line| line.text == needle)
        .map(|line| (line.x, line.y))
}

fn cached_semantic_texts(viewer: &MermaidViewerState) -> Vec<String> {
    viewer
        .cached_semantic_lines
        .iter()
        .map(|line| line.text.clone())
        .collect()
}

fn mermaid_background_charset(viewer: &MermaidViewerState) -> Vec<char> {
    viewer
        .cached_lines
        .iter()
        .flat_map(|line| line.chars())
        .filter(|ch| *ch != ' ')
        .collect()
}

fn mermaid_background_colors(viewer: &MermaidViewerState) -> Vec<Color> {
    viewer
        .cached_background_cells
        .iter()
        .flat_map(|row| row.iter())
        .filter(|cell| cell.ch != ' ')
        .map(|cell| cell.fg)
        .collect()
}

fn mermaid_background_colors_set(
    viewer: &MermaidViewerState,
) -> std::collections::BTreeSet<String> {
    mermaid_background_colors(viewer)
        .into_iter()
        .map(|color| format!("{color:?}"))
        .collect()
}

fn mermaid_text_color(renderer: &Renderer, needle: &str) -> Color {
    let (x, y) = find_text_position(renderer, needle).unwrap_or_else(|| panic!("{needle}"));
    cell_at(renderer, x, y).fg
}

fn mermaid_border_color(renderer: &Renderer, needle: &str) -> Color {
    let (x, y) = find_text_position(renderer, needle).unwrap_or_else(|| panic!("{needle}"));
    let width = display_width(needle);
    let candidates = [
        (x.saturating_sub(1), y),
        (x.saturating_add(width), y),
        (x, y.saturating_sub(1)),
        (x, y.saturating_add(1)),
    ];
    candidates
        .into_iter()
        .map(|(cx, cy)| cell_at(renderer, cx, cy))
        .find(|cell| matches!(cell.ch, '|' | '_'))
        .map(|cell| cell.fg)
        .unwrap_or_else(|| panic!("missing border for {needle}"))
}

fn mermaid_owner_key_for_text(viewer: &MermaidViewerState, needle: &str) -> String {
    let line = viewer
        .cached_semantic_lines
        .iter()
        .find(|line| line.text == needle)
        .unwrap_or_else(|| panic!("{needle}"));
    viewer
        .prepared_render
        .as_ref()
        .and_then(|prepared| prepared.semantic_lines.get(line.source_index))
        .map(|line| line.owner_key.clone())
        .unwrap_or_else(|| panic!("missing owner key for {needle}"))
}

fn mermaid_render_bounds(
    viewer: &MermaidViewerState,
    content_rect: Rect,
) -> Option<(u16, u16, u16, u16)> {
    let mut left = u16::MAX;
    let mut right = 0u16;
    let mut top = u16::MAX;
    let mut bottom = 0u16;
    let mut saw_any = false;

    for (row_offset, line) in viewer.cached_lines.iter().enumerate() {
        let y = content_rect.y + row_offset as u16;
        for (column_offset, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let x = content_rect.x + column_offset as u16;
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            saw_any = true;
        }
    }

    for line in &viewer.cached_semantic_lines {
        let text_right = line
            .x
            .saturating_add(display_width(&line.text).saturating_sub(1));
        left = left.min(line.x);
        right = right.max(text_right);
        top = top.min(line.y);
        bottom = bottom.max(line.y);
        saw_any = true;
    }

    saw_any.then_some((left, right, top, bottom))
}

fn er_order_node(owner_key: &str, x: f32, y: f32, neighbors: &[&str]) -> MermaidErOrderNode {
    MermaidErOrderNode {
        owner_key: owner_key.to_string(),
        x,
        y,
        neighbors: neighbors
            .iter()
            .map(|neighbor| (*neighbor).to_string())
            .collect(),
    }
}

fn press_mermaid_key(app: &mut App<MockApi>, layout: WorkspaceLayout, key: char) {
    assert!(handle_key_event(
        app,
        layout,
        KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
    ));
}

fn press_mermaid_tab(app: &mut App<MockApi>, layout: WorkspaceLayout) {
    assert!(handle_key_event(
        app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    ));
}

fn press_mermaid_backtab(app: &mut App<MockApi>, layout: WorkspaceLayout) {
    assert!(handle_key_event(
        app,
        layout,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    ));
}

fn scroll_mermaid(
    app: &mut App<MockApi>,
    layout: WorkspaceLayout,
    direction: MermaidZoomDirection,
) {
    let column = layout.overview_field.x + layout.overview_field.width / 2;
    let row = layout.overview_field.y + layout.overview_field.height / 2;
    assert!(app.handle_mermaid_scroll(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: match direction {
                MermaidZoomDirection::In => MouseEventKind::ScrollUp,
                MermaidZoomDirection::Out => MouseEventKind::ScrollDown,
            },
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        direction,
    ));
}

#[test]
fn mermaid_compact_overview_text_prefers_numeric_prefix_and_keywords() {
    let compact = mermaid_compact_overview_text([
        "1. Verified Identity And",
        "/api/cfo/admin/* calls are not outside the hierarchy",
    ])
    .expect("compact overview text");

    assert_eq!(compact, "1. Verified Identity");
}

#[test]
fn mermaid_compact_overview_text_splits_snake_case_into_words() {
    let compact = mermaid_compact_overview_text(["governed_revision_artifacts"])
        .expect("compact snake_case overview text");

    assert_eq!(compact, "governed revision");
}

fn visible_entity_ids(app: &App<MockApi>) -> Vec<String> {
    app.visible_entities()
        .into_iter()
        .map(|entity| entity.session.session_id.clone())
        .collect()
}

fn session_summary(session_id: &str, tmux_name: &str, cwd: &str) -> SessionSummary {
    SessionSummary {
        session_id: session_id.to_string(),
        tmux_name: tmux_name.to_string(),
        state: SessionState::Idle,
        current_command: None,
        cwd: cwd.to_string(),
        tool: Some("Codex".to_string()),
        token_count: 0,
        context_limit: 192_000,
        thought: None,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        thought_updated_at: None,
        rest_state: RestState::Drowsy,
        commit_candidate: false,
        objective_changed_at: None,
        last_skill: None,
        is_stale: false,
        attached_clients: 0,
        transport_health: TransportHealth::Healthy,
        last_activity_at: Utc::now(),
        repo_theme_id: None,
    }
}

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("valid timestamp")
        .with_timezone(&Utc)
}

fn session_summary_with_thought(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    thought: &str,
    updated_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.thought = Some(thought.to_string());
    session.thought_state = ThoughtState::Active;
    session.rest_state = RestState::Active;
    session.thought_updated_at = Some(timestamp(updated_at));
    session
}

fn mermaid_artifact(
    session_id: &str,
    path: &str,
    updated_at: &str,
    source: &str,
) -> MermaidArtifactResponse {
    let slice_name = swimmers::session::artifacts::extract_mmd_slice_name(path).map(str::to_owned);
    MermaidArtifactResponse {
        session_id: session_id.to_string(),
        available: true,
        path: Some(path.to_string()),
        updated_at: Some(timestamp(updated_at)),
        source: Some(source.to_string()),
        error: None,
        slice_name,
        plan_files: None,
    }
}

fn sleeping_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    last_activity_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.thought_state = ThoughtState::Sleeping;
    session.rest_state = RestState::Sleeping;
    session.last_activity_at = timestamp(last_activity_at);
    session
}

fn deep_sleep_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    last_activity_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.thought_state = ThoughtState::Sleeping;
    session.rest_state = RestState::DeepSleep;
    session.last_activity_at = timestamp(last_activity_at);
    session
}

fn attention_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    rest_state: RestState,
    last_activity_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.state = SessionState::Attention;
    session.rest_state = rest_state;
    session.thought_state = match rest_state {
        RestState::Sleeping | RestState::DeepSleep => ThoughtState::Sleeping,
        RestState::Active | RestState::Drowsy => ThoughtState::Holding,
    };
    session.last_activity_at = timestamp(last_activity_at);
    session
}

fn repo_theme(body: &str) -> RepoTheme {
    RepoTheme {
        body: body.to_string(),
        outline: "#222222".to_string(),
        accent: "#111111".to_string(),
        shirt: "#333333".to_string(),
    }
}

fn dir_response(path: &str, names: &[(&str, bool)]) -> DirListResponse {
    DirListResponse {
        path: path.to_string(),
        entries: names
            .iter()
            .map(|(name, has_children)| DirEntry {
                name: (*name).to_string(),
                has_children: *has_children,
                is_running: None,
            })
            .collect(),
    }
}

fn write_repo_theme_file(path: &std::path::Path, body: &str) {
    write_repo_theme_file_in(path, ".swimmers", body);
}

fn write_repo_theme_file_in(path: &std::path::Path, theme_dir: &str, body: &str) {
    let swimmers_dir = path.join(theme_dir);
    fs::create_dir_all(&swimmers_dir).expect("create theme dir");
    let contents = format!(
        concat!(
            "{{\n",
            "  \"palette\": {{\n",
            "    \"body\": \"{}\",\n",
            "    \"outline\": \"#3D2F24\",\n",
            "    \"accent\": \"#1D1914\",\n",
            "    \"shirt\": \"#AA9370\"\n",
            "  }}\n",
            "}}\n"
        ),
        body,
    );
    fs::write(swimmers_dir.join("colors.json"), contents).expect("write colors.json");
}

fn color_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb { r, g, b } => (r, g, b),
        other => panic!("expected rgb color, got {other:?}"),
    }
}

fn assert_dark_terminal_readable(color: Color) {
    assert!(
        contrast_ratio(color_rgb(color), DARK_TERMINAL_BG_RGB) >= MIN_DARK_TERMINAL_CONTRAST,
        "expected {color:?} to satisfy the dark-terminal contrast threshold"
    );
}

fn create_response(session_id: &str, tmux_name: &str, cwd: &str) -> CreateSessionResponse {
    CreateSessionResponse {
        session: session_summary(session_id, tmux_name, cwd),
        repo_theme: None,
    }
}

fn create_response_with_theme(
    session: SessionSummary,
    repo_theme: RepoTheme,
) -> CreateSessionResponse {
    CreateSessionResponse {
        session,
        repo_theme: Some(repo_theme),
    }
}

fn entity_at(
    field: Rect,
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    x: u16,
    y: u16,
) -> SessionEntity {
    let mut entity = SessionEntity::new(session_summary(session_id, tmux_name, cwd), field);
    entity.x = x.saturating_sub(field.x) as f32;
    entity.y = y.saturating_sub(field.y) as f32;
    entity.swim_anchor_x = entity.x;
    entity.swim_anchor_y = entity.y;
    entity.swim_center_y = entity.y;
    entity
}

fn entity_rect_for(app: &App<MockApi>, session_id: &str, field: Rect) -> Rect {
    app.entities
        .iter()
        .find(|entity| entity.session.session_id == session_id)
        .expect("entity should exist")
        .screen_rect(field)
}

fn sleep_grid_rect(field: Rect, slot: usize) -> Rect {
    let (x, y) = bottom_rest_origin(field, slot);
    Rect {
        x: field.x + x,
        y: field.y + y,
        width: ENTITY_WIDTH,
        height: ENTITY_HEIGHT,
    }
}

fn deep_sleep_grid_rect(field: Rect, slot: usize) -> Rect {
    let (x, y) = top_rest_origin(field, slot);
    Rect {
        x: field.x + x,
        y: field.y + y,
        width: ENTITY_WIDTH,
        height: ENTITY_HEIGHT,
    }
}

#[test]
fn wide_layout_enables_global_thought_rail() {
    let layout = test_layout(120, 32);

    assert!(layout.thought_box.is_some());
    assert!(layout.thought_content.is_some());
    assert!(layout.thought_entry_capacity() > 0);
    assert!(layout.overview_box.x > layout.workspace_box.x);
}

#[test]
fn narrow_layout_keeps_single_overview_field() {
    let layout = test_layout(96, 24);

    assert!(layout.thought_box.is_none());
    assert!(layout.thought_content.is_none());
    assert_eq!(layout.thought_entry_capacity(), 0);
    assert_eq!(layout.overview_box.x, layout.workspace_box.x);
    assert_eq!(layout.overview_field, layout.workspace_box.inset(1));
}

#[test]
fn custom_split_ratio_changes_thought_rail_width() {
    let default_layout = test_layout(120, 32);
    let wider_layout = test_layout_with_ratio(120, 32, 0.5);

    assert_eq!(
        default_layout.split_divider.map(|divider| divider.width),
        Some(THOUGHT_RAIL_GAP)
    );
    assert!(
        wider_layout
            .thought_box
            .expect("wide layout should include thought rail")
            .width
            > default_layout
                .thought_box
                .expect("default layout should include thought rail")
                .width
    );
    assert!(
        wider_layout.overview_field.width < default_layout.overview_field.width,
        "widening the clawgs rail should shrink the swimmers field"
    );
}

#[test]
fn divider_drag_updates_thought_rail_ratio() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let initial_layout = app.layout_for_terminal(120, 32);
    let initial_width = initial_layout
        .thought_box
        .expect("wide layout should include thought rail")
        .width;
    let divider = initial_layout
        .split_divider
        .expect("wide layout should expose a divider");
    let hitbox = initial_layout
        .split_hitbox
        .expect("wide layout should expose a divider hitbox");
    assert!(hitbox.contains(divider.x, divider.y));

    assert!(app.start_split_drag(initial_layout, divider.x));
    assert!(app.split_drag_active);
    assert!(app.drag_split(initial_layout, divider.x + 10));

    let dragged_layout = app.layout_for_terminal(120, 32);
    let dragged_width = dragged_layout
        .thought_box
        .expect("dragged layout should include thought rail")
        .width;
    assert!(dragged_width > initial_width);

    app.stop_split_drag();
    assert!(!app.split_drag_active);
}

#[test]
fn refresh_keeps_latest_thought_per_session_in_timestamp_order() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-2",
            "beta",
            TEST_REPO_BETA,
            "indexing repo",
            "2026-03-08T14:00:05Z",
        ),
        session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
            "writing tests",
            "2026-03-08T14:00:06Z",
        ),
    ]));
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-2",
            "beta",
            TEST_REPO_BETA,
            "indexing repo",
            "2026-03-08T14:00:05Z",
        ),
        session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
            "patching sidebar",
            "2026-03-08T14:00:07Z",
        ),
    ]));
    let mut app = make_app(api);

    app.refresh(layout);
    app.refresh(layout);

    assert_eq!(
        app.thought_log
            .iter()
            .map(|entry| (entry.session_id.as_str(), entry.thought.as_str()))
            .collect::<Vec<_>>(),
        vec![("sess-2", "indexing repo"), ("sess-1", "patching sidebar"),]
    );
}

#[test]
fn refresh_updates_native_status_label_when_backend_app_changes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)]));
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    }));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));
    let mut app = make_app(api);

    app.refresh(layout);
    assert_eq!(app.native_status_text(), "native open: iTerm");

    app.refresh(layout);
    assert_eq!(app.native_status_text(), "native open: Ghostty (swap)");
}

#[test]
fn refresh_ignores_null_duplicate_and_stale_thoughts() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
        "sess-3",
        "gamma",
        TEST_REPO_GAMMA,
        "reading logs",
        "2026-03-08T14:00:05Z",
    )]));

    let mut duplicate = session_summary_with_thought(
        "sess-3",
        "gamma",
        TEST_REPO_GAMMA,
        "reading logs",
        "2026-03-08T14:00:05Z",
    );
    let mut stale = session_summary_with_thought(
        "sess-3",
        "gamma",
        TEST_REPO_GAMMA,
        "reading logs",
        "2026-03-08T14:00:04Z",
    );
    let mut cleared = session_summary("sess-3", "gamma", TEST_REPO_GAMMA);
    duplicate.last_activity_at = timestamp("2026-03-08T14:00:06Z");
    stale.last_activity_at = timestamp("2026-03-08T14:00:07Z");
    cleared.last_activity_at = timestamp("2026-03-08T14:00:08Z");

    api.push_fetch_sessions(Ok(vec![duplicate]));
    api.push_fetch_sessions(Ok(vec![stale]));
    api.push_fetch_sessions(Ok(vec![cleared]));

    let mut app = make_app(api);
    app.refresh(layout);
    app.refresh(layout);
    app.refresh(layout);
    app.refresh(layout);

    assert_eq!(app.thought_log.len(), 1);
    assert_eq!(app.thought_log[0].thought, "reading logs");
}

#[test]
fn selection_changes_do_not_reset_global_thought_timeline() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![
            session_summary("sess-1", "alpha", TEST_REPO_ALPHA),
            session_summary("sess-2", "beta", TEST_REPO_BETA),
        ],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
            "patching sidebar",
            "2026-03-08T14:00:07Z",
        )],
        layout.thought_entry_capacity(),
    );
    app.selected_id = Some("sess-1".to_string());
    let before = app.thought_log.clone();

    app.move_selection(1, layout.overview_field);

    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
    assert_eq!(app.thought_log, before);
}

#[test]
fn thought_timeline_trims_to_visible_capacity() {
    let api = MockApi::new();
    let layout = test_layout(120, 24);
    let mut app = make_app(api);
    assert_eq!(layout.thought_entry_capacity(), 10);

    for idx in 0..15 {
        let second = idx + 1;
        let updated_at = format!("2026-03-08T14:00:{second:02}Z");
        let thought = format!("thought {idx}");
        let session_id = format!("sess-{idx}");
        let tmux_name = format!("alpha-{idx}");
        let session = session_summary_with_thought(
            &session_id,
            &tmux_name,
            TEST_REPO_ALPHA,
            &thought,
            &updated_at,
        );
        app.capture_thought_updates(&[session], layout.thought_entry_capacity());
    }

    assert_eq!(app.thought_log.len(), 10);
    assert_eq!(
        app.thought_log.first().map(|entry| entry.thought.as_str()),
        Some("thought 5")
    );
    assert_eq!(
        app.thought_log.last().map(|entry| entry.thought.as_str()),
        Some("thought 14")
    );
}

#[test]
fn header_filter_strip_uses_active_sessions_not_trimmed_thought_log() {
    let api = MockApi::new();
    let layout = test_layout(220, 24);
    let mut app = make_app(api);
    assert_eq!(layout.thought_entry_capacity(), 10);

    let sessions = (0..11)
        .map(|idx| {
            let session_id = format!("sess-{idx:02}");
            let tmux_name = format!("{idx:02}");
            let cwd = format!("{TEST_REPOS_ROOT}/r{idx:02}");
            let thought = format!("thought {idx}");
            let updated_at = format!("2026-03-08T14:00:{:02}Z", idx + 1);
            session_summary_with_thought(&session_id, &tmux_name, &cwd, &thought, &updated_at)
        })
        .collect::<Vec<_>>();

    app.merge_sessions(sessions.clone(), layout.overview_field);
    app.capture_thought_updates(&sessions, layout.thought_entry_capacity());

    assert_eq!(app.thought_log.len(), 10);
    assert!(!app
        .thought_log
        .iter()
        .any(|entry| entry.cwd == format!("{TEST_REPOS_ROOT}/r00")));

    let header = build_header_filter_layout(&app, 220);
    let labels = header
        .chips
        .iter()
        .map(|chip| chip.label.clone())
        .collect::<Vec<_>>();
    assert_eq!(labels.len(), 11);
    assert!(labels.contains(&"1xr00".to_string()));
    assert!(labels.contains(&"1xr10".to_string()));
}

#[test]
fn refresh_prunes_exited_sessions_from_thought_timeline_and_header_filter_chips() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        ),
        session_summary_with_thought(
            "sess-2",
            "9",
            TEST_REPO_SKILLS,
            "indexing docs",
            "2026-03-08T14:00:06Z",
        ),
    ]));
    api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
        "sess-2",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:06Z",
    )]));
    let mut app = make_app(api);

    app.refresh(layout);
    let initial_header = build_header_filter_layout(&app, 120);
    assert!(initial_header
        .chips
        .iter()
        .any(|chip| chip.label == "1xswimmers"));
    assert!(initial_header
        .chips
        .iter()
        .any(|chip| chip.label == "1xskills"));

    app.refresh(layout);

    assert_eq!(
        app.thought_log
            .iter()
            .map(|entry| entry.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["sess-2"]
    );

    let header = build_header_filter_layout(&app, 120);
    assert_eq!(
        header
            .chips
            .iter()
            .map(|chip| chip.label.as_str())
            .collect::<Vec<_>>(),
        vec!["1xskills"]
    );
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec!["skills/9: indexing docs"]
    );
}

#[test]
fn refresh_header_filter_strip_includes_active_repo_without_thought_history() {
    let api = MockApi::new();
    let layout = test_layout(160, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        ),
        session_summary("sess-2", "9", TEST_REPO_SKILLS),
    ]));
    let mut app = make_app(api);

    app.refresh(layout);

    let header = build_header_filter_layout(&app, 160);
    let labels = header
        .chips
        .iter()
        .map(|chip| chip.label.clone())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"1xswimmers".to_string()));
    let skills_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist even without thought history")
        .clone();

    app.handle_header_filter_click(160, skills_chip.rect.x, skills_chip.rect.y);

    assert_eq!(app.thought_filter.cwd.as_deref(), Some(TEST_REPO_SKILLS));
}

#[test]
fn render_header_filter_strip_shows_repo_chips_and_thought_rows() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let swimmers_theme_id = "/tmp/swimmers".to_string();
    let skills_theme_id = "/tmp/skills".to_string();
    let swimmers_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    let skills_color = Color::Rgb {
        r: 79,
        g: 166,
        b: 106,
    };
    app.repo_themes
        .insert(swimmers_theme_id.clone(), repo_theme("#B89875"));
    app.repo_themes
        .insert(skills_theme_id.clone(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some(swimmers_theme_id.clone());

    let mut second = session_summary_with_thought(
        "sess-2",
        "2",
        TEST_REPO_SWIMMERS,
        "wiring filter state",
        "2026-03-08T14:00:06Z",
    );
    second.repo_theme_id = Some(swimmers_theme_id);

    let mut third = session_summary_with_thought(
        "sess-3",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    third.repo_theme_id = Some(skills_theme_id);

    app.merge_sessions(
        vec![first.clone(), second.clone(), third.clone()],
        layout.overview_field,
    );
    app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "swimmers/7: patching tui",
            "swimmers/2: wiring filter state",
            "skills/9: indexing docs",
        ]
    );

    let header = build_header_filter_layout(&app, 120);
    let swimmers_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "2xswimmers")
        .expect("swimmers chip should exist");
    let skills_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist");
    assert_eq!(swimmers_chip.color, swimmers_color);
    assert_eq!(skills_chip.color, skills_color);

    let mut renderer = test_renderer(120, 32);
    render_header_filter_strip(&app, &mut renderer, 120);

    assert_eq!(
        cell_at(&renderer, swimmers_chip.rect.x, swimmers_chip.rect.y).fg,
        swimmers_color
    );
    assert_eq!(
        cell_at(&renderer, skills_chip.rect.x, skills_chip.rect.y).fg,
        skills_color
    );
    assert!(row_text(&renderer, 2).contains("[filter out]"));
    assert!(row_text(&renderer, 2).ends_with("[filter out]  1xskills  2xswimmers"));
}

#[test]
fn active_repo_header_chip_maps_to_code_open_action() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    let session = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    app.merge_sessions(vec![session.clone()], layout.overview_field);
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());
    app.set_thought_filter_cwd(TEST_REPO_SWIMMERS.to_string());

    let header = build_header_filter_layout(&app, 120);
    let active_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "code .")
        .expect("active repo chip should expose code dot")
        .clone();

    assert_eq!(
        header_filter_action_at(&app, 120, active_chip.rect.x, active_chip.rect.y),
        Some(ThoughtPanelAction::OpenRepoInEditor(
            TEST_REPO_SWIMMERS.to_string()
        ))
    );
}

#[test]
fn header_filter_strip_and_thought_rows_apply_and_clear_filters() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());

    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    app.repo_themes
        .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some("/tmp/swimmers".to_string());

    let mut second = session_summary_with_thought(
        "sess-2",
        "2",
        TEST_REPO_SWIMMERS,
        "wiring filter state",
        "2026-03-08T14:00:06Z",
    );
    second.repo_theme_id = Some("/tmp/swimmers".to_string());

    let mut third = session_summary_with_thought(
        "sess-3",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    third.repo_theme_id = Some("/tmp/skills".to_string());

    app.merge_sessions(
        vec![first.clone(), second.clone(), third.clone()],
        layout.overview_field,
    );
    app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

    let initial_header = build_header_filter_layout(&app, 120);
    let chip = initial_header
        .chips
        .iter()
        .find(|chip| chip.label == "2xswimmers")
        .expect("swimmers chip should exist")
        .clone();
    app.handle_header_filter_click(120, chip.rect.x, chip.rect.y);

    assert_eq!(app.thought_filter.cwd.as_deref(), Some(TEST_REPO_SWIMMERS));
    assert_eq!(app.active_thought_filter_text(), "filter: pwd=swimmers");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "2"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-2".to_string(), "sess-1".to_string()]
    );

    let filtered_header = build_header_filter_layout(&app, 120);
    let active_chip = filtered_header
        .chips
        .iter()
        .find(|chip| chip.label == "code .")
        .expect("active repo chip should become code dot");
    let dimmed_chip = filtered_header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("inactive repo chip should stay visible");
    assert_eq!(dimmed_chip.color, Color::DarkGrey);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(!row_text(&renderer, 1).contains("filter: pwd"));
    assert_eq!(
        cell_at(&renderer, active_chip.rect.x, active_chip.rect.y).fg,
        active_chip.color
    );
    assert_eq!(
        cell_at(&renderer, dimmed_chip.rect.x, dimmed_chip.rect.y).fg,
        Color::DarkGrey
    );
    assert!(row_text(&renderer, 2).contains("code ."));
    assert!(row_text(&renderer, 2).contains("1xskills"));
    assert!(row_text(&renderer, 2).contains("[filter out]"));
    assert!(row_text(&renderer, 2).contains("[clear filters]"));

    let filtered_panel =
        build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_index = filtered_panel
        .rows
        .iter()
        .position(|row| row.tmux_name == "2")
        .expect("session 2 row should exist");
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(filtered_panel.rows.len() as u16);
    let row_rect = filtered_panel.rows[row_index]
        .text_rect
        .expect("row should have a click target");
    app.selected_id = Some("sess-3".to_string());
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-2".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    app.handle_thought_click(
        row_rect.x.saturating_add(4),
        row_start_y + row_index as u16,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert_eq!(app.thought_filter.cwd.as_deref(), Some(TEST_REPO_SWIMMERS));
    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: pwd=swimmers");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "2"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-2".to_string(), "sess-1".to_string()]
    );
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
    assert_eq!(api.open_calls(), vec!["sess-2".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/2")
    );

    let cleared_header = build_header_filter_layout(&app, 120);
    let clear_rect = cleared_header
        .clear_filters_rect
        .expect("clear filters button should exist");
    app.handle_header_filter_click(120, clear_rect.x, clear_rect.y);

    assert_eq!(app.thought_filter, ThoughtFilter::default());
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "2", "9"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec![
            "sess-2".to_string(),
            "sess-1".to_string(),
            "sess-3".to_string(),
        ]
    );
}

#[test]
fn header_filter_strip_toggles_filter_out_mode_and_excludes_selected_projects() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    app.repo_themes
        .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some("/tmp/swimmers".to_string());

    let mut second = session_summary_with_thought(
        "sess-2",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    second.repo_theme_id = Some("/tmp/skills".to_string());

    app.merge_sessions(vec![first.clone(), second.clone()], layout.overview_field);
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

    let initial_header = build_header_filter_layout(&app, 120);
    let filter_out_rect = initial_header
        .filter_out_rect
        .expect("filter out toggle should exist");
    assert_eq!(
        header_filter_action_at(&app, 120, filter_out_rect.x, filter_out_rect.y),
        Some(ThoughtPanelAction::ToggleFilterOutMode)
    );

    app.handle_header_filter_click(120, filter_out_rect.x, filter_out_rect.y);

    assert!(app.thought_filter.filter_out_mode);
    assert_eq!(app.active_thought_filter_text(), "filter: none");

    let filter_out_header = build_header_filter_layout(&app, 120);
    let skills_chip = filter_out_header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist")
        .clone();
    assert_eq!(
        header_filter_action_at(&app, 120, skills_chip.rect.x, skills_chip.rect.y),
        Some(ThoughtPanelAction::ToggleFilterOutCwd(
            TEST_REPO_SKILLS.to_string()
        ))
    );

    app.handle_header_filter_click(120, skills_chip.rect.x, skills_chip.rect.y);

    assert!(app.thought_filter.filter_out_mode);
    assert!(app.thought_filter.excluded_cwds.contains(TEST_REPO_SKILLS));
    assert_eq!(app.active_thought_filter_text(), "filter: hide=skills");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7"]
    );
    assert_eq!(visible_entity_ids(&app), vec!["sess-1".to_string()]);

    let excluded_header = build_header_filter_layout(&app, 120);
    let excluded_chip = excluded_header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should stay visible");
    assert_eq!(excluded_chip.color, Color::DarkGrey);

    let clear_rect = excluded_header
        .clear_filters_rect
        .expect("clear filters button should exist");
    app.handle_header_filter_click(120, clear_rect.x, clear_rect.y);

    assert_eq!(app.thought_filter, ThoughtFilter::default());
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "9"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-1".to_string(), "sess-2".to_string()]
    );
}

#[test]
fn clicking_thought_body_opens_that_session() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let line = panel.rows[0].line.clone();
    let body_x = panel.rows[0]
        .text_rect
        .expect("row should have text")
        .x
        .saturating_add(1);
    assert!(body_x < thought_content.x.saturating_add(display_width(&line)));

    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-1".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    app.handle_thought_click(
        body_x,
        row_start_y,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/7")
    );
}

#[test]
fn wrapped_latest_thought_stays_bottom_aligned() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let thought_content = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 5,
    };

    app.capture_thought_updates(
        &[
            session_summary_with_thought(
                "sess-1",
                "7",
                TEST_REPO_SWIMMERS,
                "older",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-2",
                "9",
                TEST_REPO_SWIMMERS,
                "latest thought stays at bottom",
                "2026-03-08T14:00:06Z",
            ),
        ],
        4,
    );

    let panel = build_thought_panel(&app, thought_content, 4);

    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec!["latest", "thought", "stays at", "bottom"]
    );
    assert_eq!(
        panel.rows.last().map(|row| row.line.as_str()),
        Some("bottom")
    );
}

#[test]
fn clicking_wrapped_thought_line_opens_that_session() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    let thought_content = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 5,
    };
    app.merge_sessions(
        vec![session_summary("sess-2", "9", TEST_REPO_SWIMMERS)],
        test_field(),
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-2",
            "9",
            TEST_REPO_SWIMMERS,
            "latest thought stays at bottom",
            "2026-03-08T14:00:06Z",
        )],
        4,
    );

    let panel = build_thought_panel(&app, thought_content, 4);
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-2".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    app.handle_thought_click(1, row_start_y + 3, thought_content, 4);

    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
    assert_eq!(api.open_calls(), vec!["sess-2".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/9")
    );
}

#[test]
fn clicking_thought_row_surfaces_native_open_errors() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let body_x = panel.rows[0]
        .text_rect
        .expect("row should have text")
        .x
        .saturating_add(1);

    api.push_open_session(Err("native open unavailable".to_string()));
    app.handle_thought_click(
        body_x,
        row_start_y,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("native open unavailable")
    );
    assert_eq!(app.active_thought_filter_text(), "filter: none");
}

#[test]
fn repo_theme_colors_override_state_colors_in_thought_history() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let theme_id = "/tmp/buildooor".to_string();
    let theme_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    app.repo_themes.insert(
        theme_id.clone(),
        RepoTheme {
            body: "#B89875".to_string(),
            outline: "#3D2F24".to_string(),
            accent: "#1D1914".to_string(),
            shirt: "#AA9370".to_string(),
        },
    );

    let mut busy = session_summary_with_thought(
        "sess-1",
        "alpha",
        TEST_REPO_ALPHA,
        "indexing repo",
        "2026-03-08T14:00:05Z",
    );
    busy.state = SessionState::Busy;
    busy.repo_theme_id = Some(theme_id.clone());

    let mut attention = session_summary_with_thought(
        "sess-1",
        "alpha",
        TEST_REPO_ALPHA,
        "needs input",
        "2026-03-08T14:00:06Z",
    );
    attention.state = SessionState::Attention;
    attention.repo_theme_id = Some(theme_id);

    app.capture_thought_updates(&[busy], layout.thought_entry_capacity());
    app.capture_thought_updates(&[attention], layout.thought_entry_capacity());

    assert_eq!(
        app.thought_log
            .iter()
            .map(|entry| entry.color)
            .collect::<Vec<_>>(),
        vec![theme_color]
    );

    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    assert_eq!(panel.rows.len(), 1);
    assert_eq!(cell_at(&renderer, thought_content.x, row_start_y).ch, 'a');
    assert_eq!(
        cell_at(&renderer, thought_content.x, row_start_y).fg,
        theme_color
    );
}

#[test]
fn low_contrast_repo_theme_color_is_adjusted_in_thought_history_and_header() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    let theme_id = "/tmp/skills".to_string();
    let raw_color = rgb_color((0x39, 0x30, 0xB5));
    let expected = repo_theme_display_color("#3930B5").expect("display color");
    app.repo_themes
        .insert(theme_id.clone(), repo_theme("#3930B5"));

    let mut session = session_summary_with_thought(
        "sess-1",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    session.state = SessionState::Busy;
    session.repo_theme_id = Some(theme_id);

    app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
    app.merge_sessions(vec![session], layout.overview_field);

    assert_ne!(expected, raw_color);
    assert_dark_terminal_readable(expected);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 1);
    assert_eq!(panel.rows[0].color, expected);

    let header = build_header_filter_layout(&app, 120);
    let chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist");
    assert_eq!(chip.color, expected);

    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );
    assert_eq!(
        cell_at(
            &renderer,
            thought_content.x,
            thought_content.bottom().saturating_sub(1)
        )
        .fg,
        expected
    );

    render_header_filter_strip(&app, &mut renderer, 120);
    assert_eq!(cell_at(&renderer, chip.rect.x, chip.rect.y).fg, expected);
}

#[test]
fn thought_history_rows_follow_live_session_color() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let mut session = session_summary_with_thought(
        "sess-1",
        "alpha",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    session.state = SessionState::Busy;

    app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
    app.merge_sessions(vec![session.clone()], layout.overview_field);

    session.state = SessionState::Attention;
    session.last_activity_at = timestamp("2026-03-08T14:00:06Z");
    app.merge_sessions(vec![session], layout.overview_field);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let header = build_header_filter_layout(&app, 120);
    let chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xswimmers")
        .expect("repo chip should exist");

    assert_eq!(panel.rows.len(), 1);
    // Without a repo theme the color is derived from the tmux name, so it stays
    // stable across state transitions.
    let expected = name_based_color("alpha");
    assert_eq!(panel.rows[0].color, expected);
    assert_eq!(chip.color, expected);
}

#[test]
fn render_entity_uses_repo_theme_body_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_BUILDOOOR);
    session.state = SessionState::Busy;
    session.repo_theme_id = Some("/tmp/buildooor".to_string());
    let entity = SessionEntity::new(session, field);
    let mut repo_themes = HashMap::new();
    repo_themes.insert(
        "/tmp/buildooor".to_string(),
        RepoTheme {
            body: "#B89875".to_string(),
            outline: "#3D2F24".to_string(),
            accent: "#1D1914".to_string(),
            shirt: "#AA9370".to_string(),
        },
    );
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, false, 0, &repo_themes);

    assert_eq!(
        cell_at(&renderer, rect.x, rect.y).fg,
        Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        }
    );
}

#[test]
fn render_entity_adjusts_low_contrast_repo_theme_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_SKILLS);
    session.state = SessionState::Busy;
    session.repo_theme_id = Some("/tmp/skills".to_string());
    let entity = SessionEntity::new(session, field);
    let mut repo_themes = HashMap::new();
    repo_themes.insert("/tmp/skills".to_string(), repo_theme("#3930B5"));
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);
    let expected = session_display_color(&entity.session, &repo_themes);

    render_entity(&mut renderer, &entity, rect, false, 0, &repo_themes);

    assert_ne!(expected, rgb_color((0x39, 0x30, 0xB5)));
    assert_dark_terminal_readable(expected);
    assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, expected);
}

#[test]
fn selected_entity_preserves_repo_theme_body_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_BUILDOOOR);
    session.state = SessionState::Busy;
    session.repo_theme_id = Some("/tmp/buildooor".to_string());
    let entity = SessionEntity::new(session, field);
    let mut repo_themes = HashMap::new();
    repo_themes.insert("/tmp/buildooor".to_string(), repo_theme("#B89875"));
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, true, 0, &repo_themes);

    assert_eq!(
        cell_at(&renderer, rect.x, rect.y).fg,
        Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        }
    );
    assert_eq!(cell_at(&renderer, rect.x - 1, rect.y + 1).fg, Color::White);
    assert_eq!(
        cell_at(&renderer, rect.x, rect.y + SPRITE_HEIGHT).fg,
        Color::White
    );
}

#[test]
fn selected_entity_preserves_fallback_state_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_SWIMMERS);
    session.state = SessionState::Attention;
    session.rest_state = RestState::Active;
    let expected = name_based_color("alpha");
    let entity = SessionEntity::new(session, field);
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, true, 0, &HashMap::new());

    assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, expected);
    assert_eq!(cell_at(&renderer, rect.x - 1, rect.y + 1).fg, Color::White);
    assert_eq!(
        cell_at(&renderer, rect.x, rect.y + SPRITE_HEIGHT).fg,
        Color::White
    );
}

#[test]
fn spawned_selected_entity_matches_thought_color() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let field = layout.overview_field;
    let theme_id = "/tmp/swimmers".to_string();
    let theme_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    let mut spawned_session = session_summary("sess-42", "42", TEST_REPO_SWIMMERS);
    spawned_session.repo_theme_id = Some(theme_id.clone());
    api.push_create_session(Ok(create_response_with_theme(
        spawned_session.clone(),
        repo_theme("#B89875"),
    )));
    let mut app = make_app(api);

    app.spawn_session(TEST_REPO_SWIMMERS, None, field);

    let mut thought_session = session_summary_with_thought(
        "sess-42",
        "42",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    thought_session.repo_theme_id = Some(theme_id);
    app.capture_thought_updates(&[thought_session.clone()], layout.thought_entry_capacity());
    app.merge_sessions(vec![thought_session], field);

    let entity = app
        .selected()
        .expect("spawned session should be selected")
        .clone();
    let rect = entity.screen_rect(field);
    let mut entity_renderer = test_renderer(120, 32);
    render_entity(
        &mut entity_renderer,
        &entity,
        rect,
        true,
        0,
        &app.repo_themes,
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 1);
    assert_eq!(panel.rows[0].color, theme_color);

    let mut thought_renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut thought_renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    assert_eq!(cell_at(&entity_renderer, rect.x, rect.y).fg, theme_color);
    assert_eq!(
        cell_at(&thought_renderer, thought_content.x, row_start_y).fg,
        theme_color
    );
}

#[test]
fn sleeping_entity_pins_to_bottom_left_grid_slot() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![sleeping_session(
            "sess-sleep-1",
            "7",
            TEST_REPO_SWIMMERS,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-sleep-1", field),
        sleep_grid_rect(field, 0)
    );
}

#[test]
fn attention_sleeping_entity_pins_to_bottom_left_grid_slot() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![attention_session(
            "sess-attn-sleep-1",
            "7",
            TEST_REPO_SWIMMERS,
            RestState::Sleeping,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    let entity = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-attn-sleep-1")
        .expect("entity should exist");
    assert_eq!(entity.rest_anchor(), RestAnchor::Bottom);
    assert_eq!(
        entity_rect_for(&app, "sess-attn-sleep-1", field),
        sleep_grid_rect(field, 0)
    );
}

#[test]
fn deep_sleep_entity_floats_to_top_left_grid_slot() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![deep_sleep_session(
            "sess-deep-1",
            "7",
            TEST_REPO_SWIMMERS,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    let entity = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-deep-1")
        .expect("entity should exist");
    assert_eq!(entity.rest_anchor(), RestAnchor::Top);
    assert_eq!(
        entity_rect_for(&app, "sess-deep-1", field),
        deep_sleep_grid_rect(field, 0)
    );
}

#[test]
fn attention_session_state_text_uses_rest_state() {
    let active = attention_session(
        "sess-attn-active",
        "7",
        TEST_REPO_SWIMMERS,
        RestState::Active,
        "2026-03-08T12:40:00Z",
    );
    let drowsy = attention_session(
        "sess-attn-drowsy",
        "8",
        TEST_REPO_SWIMMERS,
        RestState::Drowsy,
        "2026-03-08T12:20:00Z",
    );
    let sleeping = attention_session(
        "sess-attn-sleep",
        "9",
        TEST_REPO_SWIMMERS,
        RestState::Sleeping,
        "2026-03-08T12:00:00Z",
    );
    let deep_sleep = attention_session(
        "sess-attn-deep",
        "10",
        TEST_REPO_SWIMMERS,
        RestState::DeepSleep,
        "2026-03-08T11:00:00Z",
    );

    assert_eq!(session_state_text(&active), "attention");
    assert_eq!(session_state_text(&drowsy), "drowsy");
    assert_eq!(session_state_text(&sleeping), "sleeping");
    assert_eq!(session_state_text(&deep_sleep), "deep sleep");
}

#[test]
fn render_picker_uses_current_repo_theme_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("buildooor");
    fs::create_dir_all(&repo_root).expect("create repo");
    write_repo_theme_file(&repo_root, "#B89875");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(repo_root.to_string_lossy().as_ref(), &[("src", true)]),
        true,
        SpawnTool::Codex,
    );
    let mut repo_themes = HashMap::new();
    picker.sync_theme_colors(&mut repo_themes);

    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    let expected = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    assert_eq!(
        cell_at(&renderer, layout.frame.x, layout.frame.y).fg,
        expected
    );
    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.content.y).fg,
        expected
    );
    assert_eq!(
        cell_at(
            &renderer,
            layout.spawn_here_button.x,
            layout.spawn_here_button.y
        )
        .fg,
        expected
    );
}

#[test]
fn picker_theme_color_for_path_keeps_stored_theme_body_while_adjusting_display_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("skills");
    fs::create_dir_all(repo_root.join("src")).expect("create repo");
    write_repo_theme_file(&repo_root, "#3930B5");
    let colors_path = repo_root.join(".swimmers").join("colors.json");
    let original = fs::read_to_string(&colors_path).expect("read colors.json");
    let theme_id = repo_root.to_string_lossy().into_owned();
    let mut repo_themes = HashMap::new();

    let color = picker_theme_color_for_path(theme_id.as_str(), &mut repo_themes)
        .expect("theme color should resolve");

    assert_ne!(color, rgb_color((0x39, 0x30, 0xB5)));
    assert_dark_terminal_readable(color);
    assert_eq!(
        repo_themes
            .get(theme_id.as_str())
            .expect("theme should be cached")
            .body,
        "#3930B5"
    );
    assert_eq!(
        fs::read_to_string(colors_path).expect("reread colors.json"),
        original
    );
}

#[test]
fn render_picker_adjusts_low_contrast_repo_theme_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("skills");
    fs::create_dir_all(repo_root.join("src")).expect("create repo");
    write_repo_theme_file(&repo_root, "#3930B5");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(repo_root.to_string_lossy().as_ref(), &[("src", true)]),
        true,
        SpawnTool::Codex,
    );
    let mut repo_themes = HashMap::new();
    picker.sync_theme_colors(&mut repo_themes);

    let expected = picker.current_theme_color.expect("current theme color");
    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    assert_ne!(expected, rgb_color((0x39, 0x30, 0xB5)));
    assert_dark_terminal_readable(expected);
    assert_eq!(picker.entry_theme_colors, vec![Some(expected)]);
    assert_eq!(
        cell_at(&renderer, layout.frame.x, layout.frame.y).fg,
        expected
    );
    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.content.y + 1).fg,
        expected
    );
    assert_eq!(
        cell_at(
            &renderer,
            layout.spawn_here_button.x,
            layout.spawn_here_button.y
        )
        .fg,
        expected
    );
    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.first_entry_y).fg,
        expected
    );
}

#[test]
fn render_picker_uses_entry_repo_theme_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("swimmers");
    fs::create_dir_all(&repo_root).expect("create repo");
    write_repo_theme_file(&repo_root, "#4FA66A");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(
            temp.path().to_string_lossy().as_ref(),
            &[("swimmers", true)],
        ),
        true,
        SpawnTool::Codex,
    );
    let mut repo_themes = HashMap::new();
    picker.sync_theme_colors(&mut repo_themes);

    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.first_entry_y).fg,
        Color::Rgb {
            r: 79,
            g: 166,
            b: 106,
        }
    );
}

#[test]
fn sleeping_entities_fill_bottom_row_by_sleepiness() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            sleeping_session("sess-new", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:20:00Z"),
            sleeping_session("sess-mid", "7", TEST_REPO_SWIMMERS, "2026-03-08T12:10:00Z"),
            sleeping_session("sess-old", "9", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
        ],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-old", field),
        sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-mid", field),
        sleep_grid_rect(field, 1)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-new", field),
        sleep_grid_rect(field, 2)
    );
}

#[test]
fn sleeping_entities_use_tmux_name_tiebreaker() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            sleeping_session("sess-b", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
            sleeping_session("sess-a", "7", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
        ],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-a", field),
        sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-b", field),
        sleep_grid_rect(field, 1)
    );
}

#[test]
fn existing_entity_relocates_into_sleep_grid_when_it_falls_asleep() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    app.entities
        .push(entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8));

    app.merge_sessions(
        vec![sleeping_session(
            "sess-1",
            "dev",
            TEST_REPO_DEV,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-1", field),
        sleep_grid_rect(field, 0)
    );
}

#[test]
fn sleeping_entities_stay_fixed_after_tick() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            sleeping_session("sess-a", "7", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
            sleeping_session("sess-b", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:10:00Z"),
        ],
        field,
    );
    for entity in &mut app.entities {
        entity.vx = 1.0;
        entity.vy = 1.0;
    }

    app.tick(field);

    assert_eq!(
        entity_rect_for(&app, "sess-a", field),
        sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-b", field),
        sleep_grid_rect(field, 1)
    );
}

#[test]
fn drowsy_sprite_uses_fish_motion_profile() {
    assert_eq!(SpriteKind::Drowsy.speed_scale(), 0.5);
    assert!(drowsy_frame(0)[1].contains("><-"));
}

#[test]
fn drowsy_entities_bob_in_place_after_tick() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
    entity.session.thought_state = ThoughtState::Holding;
    entity.session.rest_state = RestState::Drowsy;
    entity.bob_phase = 0.0;
    entity.vx = 1.0;
    entity.vy = 0.0;
    app.entities.push(entity);

    for _ in 0..16 {
        app.tick(field);
    }

    let rect = entity_rect_for(&app, "sess-1", field);
    assert_eq!(rect.x, 30);
    assert_ne!(rect.y, 8);
    assert!((rect.y as i32 - 8).abs() <= 3);
}

#[test]
fn deep_sleep_entities_stay_fixed_after_tick() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            deep_sleep_session(
                "sess-deep-a",
                "7",
                TEST_REPO_SWIMMERS,
                "2026-03-08T12:00:00Z",
            ),
            deep_sleep_session(
                "sess-deep-b",
                "8",
                TEST_REPO_SWIMMERS,
                "2026-03-08T12:10:00Z",
            ),
        ],
        field,
    );
    for entity in &mut app.entities {
        entity.vx = 1.0;
        entity.vy = 1.0;
    }

    app.tick(field);

    assert_eq!(
        entity_rect_for(&app, "sess-deep-a", field),
        deep_sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-deep-b", field),
        deep_sleep_grid_rect(field, 1)
    );
}

#[test]
fn active_entities_swim_in_place_with_bob() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
    entity.session.thought_state = ThoughtState::Active;
    entity.session.rest_state = RestState::Active;
    entity.bob_phase = 0.0;
    entity.vx = 1.0;
    entity.vy = 0.0;
    app.entities.push(entity);

    for _ in 0..16 {
        app.tick(field);
    }

    let moved = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-1")
        .expect("entity should exist");
    assert_eq!(moved.screen_rect(field).x, 30);
    assert_ne!(moved.screen_rect(field).y, 8);
    assert!((moved.screen_rect(field).y as i32 - 8).abs() <= 3);
}

#[test]
fn busy_entities_hold_horizontal_position() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
    entity.session.state = SessionState::Busy;
    entity.bob_phase = 0.0;
    entity.vx = 1.0;
    entity.vy = 0.0;
    app.entities.push(entity);

    for _ in 0..16 {
        app.tick(field);
    }

    let rect = entity_rect_for(&app, "sess-1", field);
    assert_eq!(rect.x, 30);
    assert_ne!(rect.y, 8);
    assert!((rect.y as i32 - 8).abs() <= 3);
}

#[test]
fn truncate_label_adds_trailing_tilde() {
    assert_eq!(truncate_label("abcdefghijkl", 6), "abcde~");
    assert_eq!(truncate_label("abc", 6), "abc");
}

#[test]
fn shorten_path_keeps_tail() {
    assert_eq!(shorten_path("/a/b/c/d/e", 8), ".../d/e");
    assert_eq!(shorten_path("/short", 20), "/short");
}

#[test]
fn intersects_detects_overlap() {
    let a = Rect {
        x: 0,
        y: 0,
        width: 5,
        height: 5,
    };
    let b = Rect {
        x: 4,
        y: 2,
        width: 5,
        height: 3,
    };
    let c = Rect {
        x: 5,
        y: 5,
        width: 2,
        height: 2,
    };
    assert!(intersects(a, b));
    assert!(!intersects(a, c));
}

#[test]
fn empty_field_click_opens_picker_with_managed_order() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(
        TEST_REPOS_ROOT,
        &[("opensource", true), ("swimmers", true)],
    )));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.entities
        .push(entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8));

    app.handle_field_click(10, 10, field);

    let picker = app.picker.as_ref().expect("picker should open");
    assert!(picker.managed_only);
    assert_eq!(picker.base_path, TEST_REPOS_ROOT);
    assert_eq!(
        picker
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["opensource", "swimmers"]
    );
    assert_eq!(api.list_calls(), vec![(None, true)]);
}

#[test]
fn navigating_into_folder_opens_initial_request_composer() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
    api.push_list_dirs(Ok(dir_response(TEST_REPO_OPENSOURCE, &[("skills", false)])));

    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    app.activate_picker_entry(0, field);
    app.activate_picker_entry(0, field);

    assert_eq!(
        api.list_calls(),
        vec![(None, true), (Some(TEST_REPO_OPENSOURCE.to_string()), true),]
    );
    assert_eq!(
        api.create_calls(),
        Vec::<(String, SpawnTool, Option<String>)>::new()
    );
    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some(TEST_REPO_SKILLS)
    );
    assert!(app.picker.is_some());
}

#[test]
fn spawn_here_opens_initial_request_for_current_path() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPO_OPENSOURCE, &[("skills", true)]),
        true,
        SpawnTool::Codex,
    ));

    app.spawn_session_from_picker(field);

    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some(TEST_REPO_OPENSOURCE)
    );
}

#[test]
fn toggling_to_all_reloads_same_path_without_reordering() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
    api.push_list_dirs(Ok(dir_response(
        TEST_REPOS_ROOT,
        &[("Alpha", true), ("beta", true), ("zzz-old", true)],
    )));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    app.picker_set_managed_only(false);

    let picker = app.picker.as_ref().expect("picker should stay open");
    assert!(!picker.managed_only);
    assert_eq!(
        picker
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "beta", "zzz-old"]
    );
    assert_eq!(
        api.list_calls(),
        vec![(None, true), (Some(TEST_REPOS_ROOT.to_string()), false),]
    );
}

#[test]
fn dir_list_failure_blocks_spawn_and_shows_error() {
    let api = MockApi::new();
    api.push_list_dirs(Err("Permission denied".to_string()));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);

    assert!(app.picker.is_none());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("Permission denied")
    );
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
}

#[test]
fn submitting_initial_request_creates_hidden_session_without_native_open() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]),
        true,
        SpawnTool::Codex,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "add hidden spawn flow".to_string(),
    });

    app.submit_initial_request(field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Codex,
            Some("add hidden spawn flow".to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
    assert!(app.picker.is_none());
    assert!(app.initial_request.is_none());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("created 55")
    );
    assert!(app
        .entities
        .iter()
        .any(|entity| entity.session.session_id == "sess-55"));
}

#[test]
fn pasting_initial_request_buffers_multiline_without_submitting() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: String::new(),
    });

    app.handle_paste(pasted);

    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some(pasted)
    );
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
}

#[test]
fn pressing_enter_after_pasting_initial_request_submits_once() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: String::new(),
    });

    app.handle_paste(pasted);
    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Codex,
            Some(pasted.to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
}

#[test]
fn session_create_failure_does_not_attempt_native_open() {
    let api = MockApi::new();
    api.push_create_session(Err("tmux failed to start".to_string()));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]),
        true,
        SpawnTool::Codex,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "fix tmux startup".to_string(),
    });

    app.submit_initial_request(field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Codex,
            Some("fix tmux startup".to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert!(app.entities.is_empty());
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("fix tmux startup")
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("tmux failed to start")
    );
}

#[test]
fn blank_initial_request_is_rejected_locally() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "   ".to_string(),
    });

    app.submit_initial_request(field);

    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("enter an initial request")
    );
}

#[test]
fn typing_initial_request_and_pressing_enter_still_creates_hidden_session() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: String::new(),
    });

    for ch in "add hidden spawn flow".chars() {
        app.handle_initial_request_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), field);
    }
    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Codex,
            Some("add hidden spawn flow".to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("created 55")
    );
}

#[test]
fn esc_cancels_initial_request_without_creating_session() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]),
        true,
        SpawnTool::Codex,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "investigate snapshot restore".to_string(),
    });

    app.handle_initial_request_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), field);

    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert!(app.picker.is_some());
}

#[test]
fn paste_outside_initial_request_is_ignored() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    app.selected_id = Some("sess-7".to_string());

    app.handle_paste("q\n### TC-7\n- Then: shell spill");

    assert_eq!(app.selected_id.as_deref(), Some("sess-7"));
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert!(app.picker.is_none());
}

#[test]
fn clicking_existing_swimmer_still_opens_it_directly() {
    let api = MockApi::new();
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-7".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.entities
        .push(entity_at(field, "sess-7", "dev", TEST_REPO_DEV, 30, 8));
    app.selected_id = Some("sess-7".to_string());

    app.handle_field_click(30, 8, field);

    assert!(api.list_calls().is_empty());
    assert!(api.create_calls().is_empty());
    assert_eq!(api.open_calls(), vec!["sess-7".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused dev")
    );
}

#[test]
fn filtered_out_swimmers_are_not_click_targets() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("swimmers", true)])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.entities
        .push(entity_at(field, "sess-1", "2", TEST_REPO_SWIMMERS, 12, 6));
    app.entities
        .push(entity_at(field, "sess-3", "9", TEST_REPO_SKILLS, 30, 8));
    app.selected_id = Some("sess-3".to_string());

    app.set_thought_filter_cwd(TEST_REPO_SWIMMERS.to_string());
    app.handle_field_click(30, 8, field);

    assert_eq!(visible_entity_ids(&app), vec!["sess-1".to_string()]);
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert!(api.open_calls().is_empty());
    assert!(app.picker.is_some());
}

#[test]
fn refresh_clears_selection_when_filters_hide_all_sessions() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-3", "9", TEST_REPO_SKILLS)]));
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![
            session_summary("sess-1", "7", TEST_REPO_SWIMMERS),
            session_summary("sess-2", "2", TEST_REPO_SWIMMERS),
        ],
        layout.overview_field,
    );
    app.selected_id = Some("sess-1".to_string());
    app.set_thought_filter_cwd(TEST_REPO_SWIMMERS.to_string());

    app.refresh(layout);

    assert!(app.visible_entities().is_empty());
    assert!(app.selected_id.is_none());
    assert_eq!(
        api.publish_calls(),
        vec![Some("sess-2".to_string()), Some("sess-1".to_string()), None,]
    );

    app.open_selected();

    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("no session selected")
    );
}

#[test]
fn refresh_publishes_selected_session_for_external_dispatch() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary(
        "sess-swimmers",
        "7",
        TEST_REPO_SWIMMERS,
    )]));
    let mut app = make_app(api.clone());

    app.refresh(layout);

    assert_eq!(app.selected_id.as_deref(), Some("sess-swimmers"));
    assert_eq!(api.publish_calls(), vec![Some("sess-swimmers".to_string())]);
}

#[test]
fn refresh_keeps_cached_repo_theme_when_session_still_references_it() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let theme_id = "/tmp/buildooor".to_string();
    let mut session = session_summary("sess-buildooor", "7", "/tmp/buildooor/src");
    session.repo_theme_id = Some(theme_id.clone());
    api.push_fetch_sessions(Ok(vec![session]));

    let mut app = make_app(api);
    app.repo_themes
        .insert(theme_id.clone(), repo_theme("#B89875"));

    app.refresh(layout);

    assert_eq!(
        app.repo_themes.get(&theme_id).expect("theme retained").body,
        "#B89875"
    );
    assert_eq!(
        session_display_color(&app.entities[0].session, &app.repo_themes),
        repo_theme_display_color("#B89875").expect("display color")
    );
}

#[test]
fn picker_action_at_resolves_controls_and_entries() {
    let mut picker = PickerState::new(
        4,
        4,
        dir_response("/tmp", &[("alpha", true), ("beta", false)]),
        true,
        SpawnTool::Codex,
    );
    picker.apply_response(dir_response("/tmp/nested", &[("child", false)]));
    let layout = picker_layout(&picker, test_field());

    assert!(matches!(
        picker_action_at(
            &picker,
            layout,
            layout.close_button.x,
            layout.close_button.y
        ),
        Some(PickerAction::Close)
    ));
    assert!(matches!(
        picker_action_at(&picker, layout, layout.env_button.x, layout.env_button.y),
        Some(PickerAction::ToggleManaged(true))
    ));
    assert!(matches!(
        picker_action_at(&picker, layout, layout.all_button.x, layout.all_button.y),
        Some(PickerAction::ToggleManaged(false))
    ));
    assert!(matches!(
        picker_action_at(
            &picker,
            layout,
            layout.spawn_here_button.x,
            layout.spawn_here_button.y
        ),
        Some(PickerAction::ActivateCurrentPath)
    ));
    assert!(matches!(
        picker_action_at(&picker, layout, layout.content.x, layout.first_entry_y),
        Some(PickerAction::ActivateEntry(0))
    ));
    assert!(matches!(
        picker_action_at(
            &picker,
            layout,
            layout.content.right(),
            layout.first_entry_y
        ),
        None
    ));
    assert!(matches!(
        layout
            .back_button
            .and_then(|button| picker_action_at(&picker, layout, button.x, button.y)),
        Some(PickerAction::Up)
    ));
    assert!(matches!(
        picker_action_at(&picker, layout, layout.tool_button.x, layout.tool_button.y),
        Some(PickerAction::ToggleTool)
    ));
}

#[test]
fn toggle_tool_switches_spawn_tool_and_persists_across_picker_reopen() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("swimmers", false)])));
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("swimmers", false)])));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    assert_eq!(app.spawn_tool, SpawnTool::Codex);
    assert_eq!(
        app.picker.as_ref().map(|p| p.spawn_tool),
        Some(SpawnTool::Codex)
    );

    app.handle_picker_action(PickerAction::ToggleTool, field);
    assert_eq!(app.spawn_tool, SpawnTool::Claude);
    assert_eq!(
        app.picker.as_ref().map(|p| p.spawn_tool),
        Some(SpawnTool::Claude)
    );

    app.close_picker();
    app.handle_field_click(10, 10, field);
    assert_eq!(
        app.picker.as_ref().map(|p| p.spawn_tool),
        Some(SpawnTool::Claude)
    );
}

#[test]
fn spawn_session_uses_selected_tool() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-99", "99", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.spawn_tool = SpawnTool::Claude;
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "fix the build".to_string(),
    });

    app.submit_initial_request(field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Claude,
            Some("fix the build".to_string()),
        )]
    );
}

#[test]
fn renderer_flush_copies_drawn_cells_into_last_buffer() {
    let mut renderer = test_renderer(4, 2);
    renderer.draw_char(0, 0, 'A', Color::Green);
    renderer.draw_char(1, 0, 'B', Color::Yellow);

    renderer.flush().expect("flush should succeed");

    assert!(renderer
        .buffer
        .iter()
        .zip(renderer.last_buffer.iter())
        .all(|(current, previous)| current == previous));
}

#[test]
fn move_selection_updates_picker_and_visible_session_selection() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![
            session_summary("sess-1", "1", TEST_REPO_ALPHA),
            session_summary("sess-2", "2", TEST_REPO_BETA),
        ],
        layout.overview_field,
    );

    app.move_selection(1, layout.overview_field);
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

    let mut picker = PickerState::new(
        3,
        3,
        dir_response("/tmp", &[("alpha", false), ("beta", false)]),
        true,
        SpawnTool::Codex,
    );
    picker.selection = PickerSelection::SpawnHere;
    app.picker = Some(picker);

    app.move_selection(1, layout.overview_field);

    assert!(matches!(
        app.picker.as_ref().map(|picker| picker.selection),
        Some(PickerSelection::Entry(0))
    ));
}

#[test]
fn handle_key_event_covers_initial_request_picker_and_quit_paths() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![
            session_summary("sess-1", "1", TEST_REPO_ALPHA),
            session_summary("sess-2", "2", TEST_REPO_BETA),
        ],
        layout.overview_field,
    );

    app.open_initial_request("/tmp/project".to_string());
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    ));
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("x")
    );

    app.close_initial_request();
    app.picker = Some(PickerState::new(
        3,
        3,
        dir_response("/tmp", &[("alpha", false)]),
        true,
        SpawnTool::Codex,
    ));
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(app.picker.is_none());

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    ));
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

    assert!(!handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ));
}

#[test]
fn handle_key_event_opens_thought_config_editor() {
    let api = MockApi::new();
    api.push_fetch_thought_config(Ok(ThoughtConfigResponse {
        config: ThoughtConfig {
            backend: "claude".to_string(),
            model: "haiku".to_string(),
            ..ThoughtConfig::default()
        },
        daemon_defaults: Some(DaemonDefaults {
            model: "haiku".to_string(),
            backend: "claude".to_string(),
            agent_prompt: "agent".to_string(),
            terminal_prompt: "terminal".to_string(),
        }),
        ui: swimmers::types::ThoughtConfigUiMetadata::default(),
    }));
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
    ));

    let editor = app
        .thought_config_editor
        .as_ref()
        .expect("thought config editor should open");
    assert_eq!(editor.config.backend, "openrouter");
    assert_eq!(editor.config.model, "openrouter/free");
}

#[test]
fn handle_key_event_toggles_native_app_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    });
    api.push_set_native_app(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
    ));

    assert_eq!(api.set_native_app_calls(), vec![NativeDesktopApp::Ghostty]);
    assert_eq!(
        app.native_status.as_ref().and_then(|status| status.app_id),
        Some(NativeDesktopApp::Ghostty)
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("native open target: Ghostty (swap)")
    );
}

#[test]
fn handle_key_event_toggles_ghostty_mode_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    });
    api.push_set_native_mode(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Add),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
    ));

    assert_eq!(api.set_native_mode_calls(), vec![GhosttyOpenMode::Add]);
    assert_eq!(
        app.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode),
        Some(GhosttyOpenMode::Add)
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("Ghostty preview mode: add")
    );
}

#[test]
fn thought_config_editor_updates_backend_and_model_then_saves() {
    let api = MockApi::new();
    api.push_fetch_thought_config(Ok(ThoughtConfigResponse {
        config: ThoughtConfig::default(),
        daemon_defaults: Some(DaemonDefaults {
            model: "openrouter/free".to_string(),
            backend: "openrouter".to_string(),
            agent_prompt: "agent".to_string(),
            terminal_prompt: "terminal".to_string(),
        }),
        ui: swimmers::types::ThoughtConfigUiMetadata::default(),
    }));
    api.push_update_thought_config(Ok(ThoughtConfig {
        backend: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        ..ThoughtConfig::default()
    }));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());

    app.open_thought_config_editor();
    assert!(app.thought_config_editor.is_some());

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    for ch in "gpt-5.4".chars() {
        handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );

    assert!(app.thought_config_editor.is_none());
    assert_eq!(api.update_thought_config_calls().len(), 1);
    let saved = api
        .update_thought_config_calls()
        .into_iter()
        .next()
        .expect("saved config");
    assert_eq!(saved.backend, "codex");
    assert_eq!(saved.model, "gpt-5.4");
    assert_eq!(api.test_thought_config_calls().len(), 1);
}

#[test]
fn thought_config_editor_test_button_probes_without_saving() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Test;
    }
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );

    assert!(app.thought_config_editor.is_some());
    assert!(api.update_thought_config_calls().is_empty());
    let tested = api
        .test_thought_config_calls()
        .into_iter()
        .next()
        .expect("tested config");
    assert_eq!(tested.backend, "openrouter");
    assert_eq!(tested.model, "openrouter/free");
}

#[test]
fn thought_config_editor_test_button_rotates_openrouter_model_after_invalid_model_error() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "old/expired:free".to_string(),
            ..ThoughtConfig::default()
        },
        Some(DaemonDefaults {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            agent_prompt: String::new(),
            terminal_prompt: String::new(),
        }),
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Test;
    }
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: false,
        message: "probe failed: old/expired:free is not a valid model ID".to_string(),
        last_backend_error: Some("old/expired:free is not a valid model ID".to_string()),
        llm_calls: 0,
    }));
    api.push_refresh_openrouter_candidates(Ok(vec![
        "openrouter/free".to_string(),
        "google/gemma-3-4b-it:free".to_string(),
    ]));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );

    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free")
    );
    assert!(app
        .visible_message()
        .unwrap_or_default()
        .contains("rotated to openrouter/free"));
}

#[test]
fn thought_config_editor_save_rotates_and_persists_openrouter_model_after_invalid_model_error() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "old/expired:free".to_string(),
            ..ThoughtConfig::default()
        },
        Some(DaemonDefaults {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            agent_prompt: String::new(),
            terminal_prompt: String::new(),
        }),
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Save;
    }
    api.push_update_thought_config(Ok(ThoughtConfig {
        backend: "openrouter".to_string(),
        model: "old/expired:free".to_string(),
        ..ThoughtConfig::default()
    }));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: false,
        message: "probe failed: old/expired:free is not a valid model ID".to_string(),
        last_backend_error: Some("old/expired:free is not a valid model ID".to_string()),
        llm_calls: 0,
    }));
    api.push_refresh_openrouter_candidates(Ok(vec![
        "openrouter/free".to_string(),
        "google/gemma-3-4b-it:free".to_string(),
    ]));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));
    api.push_update_thought_config(Ok(ThoughtConfig {
        backend: "openrouter".to_string(),
        model: "openrouter/free".to_string(),
        ..ThoughtConfig::default()
    }));

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );

    assert!(app.thought_config_editor.is_none());
    assert_eq!(api.update_thought_config_calls().len(), 2);
    assert_eq!(
        api.update_thought_config_calls()
            .last()
            .map(|config| config.model.as_str()),
        Some("openrouter/free")
    );
    assert!(app
        .visible_message()
        .unwrap_or_default()
        .contains("rotated to openrouter/free"));
}

#[test]
fn thought_config_editor_cycles_current_openrouter_model_presets() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: String::new(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Model;
        editor.config.model.clear();
        editor.replace_openrouter_model_presets(vec![
            "openrouter/free".to_string(),
            "nvidia/nemotron-3-super-120b-a12b:free".to_string(),
            "arcee-ai/trinity-large-preview:free".to_string(),
        ]);
    }

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free")
    );

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("nvidia/nemotron-3-super-120b-a12b:free")
    );

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("arcee-ai/trinity-large-preview:free")
    );
}

#[test]
fn thought_config_editor_clears_incompatible_model_when_backend_changes() {
    let mut editor = ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    );

    editor.cycle_backend(1);
    assert_eq!(editor.backend_label(), "codex");
    assert!(editor.config.model.is_empty());

    editor.config.model = "gpt-5.4".to_string();
    editor.cycle_backend(-1);
    assert_eq!(editor.backend_label(), "openrouter");
    assert!(editor.config.model.is_empty());
}

#[test]
fn picker_activate_selection_opens_initial_request_and_reloads_children() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        2,
        2,
        dir_response("/tmp", &[("child", true), ("leaf", false)]),
        true,
        SpawnTool::Codex,
    ));

    app.picker_activate_selection(layout.overview_field);
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some("/tmp")
    );

    app.close_initial_request();
    if let Some(picker) = &mut app.picker {
        picker.selection = PickerSelection::Entry(0);
    }
    api.push_list_dirs(Ok(dir_response("/tmp/child", &[("nested", false)])));
    app.picker_activate_selection(layout.overview_field);
    assert_eq!(
        api.list_calls(),
        vec![(Some("/tmp/child".to_string()), true)]
    );

    if let Some(picker) = &mut app.picker {
        picker.apply_response(dir_response("/tmp", &[("leaf", false)]));
        picker.selection = PickerSelection::Entry(0);
    }
    app.picker_activate_selection(layout.overview_field);
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some("/tmp/leaf")
    );
}

#[test]
fn handle_workspace_click_routes_thought_and_overview_interactions() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let body_x = panel.rows[0]
        .text_rect
        .expect("row should have text")
        .x
        .saturating_add(1);
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-1".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: body_x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/7")
    );

    let entity_rect = entity_rect_for(&app, "sess-1", layout.overview_field);
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-1".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: entity_rect.x,
            row: entity_rect.y,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(
        api.open_calls(),
        vec!["sess-1".to_string(), "sess-1".to_string()]
    );
}

#[test]
fn clicking_native_status_label_toggles_native_app_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    });
    api.push_set_native_app(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));
    let rect = app
        .native_status_rect(120)
        .expect("native status should render in header");

    assert!(handle_split_or_header_click(
        &mut app,
        120,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert_eq!(api.set_native_app_calls(), vec![NativeDesktopApp::Ghostty]);
    assert_eq!(
        app.native_status.as_ref().and_then(|status| status.app_id),
        Some(NativeDesktopApp::Ghostty)
    );
}

#[test]
fn clicking_ghostty_mode_label_toggles_preview_mode_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    });
    api.push_set_native_mode(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Add),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));
    let rect = app
        .ghostty_mode_rect(120)
        .expect("ghostty mode should render in header");

    assert!(handle_split_or_header_click(
        &mut app,
        120,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert_eq!(api.set_native_mode_calls(), vec![GhosttyOpenMode::Add]);
    assert_eq!(
        app.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode),
        Some(GhosttyOpenMode::Add)
    );
}

#[test]
fn clicking_commit_badge_launches_commit_codex_without_opening_session() {
    let api = MockApi::new();
    let launcher = Arc::new(MockCommitLauncher::default());
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app_with_commit_launcher(api.clone(), launcher.clone());
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.commit_candidate = true;
    app.merge_sessions(vec![session.clone()], layout.overview_field);
    let mut thought_session = session.clone();
    thought_session.thought = Some("ready to commit".to_string());
    thought_session.thought_updated_at = Some(
        DateTime::parse_from_rfc3339("2026-03-29T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc),
    );
    app.capture_thought_updates(&[thought_session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let commit_rect = panel.rows[0].commit_rect.expect("commit badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: commit_rect.x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );

    let launch_calls = launcher.calls();
    assert_eq!(api.open_calls(), Vec::<String>::new());
    assert_eq!(launch_calls.len(), 1);
    assert_eq!(launch_calls[0].session_id, session.session_id);
    assert_eq!(launch_calls[0].cwd, session.cwd);
    assert_eq!(launch_calls[0].tmux_name, session.tmux_name);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("commit codex: tmux a -t commit-7-123")
    );
}

#[test]
fn clicking_commit_badge_surfaces_commit_launch_errors() {
    let api = MockApi::new();
    let launcher = Arc::new(MockCommitLauncher::default());
    launcher.fail_with("tmux not found");
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app_with_commit_launcher(api, launcher);
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.commit_candidate = true;
    app.merge_sessions(vec![session], layout.overview_field);
    let mut thought_session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    thought_session.commit_candidate = true;
    thought_session.thought = Some("ready to commit".to_string());
    thought_session.thought_updated_at = Some(
        DateTime::parse_from_rfc3339("2026-03-29T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc),
    );
    app.capture_thought_updates(&[thought_session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let commit_rect = panel.rows[0].commit_rect.expect("commit badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: commit_rect.x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("failed to launch commit codex: tmux not found")
    );
}

#[test]
fn thought_panel_renders_shift_badge_for_objective_changes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    let mut session = session_summary_with_thought(
        "sess-shift",
        "2",
        TEST_REPO_SWIMMERS,
        "reframed the plan",
        "2026-03-29T14:00:05Z",
    );
    session.objective_changed_at = session.thought_updated_at;

    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let shift_rect = panel.rows[0].shift_rect.expect("shift badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert_eq!(cell_at(&renderer, shift_rect.x, row_y).ch, '[');
    assert_eq!(cell_at(&renderer, shift_rect.x, row_y).fg, Color::Yellow);
}

#[test]
fn objective_shift_entries_override_timestamp_order_in_the_visible_rail() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let mut shift = session_summary_with_thought(
        "sess-shift",
        "2",
        TEST_REPO_ALPHA,
        "reframed the plan",
        "2026-03-29T14:00:05Z",
    );
    shift.objective_changed_at = shift.thought_updated_at;

    let plain = session_summary_with_thought(
        "sess-plain",
        "9",
        TEST_REPO_SWIMMERS,
        "routine update",
        "2026-03-29T14:00:06Z",
    );

    app.capture_thought_updates(&[shift, plain], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let shift_index = panel
        .rows
        .iter()
        .position(|row| row.line == "alpha/2: reframed the plan")
        .expect("shift row");
    let plain_index = panel
        .rows
        .iter()
        .position(|row| row.line == "swimmers/9: routine update")
        .expect("plain row");

    assert!(shift_index > plain_index);
    assert!(panel.rows[shift_index].shift_rect.is_some());
}

#[test]
fn refresh_builds_synthetic_mermaid_row_and_preserves_text_click_behavior() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.commit_candidate = true;
    api.push_fetch_sessions(Ok(vec![session]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph TD\nA-->B\n",
    )));
    let mut app = make_app(api);

    app.refresh(layout);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 2);
    assert_eq!(panel.rows[0].line, "swimmers/7: mermaid");
    assert_eq!(panel.rows[1].line, "diagram ready");
    let mermaid_rect = panel.rows[0].mermaid_rect.expect("mermaid button");
    let commit_rect = panel.rows[0].commit_rect.expect("commit badge");
    let text_rect = panel.rows[0].text_rect.expect("synthetic row text");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            mermaid_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenMermaid("sess-1".to_string()))
    );
    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            commit_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::LaunchCommitCodex("sess-1".to_string()))
    );
    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            text_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenSession {
            session_id: "sess-1".to_string(),
            label: "swimmers/7".to_string(),
        })
    );
}

#[test]
fn mermaid_viewer_renders_inline_unsupported_state_and_back_button_restores_aquarium() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = Some("unsupported terminal backend".to_string());

    app.render(&mut renderer, layout);

    let message_row = mermaid_content_rect(layout.overview_field).y;
    assert!(row_text(&renderer, message_row).contains("unsupported terminal backend"));

    let back_rect = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.back_rect.expect("back rect"),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: back_rect.x,
            row: back_rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn mermaid_keyboard_controls_pan_zoom_reset_and_escape() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    let content_rect = mermaid_content_rect(layout.overview_field);
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.content_rect = Some(content_rect);
    viewer.diagram_width = 1000.0;
    viewer.diagram_height = 800.0;
    viewer.center_x = 500.0;
    viewer.center_y = 400.0;
    viewer.unsupported_reason = None;

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
    ));
    let (zoom_after_plus, center_after_plus) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.zoom, (viewer.center_x, viewer.center_y)),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(zoom_after_plus, 1.5);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    ));
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    ));
    let (center_after_pan_x, center_after_pan_y) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(center_after_pan_x > center_after_plus.0);
    assert!(center_after_pan_y > center_after_plus.1);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
    ));
    let (zoom_after_reset, center_after_reset_x, center_after_reset_y) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.zoom, viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(zoom_after_reset, 1.0);
    assert_eq!(center_after_reset_x, 0.0);
    assert_eq!(center_after_reset_y, 0.0);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn mermaid_mouse_drag_and_scroll_update_viewport() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    let content_rect = mermaid_content_rect(layout.overview_field);
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.content_rect = Some(content_rect);
    viewer.diagram_width = 1000.0;
    viewer.diagram_height = 800.0;
    viewer.center_x = 500.0;
    viewer.center_y = 400.0;
    viewer.unsupported_reason = None;
    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);

    let (start_column, start_row) =
        find_blank_position(&renderer, content_rect).expect("empty Mermaid canvas cell");
    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: start_column,
            row: start_row,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(app.handle_mermaid_mouse_drag(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: start_column + 5,
            row: start_row + 2,
            modifiers: KeyModifiers::NONE,
        },
    ));
    let (center_after_drag_x, center_after_drag_y) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_ne!(center_after_drag_x, 500.0);
    assert_ne!(center_after_drag_y, 400.0);
    assert!(app.handle_mermaid_mouse_up());

    let zoom_before_scroll = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.zoom,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(app.handle_mermaid_scroll(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: start_column,
            row: start_row,
            modifiers: KeyModifiers::NONE,
        },
        MermaidZoomDirection::In,
    ));
    let zoom_after_scroll = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.zoom,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(zoom_after_scroll > zoom_before_scroll);
    assert_eq!(zoom_after_scroll, 1.25);
}

#[test]
fn mermaid_clicking_visible_owner_label_focuses_it() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    app.render(&mut renderer, layout);

    let beta = find_text_position(&renderer, "Beta Node").expect("Beta Node overlay");
    let center_before = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: beta.0,
            row: beta.1,
            modifiers: KeyModifiers::NONE,
        },
    ));

    let (focus_status, focused_source_index, center_after) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            viewer.focused_source_index,
            (viewer.center_x, viewer.center_y),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Beta Node"));
    assert!(focused_source_index.is_some());
    assert_ne!(center_after, center_before);
}

// ── render_thought_config_editor coverage ────────────────────────────────────

#[test]
fn render_thought_config_editor_enabled_field_focused() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig { enabled: true, ..ThoughtConfig::default() },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Enabled;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_thought_config_editor_model_field_focused_with_model() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            enabled: false,
            model: "claude-opus-4-6".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Model;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_thought_config_editor_model_field_focused_empty_model() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig { model: String::new(), ..ThoughtConfig::default() },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Model;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_thought_config_editor_save_and_cancel_focused() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig::default(),
        None,
    ));
    for focus in [
        ThoughtConfigEditorField::Save,
        ThoughtConfigEditorField::Cancel,
        ThoughtConfigEditorField::Test,
        ThoughtConfigEditorField::Backend,
    ] {
        if let Some(editor) = &mut app.thought_config_editor {
            editor.focus = focus;
        }
        app.render(&mut renderer, layout);
    }
}

// ── render_plan_text_content coverage ────────────────────────────────────────

fn open_mermaid_on_plan_tab(content: Option<&str>, active_tab: DomainPlanTab) -> (App<MockApi>, Renderer, WorkspaceLayout) {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph LR\nA-->B",
    );
    artifact.plan_files = Some(vec![
        "schema.mmd".to_string(),
        "plan.md".to_string(),
        "backend.md".to_string(),
    ]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);
    app.open_mermaid_viewer("sess-1".to_string());
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.active_tab = active_tab;
        viewer.plan_text_content = content.map(str::to_string);
    }
    let renderer = test_renderer(120, 32);
    (app, renderer, layout)
}

#[test]
fn render_plan_text_content_loading_state_when_no_content() {
    let (mut app, mut renderer, layout) = open_mermaid_on_plan_tab(None, DomainPlanTab::Plan);
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_heading_and_list_lines() {
    let content = "# Heading\n- list item\n  - nested\nbody text\n| table |\n|-|-|";
    let (mut app, mut renderer, layout) = open_mermaid_on_plan_tab(Some(content), DomainPlanTab::Plan);
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_scroll_indicator_when_content_exceeds_height() {
    // 50 lines of content will overflow the viewport height (~28 usable rows)
    let content = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, mut renderer, layout) = open_mermaid_on_plan_tab(Some(&content), DomainPlanTab::Plan);
    // Set scroll to trigger the non-zero pct branch
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.plan_text_scroll = 5;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_scroll_indicator_at_top_pct_100() {
    // Short enough that scroll is 0 but total_lines > visible → pct = 100 when max_scroll == 0
    // Actually max_scroll == 0 when total_lines <= visible_height, so we need more lines but scroll stays 0
    let content = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, mut renderer, layout) = open_mermaid_on_plan_tab(Some(&content), DomainPlanTab::Plan);
    // Leave scroll at 0; max_scroll > 0 so we get normal pct calculation
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_rewraps_on_second_render_same_width() {
    let content = "# Title\nbody";
    let (mut app, mut renderer, layout) = open_mermaid_on_plan_tab(Some(content), DomainPlanTab::Backend);
    // First render populates plan_text_lines
    app.render(&mut renderer, layout);
    // Second render should reuse cached lines (no re-wrap needed)
    app.render(&mut renderer, layout);
}

// ── switch_plan_tab tests ─────────────────────────────────────────────────────

fn open_mermaid_with_plan_tabs(api: MockApi) -> App<MockApi> {
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph LR\nA-->B",
    );
    artifact.plan_files = Some(vec![
        "schema.mmd".to_string(),
        "plan.md".to_string(),
        "backend.md".to_string(),
    ]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);
    app.open_mermaid_viewer("sess-1".to_string());
    app
}

#[test]
fn switch_plan_tab_noop_in_aquarium_mode() {
    let api = MockApi::new();
    let mut app = make_app(api);
    // Default mode is Aquarium; switch_plan_tab must not panic or change state
    app.switch_plan_tab(DomainPlanTab::Plan);
}

#[test]
fn switch_plan_tab_noop_when_no_plan_tabs() {
    let api = MockApi::new();
    let (mut app, _, _) = open_mermaid_test_viewer("graph LR\nA-->B", 120, 32);
    // viewer has no plan_tabs (open_mermaid_test_viewer doesn't set plan_files)
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else { panic!() };
    // active_tab unchanged
    assert_eq!(viewer.active_tab, DomainPlanTab::Schema);
}

#[test]
fn switch_plan_tab_noop_when_already_on_tab() {
    let api = MockApi::new();
    let mut app = open_mermaid_with_plan_tabs(api);
    // active_tab starts at Schema; switching to Schema again is a no-op
    app.switch_plan_tab(DomainPlanTab::Schema);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else { panic!() };
    assert_eq!(viewer.active_tab, DomainPlanTab::Schema);
}

#[test]
fn switch_plan_tab_to_schema_updates_viewer_without_fetch() {
    let api = MockApi::new();
    let mut app = open_mermaid_with_plan_tabs(api.clone());
    // Set active_tab to Plan first so switching to Schema is valid
    {
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else { panic!() };
        viewer.active_tab = DomainPlanTab::Plan;
        viewer.plan_text_content = Some("old content".to_string());
    }
    app.switch_plan_tab(DomainPlanTab::Schema);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else { panic!() };
    assert_eq!(viewer.active_tab, DomainPlanTab::Schema);
    assert!(viewer.plan_text_content.is_none());
    // No plan file fetch should have been issued
    assert_eq!(api.native_status_calls(), 0);
}

#[test]
fn switch_plan_tab_to_non_schema_fetches_plan_file_ok() {
    let api = MockApi::new();
    api.push_plan_file(Ok(PlanFileResponse {
        session_id: "sess-1".to_string(),
        name: "plan.md".to_string(),
        content: Some("# Plan\n- slice one".to_string()),
        error: None,
    }));
    let mut app = open_mermaid_with_plan_tabs(api);
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else { panic!() };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    assert_eq!(
        viewer.plan_text_content.as_deref(),
        Some("# Plan\n- slice one")
    );
    assert_eq!(viewer.plan_text_scroll, 0);
}

#[test]
fn switch_plan_tab_to_non_schema_shows_error_from_response() {
    let api = MockApi::new();
    api.push_plan_file(Ok(PlanFileResponse {
        session_id: "sess-1".to_string(),
        name: "plan.md".to_string(),
        content: None,
        error: Some("file not found".to_string()),
    }));
    let mut app = open_mermaid_with_plan_tabs(api);
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else { panic!() };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    assert!(
        app.message
            .as_ref()
            .map(|(m, _)| m.contains("plan file"))
            .unwrap_or(false)
    );
}

#[test]
fn switch_plan_tab_to_non_schema_shows_error_on_fetch_failure() {
    let api = MockApi::new();
    api.push_plan_file(Err("network error".to_string()));
    let mut app = open_mermaid_with_plan_tabs(api);
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else { panic!() };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    assert!(viewer.plan_text_content.is_none());
    assert!(
        app.message
            .as_ref()
            .map(|(m, _)| m.contains("plan file fetch failed"))
            .unwrap_or(false)
    );
}

#[test]
fn mermaid_clicking_sequence_diagram_does_not_create_focus() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);
    app.render(&mut renderer, layout);
    let content_rect = mermaid_content_rect(layout.overview_field);
    let (column, row) = find_blank_position(&renderer, content_rect).expect("blank sequence cell");

    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
    ));

    let (focused_source_index, focus_status, render_error) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focused_source_index,
            viewer.focus_status.clone(),
            viewer.render_error.clone(),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_source_index, None);
    assert_eq!(focus_status, None);
    assert_eq!(render_error, None);
}

#[test]
fn mermaid_render_reuses_prepared_source_state_across_zoom_and_pan() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = None;

    app.render(&mut renderer, layout);
    let (prepare_after_first, viewport_after_first, first_lines_empty) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.source_prepare_count,
            viewer.viewport_render_count,
            viewer.cached_lines.is_empty(),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_first, 1);
    assert_eq!(viewport_after_first, 1);
    assert!(!first_lines_empty);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);
    let (prepare_after_zoom, viewport_after_zoom) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => {
            (viewer.source_prepare_count, viewer.viewport_render_count)
        }
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_zoom, 1);
    assert_eq!(viewport_after_zoom, 2);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);
    let (prepare_after_pan, viewport_after_pan) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => {
            (viewer.source_prepare_count, viewer.viewport_render_count)
        }
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_pan, 1);
    assert_eq!(viewport_after_pan, 3);
}

#[test]
fn mermaid_refresh_invalidates_prepared_source_state_when_artifact_changes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    let mut renderer = test_renderer(120, 32);
    let sessions = vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)];
    app.merge_sessions(sessions.clone(), layout.overview_field);
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow-a.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = None;

    app.render(&mut renderer, layout);
    let prepare_after_first = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.source_prepare_count,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_first, 1);

    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow-b.mmd",
        "2026-03-23T10:06:00Z",
        "graph TD\nA-->C\n",
    )));
    app.refresh_mermaid_artifacts(&sessions);
    app.render(&mut renderer, layout);

    let (prepare_after_refresh, refreshed_path) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.source_prepare_count,
            viewer.path.as_deref().map(str::to_string),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_refresh, 2);
    assert_eq!(
        refreshed_path.as_deref(),
        Some("/tmp/repos/swimmers/flow-b.mmd")
    );
}

#[test]
fn mermaid_graph_node_labels_render_as_terminal_text() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    app.render(&mut renderer, layout);

    let alpha = find_text_position(&renderer, "Alpha Node").expect("Alpha Node overlay");
    let beta = find_text_position(&renderer, "Beta Node").expect("Beta Node overlay");
    assert_eq!(cell_at(&renderer, alpha.0, alpha.1).ch, 'A');
    assert_eq!(cell_at(&renderer, beta.0, beta.1).ch, 'B');
    assert!(row_text(&renderer, layout.overview_field.y).contains("outline"));
}

#[test]
fn mermaid_outline_background_stays_sparse_for_simple_flowchart() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    app.render(&mut renderer, layout);

    let background_chars = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => mermaid_background_charset(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        !background_chars.is_empty(),
        "outline should draw connectors"
    );
    assert!(
        background_chars.len() < 40,
        "outline background should stay sparse: {background_chars:?}"
    );
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '>' | '<')),
        "{background_chars:?}"
    );
}

#[test]
fn mermaid_merge_outline_segments_coalesces_overlapping_ranges() {
    let merged = mermaid_merge_outline_segments(&[
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Horizontal,
            fixed: 8,
            start: 10,
            end: 16,
        },
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Horizontal,
            fixed: 8,
            start: 14,
            end: 22,
        },
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Vertical,
            fixed: 30,
            start: 4,
            end: 7,
        },
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Vertical,
            fixed: 30,
            start: 8,
            end: 11,
        },
    ]);

    assert_eq!(
        merged,
        vec![
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Horizontal,
                fixed: 8,
                start: 10,
                end: 22,
            },
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Vertical,
                fixed: 30,
                start: 4,
                end: 11,
            },
        ]
    );
}

#[test]
fn mermaid_outline_background_coalesces_duplicate_edges() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 12,
    };
    let nodes = vec![
        MermaidOutlineNode {
            key: "node:left".to_string(),
            source_index: 0,
            x: 2,
            y: 2,
            text_width: 4,
        },
        MermaidOutlineNode {
            key: "node:right".to_string(),
            source_index: 1,
            x: 26,
            y: 8,
            text_width: 5,
        },
    ];
    let single = mermaid_render_outline_background(
        content_rect,
        &nodes,
        [MermaidOutlineEdge {
            from_key: "node:left".to_string(),
            to_key: "node:right".to_string(),
            directed: true,
        }],
    );
    let duplicated = mermaid_render_outline_background(
        content_rect,
        &nodes,
        [
            MermaidOutlineEdge {
                from_key: "node:left".to_string(),
                to_key: "node:right".to_string(),
                directed: true,
            },
            MermaidOutlineEdge {
                from_key: "node:left".to_string(),
                to_key: "node:right".to_string(),
                directed: true,
            },
        ],
    );

    assert_eq!(duplicated, single);
}

#[test]
fn mermaid_tab_focuses_first_visible_semantic_target_and_highlights_it() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    let (focus_status, focused_source_index, alpha_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            viewer.focused_source_index,
            find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node overlay"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Alpha Node"));
    assert!(focused_source_index.is_some());
    assert_eq!(
        cell_at(&renderer, alpha_position.0, alpha_position.1).fg,
        MERMAID_FOCUS_COLOR
    );
}

#[test]
fn mermaid_tab_cycles_forward_and_back_between_visible_targets() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);
    let (focus_status, beta_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node overlay"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Beta Node"));
    assert_eq!(
        cell_at(&renderer, beta_position.0, beta_position.1).fg,
        MERMAID_FOCUS_COLOR
    );

    press_mermaid_backtab(&mut app, layout);
    app.render(&mut renderer, layout);
    let focus_status = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.focus_status.clone(),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Alpha Node"));
}

#[test]
fn mermaid_er_entities_state_shows_only_entity_names_and_is_centered() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let (semantic_texts, bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(semantic_texts.contains(&"ORDER".to_string()));
    assert!(!semantic_texts.contains(&"email".to_string()));
    assert!(!semantic_texts.contains(&"user_id".to_string()));
    assert!(!semantic_texts.contains(&"uuid".to_string()));
    assert!(!semantic_texts.contains(&"places".to_string()));
    let center_x = (bounds.0 + bounds.1) / 2;
    let center_y = (bounds.2 + bounds.3) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    let expected_y = content_rect.y + content_rect.height / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 2);
    assert!((center_y as i32 - expected_y as i32).abs() <= 1);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
}

#[test]
fn mermaid_flowchart_overview_hides_edge_labels_until_zoomed() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    assert!(find_text_position(&renderer, "Group One").is_some());
    assert!(find_text_position(&renderer, "Producer").is_none());
    assert!(find_text_position(&renderer, "Consumer").is_none());
    assert!(find_text_position(&renderer, "ships data").is_none());
    assert!(row_text(&renderer, layout.overview_field.y).contains("outline"));
}

#[test]
fn mermaid_outline_collapses_subgraph_edges_to_top_level_groups() {
    let source = "graph LR\nsubgraph Left Side\nA[Alpha]\nB[Beta]\nend\nsubgraph Right Side\nC[Gamma]\nD[Delta]\nend\nA --> C\nB --> D\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 140, 36);

    app.render(&mut renderer, layout);

    let (semantic_texts, background_chars) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_background_charset(viewer),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(
        semantic_texts,
        vec!["Left Side".to_string(), "Right Side".to_string()]
    );
    assert!(find_text_position(&renderer, "Alpha").is_none());
    assert!(find_text_position(&renderer, "Beta").is_none());
    assert!(find_text_position(&renderer, "Gamma").is_none());
    assert!(find_text_position(&renderer, "Delta").is_none());
    assert!(
        background_chars
            .iter()
            .any(|ch| matches!(ch, '_' | '>' | '<')),
        "{background_chars:?}"
    );
}

#[test]
fn mermaid_flowchart_overview_compacts_long_node_labels() {
    let source = "graph TD\nA[1. Verified Identity And api cfo admin hierarchy role restricted]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let (semantic_texts, background_chars) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_background_charset(viewer),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.iter().any(|text| text.starts_with("1. Ver")),
        "{semantic_texts:?}"
    );
    assert!(!semantic_texts.iter().any(|text| text.contains("hierarchy")));
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '\\' | '>' | '<')),
        "{background_chars:?}"
    );
    assert!(row_text(&renderer, layout.overview_field.y).contains("outline"));
}

#[test]
fn mermaid_er_overview_shows_compact_entity_words_without_svg_text_noise() {
    let source = "erDiagram\ngoverned_revision_artifacts {\n  uuid id PK\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let (semantic_texts, background_chars) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_background_charset(viewer),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts
            .iter()
            .any(|text| text == "governed revision"),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts
            .iter()
            .any(|text| text.contains("governed_revision_artifacts")),
        "{semantic_texts:?}"
    );
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '\\' | '>' | '<')),
        "{background_chars:?}"
    );
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
}

#[test]
fn mermaid_detail_projection_suppresses_edge_labels_in_compact_views() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        find_text_position(&renderer, "ships data").is_none(),
        "status row: {}; semantic_texts: {:?}",
        row_text(&renderer, layout.overview_field.y),
        semantic_texts
    );
    assert!(find_text_position(&renderer, "Producer").is_some());
    assert!(find_text_position(&renderer, "Consumer").is_some());
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("detail L2"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("zoom 150%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_tab_focuses_visible_owner_labels_in_detail_l2() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    let (focus_status, producer_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            find_cached_semantic_line(viewer, "Producer").expect("Producer overlay"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L2"));
    assert_eq!(focus_status.as_deref(), Some("focus Producer"));
    assert_eq!(
        cell_at(&renderer, producer_position.0, producer_position.1).fg,
        MERMAID_FOCUS_COLOR
    );
    assert!(find_text_position(&renderer, "ships data").is_none());
}

#[test]
fn mermaid_escape_clears_focus_before_closing() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    let (focused_source_index, focus_status) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.focused_source_index, viewer.focus_status.clone()),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_source_index, None);
    assert_eq!(focus_status, None);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn mermaid_er_schema_uses_smart_colors_for_titles_types_and_connectors() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..3 {
        scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    }
    app.render(&mut renderer, layout);

    let (background_colors, user_owner_key, order_owner_key, owner_colors) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                mermaid_background_colors_set(viewer),
                mermaid_owner_key_for_text(viewer, "USER"),
                mermaid_owner_key_for_text(viewer, "ORDER"),
                mermaid_owner_accent_map(
                    &viewer
                        .prepared_render
                        .as_ref()
                        .expect("prepared render")
                        .semantic_lines,
                ),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

    let user_accent = mermaid_owner_accent_color(&user_owner_key, &owner_colors);
    let order_accent = mermaid_owner_accent_color(&order_owner_key, &owner_colors);
    assert_eq!(mermaid_text_color(&renderer, "USER"), user_accent);
    assert_eq!(mermaid_text_color(&renderer, "ORDER"), order_accent);
    assert_ne!(user_accent, order_accent);
    assert_eq!(mermaid_border_color(&renderer, "USER"), user_accent);
    assert_eq!(mermaid_border_color(&renderer, "ORDER"), order_accent);
    assert_eq!(mermaid_text_color(&renderer, "uuid"), MERMAID_TYPE_COLOR);
    assert_eq!(mermaid_text_color(&renderer, "email"), MERMAID_BODY_COLOR);
    assert_eq!(
        mermaid_text_color(&renderer, "user_id FK"),
        MERMAID_BODY_COLOR
    );
    assert!(background_colors.contains(&format!("{MERMAID_CONNECTOR_COLOR:?}")));
}

#[test]
fn mermaid_flowchart_detail_uses_smart_colors_for_titles_labels_and_connectors() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);

    let (producer_owner_key, consumer_owner_key, background_colors, owner_colors) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                mermaid_owner_key_for_text(viewer, "Producer"),
                mermaid_owner_key_for_text(viewer, "Consumer"),
                mermaid_background_colors_set(viewer),
                mermaid_owner_accent_map(
                    &viewer
                        .prepared_render
                        .as_ref()
                        .expect("prepared render")
                        .semantic_lines,
                ),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

    assert_eq!(
        mermaid_text_color(&renderer, "Producer"),
        mermaid_owner_accent_color(&producer_owner_key, &owner_colors)
    );
    assert_eq!(
        mermaid_text_color(&renderer, "Consumer"),
        mermaid_owner_accent_color(&consumer_owner_key, &owner_colors)
    );
    assert_eq!(
        mermaid_border_color(&renderer, "Producer"),
        mermaid_owner_accent_color(&producer_owner_key, &owner_colors)
    );
    assert_eq!(find_text_position(&renderer, "ships data"), None);
    assert!(!background_colors.is_empty());
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L2"));
}

#[test]
fn mermaid_sequence_diagram_connector_fallback_uses_dark_grey_cells() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    app.render(&mut renderer, layout);

    let background_colors = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => mermaid_background_colors(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(!background_colors.is_empty());
    assert!(
        background_colors
            .iter()
            .all(|color| *color == MERMAID_CONNECTOR_COLOR),
        "{background_colors:?}"
    );
}

#[test]
fn mermaid_error_and_unsupported_states_keep_existing_colors() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.unsupported_reason =
            Some("inline Mermaid rendering is unsupported for TERM=dumb".to_string());
    }
    app.render(&mut renderer, layout);
    let unsupported = find_text_position(
        &renderer,
        "inline Mermaid rendering is unsupported for TERM=dumb",
    )
    .expect("unsupported text");
    assert_eq!(
        cell_at(&renderer, unsupported.0, unsupported.1).fg,
        Color::DarkGrey
    );
    assert_eq!(
        cell_at(&renderer, layout.overview_field.x, layout.overview_field.y).fg,
        Color::Cyan
    );

    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.artifact_error = Some("failed to parse mermaid artifact: bad source".to_string());
    }
    app.render(&mut renderer, layout);
    let artifact_error = find_text_position(&renderer, "failed to parse mermaid artifact")
        .expect("artifact error text");
    assert_eq!(
        cell_at(&renderer, artifact_error.0, artifact_error.1).fg,
        Color::Red
    );
}

#[test]
fn mermaid_owner_accents_stay_stable_across_pan_and_zoom() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    let content_rect = mermaid_content_rect(layout.overview_field);

    app.render(&mut renderer, layout);
    let (user_before, order_before) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "USER")
                .map(|line| line.color)
                .expect("USER before"),
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "ORDER")
                .map(|line| line.color)
                .expect("ORDER before"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    app.zoom_mermaid_viewer(MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
    app.pan_mermaid_viewer(18.0, 12.0);
    app.render(&mut renderer, layout);

    let (user_after, order_after, prepare_count) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "USER")
                .map(|line| line.color)
                .expect("USER after"),
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "ORDER")
                .map(|line| line.color)
                .expect("ORDER after"),
            viewer.source_prepare_count,
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    assert_eq!(user_after, user_before);
    assert_eq!(order_after, order_before);
    assert_eq!(prepare_count, 1);
}

#[test]
fn mermaid_er_scroll_enters_keys_then_columns_then_schema_states() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(
        semantic_texts.contains(&"id PK".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"email".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"uuid".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"string".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER keys"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.contains(&"email".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        semantic_texts.contains(&"id PK".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"uuid".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER columns"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.iter().any(|text| text.contains("uuid")),
        "{semantic_texts:?}"
    );
    assert!(
        semantic_texts.iter().any(|text| text.contains("string")),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER schema"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_er_reset_fit_returns_to_entities_state() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..3 {
        scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    }
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER schema"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(!semantic_texts.contains(&"id PK".to_string()));
    assert!(!semantic_texts.contains(&"email".to_string()));
    assert!(!semantic_texts.contains(&"uuid".to_string()));
    assert!(!semantic_texts.contains(&"string".to_string()));
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("fit 100%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_er_dense_schema_fit_is_centered_and_uses_the_viewport() {
    let source = r#"erDiagram
applications {
  uuid id PK
}
conversation_anchor_types {
  uuid id PK
  uuid application_id FK
  string anchor_type
}
conversation_anchors {
  uuid id PK
  uuid application_id FK
  uuid anchor_type_id FK
  string anchor_key
}
conversations {
  uuid id PK
  uuid application_id FK
  uuid anchor_id FK
  string conversation_type
}
conversation_policy_bindings {
  uuid id PK
  uuid conversation_id FK
  string policy_template_key
}
conversation_named_participants {
  uuid id PK
  uuid conversation_id FK
  string actor_type
}
conversation_effective_participants {
  uuid id PK
  uuid conversation_id FK
  boolean can_read
}
conversation_messages {
  uuid id PK
  uuid conversation_id FK
  string kind
}
conversation_events {
  uuid id PK
  uuid conversation_id FK
  uuid message_id FK
}
conversation_reads {
  uuid id PK
  uuid conversation_id FK
  uuid last_event_id FK
}
applications ||--o{ conversation_anchor_types : owns
applications ||--o{ conversation_anchors : scopes
applications ||--o{ conversations : scopes
conversation_anchor_types ||--o{ conversation_anchors : categorizes
conversation_anchors ||--o{ conversations : roots
conversations ||--o{ conversation_policy_bindings : uses
conversations ||--o{ conversation_named_participants : includes
conversations ||--o{ conversation_effective_participants : materializes
conversations ||--o{ conversation_messages : stores
conversations ||--o{ conversation_events : records
conversations ||--o{ conversation_reads : tracks
"#;
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 160, 48);

    app.render(&mut renderer, layout);

    let (semantic_texts, bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.len() >= 6, "{semantic_texts:?}");
    assert!(
        !semantic_texts.iter().any(|text| text.contains(" PK")),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.iter().any(|text| text.contains(" FK")),
        "{semantic_texts:?}"
    );
    let center_x = (bounds.0 + bounds.1) / 2;
    let center_y = (bounds.2 + bounds.3) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    let expected_y = content_rect.y + content_rect.height / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 3);
    assert!((center_y as i32 - expected_y as i32).abs() <= 2);
    let width_occupancy = f32::from(bounds.1.saturating_sub(bounds.0).saturating_add(1))
        / f32::from(content_rect.width);
    let height_occupancy = f32::from(bounds.3.saturating_sub(bounds.2).saturating_add(1))
        / f32::from(content_rect.height);
    assert!(width_occupancy >= 0.40, "{width_occupancy}");
    assert!(height_occupancy >= 0.30, "{height_occupancy}");
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
}

#[test]
fn mermaid_er_scroll_states_are_discrete_and_reversible() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER keys"));
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER columns"));
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::Out);
    app.render(&mut renderer, layout);
    let status = row_text(&renderer, layout.overview_field.y);
    assert!(status.contains("ER keys"), "{status}");
    assert!(!status.contains("detail L"), "{status}");
}

#[test]
fn mermaid_er_zoom_resets_pan_and_recenters_packed_layout() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.center_x = 500.0;
        viewer.center_y = 400.0;
        viewer.invalidate_viewport_cache();
    }
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let (center_x, center_y, bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.center_x,
            viewer.center_y,
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_ne!(center_x, 500.0);
    assert_ne!(center_y, 400.0);
    let center_x = (bounds.0 + bounds.1) / 2;
    let center_y = (bounds.2 + bounds.3) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    let expected_y = content_rect.y + content_rect.height / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 2);
    assert!((center_y as i32 - expected_y as i32).abs() <= 1);
}

#[test]
fn mermaid_er_order_clusters_connected_nodes_before_isolated_scanline_nodes() {
    let order = mermaid_order_er_nodes(&[
        er_order_node("node:a_leaf", 0.0, 0.0, &["node:a_hub"]),
        er_order_node("node:b_isolated", 10.0, 0.0, &[]),
        er_order_node("node:a_hub", 0.0, 10.0, &["node:a_leaf", "node:a_tail"]),
        er_order_node("node:a_tail", 0.0, 20.0, &["node:a_hub"]),
    ]);

    assert_eq!(
        order,
        vec![
            "node:a_hub".to_string(),
            "node:a_leaf".to_string(),
            "node:a_tail".to_string(),
            "node:b_isolated".to_string(),
        ]
    );
}

#[test]
fn mermaid_er_order_keeps_components_contiguous_when_xy_positions_interleave() {
    let order = mermaid_order_er_nodes(&[
        er_order_node("node:north_a", 0.0, 0.0, &["node:north_b"]),
        er_order_node("node:south_a", 20.0, 0.0, &["node:south_b"]),
        er_order_node("node:north_b", 0.0, 10.0, &["node:north_a"]),
        er_order_node("node:south_b", 20.0, 10.0, &["node:south_a"]),
    ]);

    assert_eq!(
        order,
        vec![
            "node:north_a".to_string(),
            "node:north_b".to_string(),
            "node:south_a".to_string(),
            "node:south_b".to_string(),
        ]
    );
}

#[test]
fn mermaid_too_small_view_keeps_existing_guard() {
    let (mut app, mut renderer, _layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    let small_field = Rect {
        x: 0,
        y: 0,
        width: 15,
        height: 7,
    };
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    render_mermaid_viewer(&mut renderer, small_field, viewer);

    assert!(find_text_position(&renderer, "Mermaid view").is_some());
    assert!(find_text_position(&renderer, "too small").is_some());
    let semantic_count = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.cached_semantic_lines.len(),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(semantic_count, 0);
}

#[test]
fn mermaid_semantic_labels_track_zoom_and_pan() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    let content_rect = mermaid_content_rect(layout.overview_field);

    app.render(&mut renderer, layout);
    let (alpha_before, beta_before) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node before"),
            find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node before"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    app.zoom_mermaid_viewer(MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
    app.pan_mermaid_viewer(24.0, 18.0);
    app.render(&mut renderer, layout);

    let (alpha_after, beta_after, prepare_count) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node after"),
            find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node after"),
            viewer.source_prepare_count,
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_ne!(alpha_after, alpha_before);
    assert_ne!(beta_after, beta_before);
    assert_eq!(prepare_count, 1);
}

#[test]
fn mermaid_zoom_status_clamps_to_fit_and_uses_round_percentages() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Producer] --> B[Consumer]\n", 120, 32);

    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("fit 100%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    press_mermaid_key(&mut app, layout, '-');
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("fit 100%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("zoom 150%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
    assert!(
        !row_text(&renderer, layout.overview_field.y).contains("179%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_sequence_diagram_falls_back_to_connector_only_background() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    app.render(&mut renderer, layout);

    let (render_error, cached_lines_empty, cached_semantic_lines_empty, background_chars) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                viewer.render_error.clone(),
                viewer.cached_lines.is_empty(),
                viewer.cached_semantic_lines.is_empty(),
                mermaid_background_charset(viewer),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
    assert_eq!(render_error, None);
    assert!(!cached_lines_empty);
    assert!(cached_semantic_lines_empty);
    assert!(find_text_position(&renderer, "hello").is_none());
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '\\' | '>' | '<')),
        "{background_chars:?}"
    );
}

#[test]
fn mermaid_tab_reports_no_semantic_targets_for_sequence_diagrams() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    let (focused_source_index, focus_status) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.focused_source_index, viewer.focus_status.clone()),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_source_index, None);
    assert_eq!(focus_status.as_deref(), Some("no semantic targets"));
    assert!(row_text(&renderer, layout.overview_field.y).contains("no semantic targets"));
}

#[test]
fn mermaid_semantic_labels_clip_to_viewport_bounds() {
    let content_rect = Rect {
        x: 42,
        y: 10,
        width: 20,
        height: 5,
    };
    let projected = project_mermaid_semantic_lines(
        &[MermaidSemanticLine {
            text: "Alpha Node".to_string(),
            diagram_x: 0.0,
            diagram_y: 4.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::NodeSummary,
            owner_key: "node:A".to_string(),
            outline_eligible: true,
            owner_width: 20.0,
            owner_height: 8.0,
        }],
        MermaidViewportTransform {
            scale: 1.0,
            tx: -4.0,
            ty: 0.0,
        },
        content_rect,
        MermaidViewState::L1,
    );

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].x, content_rect.x);
    assert_eq!(projected[0].y, content_rect.y + 1);
    assert_eq!(projected[0].text, "Alpha Node");
}

#[test]
fn mermaid_compacts_multiline_node_text_to_consecutive_rows() {
    let content_rect = Rect {
        x: 10,
        y: 10,
        width: 30,
        height: 8,
    };
    let projected = project_mermaid_semantic_lines(
        &[
            MermaidSemanticLine {
                text: "first line".to_string(),
                diagram_x: 0.0,
                diagram_y: 4.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 20.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "second line".to_string(),
                diagram_x: 0.0,
                diagram_y: 12.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 20.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "third line".to_string(),
                diagram_x: 0.0,
                diagram_y: 20.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 20.0,
                owner_height: 20.0,
            },
        ],
        MermaidViewportTransform {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        content_rect,
        MermaidViewState::L2,
    );

    assert_eq!(projected.len(), 3);
    assert_eq!(
        projected.iter().map(|line| line.y).collect::<Vec<_>>(),
        vec![content_rect.y + 1, content_rect.y + 2, content_rect.y + 3]
    );
}

#[test]
fn mermaid_detail_projection_hides_owner_summary_when_detail_lines_exist() {
    let content_rect = Rect {
        x: 10,
        y: 10,
        width: 40,
        height: 10,
    };
    let projected = project_mermaid_semantic_lines(
        &[
            MermaidSemanticLine {
                text: "Alpha compact".to_string(),
                diagram_x: 0.0,
                diagram_y: 12.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeSummary,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 24.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "Alpha Full".to_string(),
                diagram_x: 0.0,
                diagram_y: 4.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 24.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "Second Line".to_string(),
                diagram_x: 0.0,
                diagram_y: 8.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 24.0,
                owner_height: 20.0,
            },
        ],
        MermaidViewportTransform {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        content_rect,
        MermaidViewState::L2,
    );

    assert_eq!(
        projected
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>(),
        vec!["Alpha Full".to_string(), "Second Line".to_string()]
    );
}

#[test]
fn mermaid_detail_box_rects_wrap_visible_lines_tightly() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 20,
    };
    let source_lines = vec![
        MermaidSemanticLine {
            text: "USER".to_string(),
            diagram_x: 0.0,
            diagram_y: 0.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::NodeTitle,
            owner_key: "node:USER".to_string(),
            outline_eligible: false,
            owner_width: 20.0,
            owner_height: 20.0,
        },
        MermaidSemanticLine {
            text: "id".to_string(),
            diagram_x: 0.0,
            diagram_y: 0.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::ErAttributeName,
            owner_key: "node:USER".to_string(),
            outline_eligible: false,
            owner_width: 20.0,
            owner_height: 20.0,
        },
        MermaidSemanticLine {
            text: "email".to_string(),
            diagram_x: 0.0,
            diagram_y: 0.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::ErAttributeName,
            owner_key: "node:USER".to_string(),
            outline_eligible: false,
            owner_width: 20.0,
            owner_height: 20.0,
        },
    ];
    let projected = vec![
        MermaidProjectedLine {
            source_index: 0,
            x: 20,
            y: 11,
            text: "USER".to_string(),
            color: MERMAID_BODY_COLOR,
        },
        MermaidProjectedLine {
            source_index: 1,
            x: 18,
            y: 12,
            text: "id".to_string(),
            color: MERMAID_BODY_COLOR,
        },
        MermaidProjectedLine {
            source_index: 2,
            x: 18,
            y: 13,
            text: "email".to_string(),
            color: MERMAID_BODY_COLOR,
        },
    ];

    let rects = mermaid_detail_box_rects(&source_lines, &projected, content_rect);
    assert_eq!(
        rects.get("node:USER").copied(),
        Some(MermaidOutlineLabelRect {
            left: 17,
            right: 24,
            top: 10,
            bottom: 14,
        })
    );
}

#[test]
fn mermaid_packed_detail_rects_center_cluster_within_viewport() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 60,
        height: 20,
    };
    let owners = vec![
        MermaidPackedDetailOwner {
            owner_key: "node:a".to_string(),
            sort_x: 48,
            sort_y: 1,
            lines: vec![MermaidPackedDetailLine {
                source_index: 0,
                text: "SSH as sandbox user".to_string(),
                color: MERMAID_BODY_COLOR,
                kind: MermaidSemanticKind::NodeTitle,
            }],
        },
        MermaidPackedDetailOwner {
            owner_key: "node:b".to_string(),
            sort_x: 50,
            sort_y: 5,
            lines: vec![
                MermaidPackedDetailLine {
                    source_index: 1,
                    text: "skillbox-login.sh".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 2,
                    text: "ForceCommand".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
            ],
        },
        MermaidPackedDetailOwner {
            owner_key: "node:c".to_string(),
            sort_x: 48,
            sort_y: 10,
            lines: vec![
                MermaidPackedDetailLine {
                    source_index: 3,
                    text: "tailscale whois".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 4,
                    text: "identity resolution".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
            ],
        },
        MermaidPackedDetailOwner {
            owner_key: "node:d".to_string(),
            sort_x: 50,
            sort_y: 15,
            lines: vec![
                MermaidPackedDetailLine {
                    source_index: 5,
                    text: "SKILLBOX_DEV".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 6,
                    text: "GIT_AUTHOR_NAME".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 7,
                    text: "GIT_AUTHOR_EMAIL".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
            ],
        },
    ];

    let rects = mermaid_pack_detail_box_rects(content_rect, &owners);
    assert_eq!(rects.len(), owners.len());

    let left = rects.values().map(|rect| rect.left).min().expect("left");
    let right = rects.values().map(|rect| rect.right).max().expect("right");
    let top = rects.values().map(|rect| rect.top).min().expect("top");
    let bottom = rects
        .values()
        .map(|rect| rect.bottom)
        .max()
        .expect("bottom");
    let center_x = (left + right) / 2;
    let center_y = (top + bottom) / 2;
    let expected_x = i32::from(content_rect.x + content_rect.width / 2);
    let expected_y = i32::from(content_rect.y + content_rect.height / 2);
    assert!((center_x - expected_x).abs() <= 2);
    assert!((center_y - expected_y).abs() <= 2);
    assert!(right - left >= i32::from(content_rect.width / 3));
}

#[test]
fn mermaid_er_detail_view_draws_compact_box_around_visible_lines() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..2 {
        press_mermaid_key(&mut app, layout, '+');
    }
    app.render(&mut renderer, layout);

    let user = find_text_position(&renderer, "USER").expect("USER label");
    let id = find_text_position(&renderer, "id PK").expect("id label");
    let email = find_text_position(&renderer, "email").expect("email label");

    assert_eq!(id.1, user.1 + 1);
    assert_eq!(email.1, id.1 + 1);

    let left = user.0.min(id.0).min(email.0).saturating_sub(1);
    let right = (user.0 + display_width("USER") - 1)
        .max(id.0 + display_width("id PK") - 1)
        .max(email.0 + display_width("email") - 1)
        .saturating_add(1);

    assert_eq!(cell_at(&renderer, left, user.1).ch, '|');
    assert_eq!(cell_at(&renderer, left, id.1).ch, '|');
    assert_eq!(cell_at(&renderer, left, email.1).ch, '|');
    assert_eq!(cell_at(&renderer, right, user.1).ch, '|');
    assert_eq!(cell_at(&renderer, right, id.1).ch, '|');
    assert_eq!(cell_at(&renderer, right, email.1).ch, '|');
    assert_eq!(
        cell_at(&renderer, left + 1, user.1.saturating_sub(1)).ch,
        '_'
    );
    assert_eq!(
        cell_at(&renderer, left + 1, email.1.saturating_add(1)).ch,
        '_'
    );
}

#[test]
fn mermaid_flowchart_detail_l2_packs_boxes_to_use_viewport() {
    let source = "graph TD\nA[SSH as sandbox user] -->|triggers| B[skillbox-login.sh]\nB -->|runs| C[tailscale whois]\nC -->|sets| D[SKILLBOX_DEV]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);

    let (bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    let center_x = (bounds.0 + bounds.1) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 2);
}

#[test]
fn mermaid_er_semantic_columns_cap_type_to_name_gap_at_three_spaces() {
    let node = mermaid_rs_renderer::NodeLayout {
        id: "ITEM".to_string(),
        x: 10.0,
        y: 10.0,
        width: 140.0,
        height: 80.0,
        label: mermaid_rs_renderer::layout::TextBlock {
            lines: vec![
                "ITEM".to_string(),
                "---".to_string(),
                "uuid id PK".to_string(),
                "decimal total".to_string(),
                "bool open".to_string(),
            ],
            width: 0.0,
            height: 0.0,
        },
        shape: mermaid_rs_renderer::ir::NodeShape::Rectangle,
        style: mermaid_rs_renderer::ir::NodeStyle::default(),
        link: None,
        anchor_subgraph: None,
        hidden: false,
        icon: None,
    };
    let mut semantic_lines = Vec::new();
    extend_mermaid_er_semantic_lines(
        &mut semantic_lines,
        &node,
        10.0,
        14.0,
        10.0,
        "node:ITEM",
        true,
    );

    let projected = project_mermaid_semantic_lines(
        &semantic_lines,
        MermaidViewportTransform {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 32,
        },
        MermaidViewState::L3,
    );

    let x_for = |needle: &str| -> u16 {
        projected
            .iter()
            .find(|line| line.text == needle)
            .map(|line| line.x)
            .unwrap_or_else(|| panic!("{needle}"))
    };

    let uuid = x_for("uuid");
    let id = x_for("id PK");
    let decimal = x_for("decimal");
    let total = x_for("total");
    let bool_pos = x_for("bool");
    let open = x_for("open");

    assert_eq!(id, uuid + display_width("uuid") + 3);
    assert_eq!(total, decimal + display_width("decimal") + 3);
    assert_eq!(open, bool_pos + display_width("bool") + 3);
}

#[test]
fn mermaid_resize_reprojects_semantic_labels() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);
    let group_before = find_text_position(&renderer, "Group One").expect("Group One before");

    let resized_layout = test_layout(160, 48);
    let mut resized_renderer = test_renderer(160, 48);
    app.render(&mut resized_renderer, resized_layout);

    let group_after = find_text_position(&resized_renderer, "Group One").expect("Group One after");
    assert_ne!(group_after, group_before);
    assert!(find_text_position(&resized_renderer, "Producer").is_none());
}

#[test]
fn mermaid_resize_preserves_focused_semantic_target() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);
    let (focused_before, focus_status_before) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.focused_source_index, viewer.focus_status.clone()),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    let resized_layout = test_layout(160, 48);
    let mut resized_renderer = test_renderer(160, 48);
    app.render(&mut resized_renderer, resized_layout);

    let (focused_after, focus_status, highlighted_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focused_source_index,
            viewer.focus_status.clone(),
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| Some(line.source_index) == viewer.focused_source_index)
                .map(|line| (line.x, line.y))
                .expect("focused semantic line after resize"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_after, focused_before);
    assert_eq!(focus_status, focus_status_before);
    assert_eq!(
        cell_at(
            &resized_renderer,
            highlighted_position.0,
            highlighted_position.1
        )
        .fg,
        MERMAID_FOCUS_COLOR
    );
}

#[test]
fn mermaid_pan_and_zoom_preserve_focused_target() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    let focused_before = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.focused_source_index,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    press_mermaid_key(&mut app, layout, '+');
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);

    let focused_after = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.focused_source_index,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_after, focused_before);
    assert!(row_text(&renderer, layout.overview_field.y).contains("zoom 150%"));
    assert!(row_text(&renderer, layout.overview_field.y).contains("focus Alpha Node"));
}

#[test]
fn mermaid_open_shortcut_uses_artifact_path_and_stays_in_viewer() {
    let api = MockApi::new();
    let opener = Arc::new(MockArtifactOpener::default());
    let layout = test_layout(120, 32);
    let mut app = make_app_with_artifact_opener(api, opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));

    assert_eq!(
        opener.calls(),
        vec!["/tmp/repos/swimmers/flow.mmd".to_string()]
    );
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Mermaid(_)));
    assert_eq!(
        app.visible_message(),
        Some("open Mermaid artifact -> flow.mmd")
    );
}

#[test]
fn mermaid_open_shortcut_reports_failures_and_missing_paths() {
    let api = MockApi::new();
    let opener = Arc::new(MockArtifactOpener::default());
    opener.fail_with("boom");
    let layout = test_layout(120, 32);
    let mut app = make_app_with_artifact_opener(api, opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));
    assert_eq!(
        app.visible_message(),
        Some("failed to open Mermaid artifact: boom")
    );
    assert_eq!(
        opener.calls(),
        vec!["/tmp/repos/swimmers/flow.mmd".to_string()]
    );

    let opener = Arc::new(MockArtifactOpener::default());
    let mut app = make_app_with_artifact_opener(MockApi::new(), opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );
    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.path = None;

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));
    assert_eq!(opener.calls(), Vec::<String>::new());
    assert_eq!(
        app.visible_message(),
        Some("Mermaid artifact path unavailable")
    );
}

proptest::proptest! {
    #[test]
    fn mermaid_mr_fit_is_canonical_after_zoom_and_pan_sequences(
        source in mermaid_flowchart_source_strategy(),
        width in 100u16..160,
        height in 24u16..52,
        ops in mermaid_metamorphic_ops_strategy(),
    ) {
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(&source, width, height);
        let baseline = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        apply_mermaid_metamorphic_ops(&mut app, layout, &ops);
        app.reset_mermaid_viewer_fit();
        let after_fit = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        proptest::prop_assert_eq!(after_fit, baseline);
    }

    #[test]
    fn mermaid_mr_pan_round_trip_restores_viewport(
        source in mermaid_flowchart_source_strategy(),
        width in 110u16..180,
        height in 28u16..56,
        x_ratio_percent in -90i16..=90,
        y_ratio_percent in -90i16..=90,
    ) {
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(&source, width, height);
        let content_rect = mermaid_content_rect_for_layout(layout);

        app.zoom_mermaid_viewer(MERMAID_KEYBOARD_ZOOM_STEP_PERCENT, None, content_rect);
        let baseline = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        let (dx, dy) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                let (left, right, up, down) = mermaid_pan_headroom(viewer, content_rect);
                (
                    mermaid_safe_pan_distance(x_ratio_percent, left, right),
                    mermaid_safe_pan_distance(y_ratio_percent, up, down),
                )
            }
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        proptest::prop_assume!(dx.abs() > 0.5 || dy.abs() > 0.5);

        app.pan_mermaid_viewer(dx, dy);
        let after_pan = render_mermaid_snapshot(&mut app, &mut renderer, layout);
        proptest::prop_assume!(after_pan != baseline);

        app.pan_mermaid_viewer(-dx, -dy);
        let round_trip = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        proptest::prop_assert_eq!(round_trip, baseline);
    }

    #[test]
    fn mermaid_mr_pointer_zoom_keeps_anchor_stable(
        source in mermaid_anchorable_source_strategy(),
        width in 120u16..180,
        height in 28u16..56,
        anchor_pick in 0usize..8,
    ) {
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(&source, width, height);
        app.render(&mut renderer, layout);

        let (source_index, anchor_x, anchor_y) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                let content_rect = viewer.content_rect.expect("content rect");
                let eligible = viewer
                    .cached_semantic_lines
                    .iter()
                    .filter(|line| {
                        line.x > content_rect.x.saturating_add(1)
                            && line.y > content_rect.y
                            && line.x.saturating_add(display_width(&line.text))
                                < content_rect.right().saturating_sub(1)
                            && line.y < content_rect.bottom().saturating_sub(1)
                    })
                    .collect::<Vec<_>>();
                proptest::prop_assume!(!eligible.is_empty());
                let anchor = eligible[anchor_pick % eligible.len()];
                (
                    anchor.source_index,
                    anchor.x.saturating_add(display_width(&anchor.text) / 2),
                    anchor.y,
                )
            }
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

        let content_rect = mermaid_content_rect_for_layout(layout);
        app.zoom_mermaid_viewer(
            MERMAID_SCROLL_ZOOM_STEP_PERCENT,
            Some((anchor_x, anchor_y)),
            content_rect,
        );
        app.render(&mut renderer, layout);

        let anchored_line = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.source_index == source_index),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        proptest::prop_assume!(anchored_line.is_some());
        let anchored_line = anchored_line.expect("anchored line");
        let anchored_center_x = anchored_line
            .x
            .saturating_add(display_width(&anchored_line.text) / 2);

        proptest::prop_assert!(
            (anchored_center_x as i32 - anchor_x as i32).abs() <= 2,
            "expected x anchor to stay stable: before={anchor_x}, after={}",
            anchored_center_x
        );
        proptest::prop_assert!(
            (anchored_line.y as i32 - anchor_y as i32).abs() <= 1,
            "expected y anchor to stay stable: before={anchor_y}, after={}",
            anchored_line.y
        );
    }
}

#[test]
fn handle_tui_event_covers_key_paste_mouse_and_resize_paths() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
    app.open_initial_request("/tmp/project".to_string());

    assert!(handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Paste("hello".to_string()),
    )
    .expect("paste event should succeed"));
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("hello")
    );

    app.close_initial_request();
    assert!(!handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
    )
    .expect("quit key should succeed"));

    assert!(handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }),
    )
    .expect("mouse up should succeed"));

    assert!(
        handle_tui_event(&mut app, &mut renderer, layout, Event::Resize(90, 20),)
            .expect("resize should succeed")
    );
    assert_eq!(renderer.width(), 90);
    assert_eq!(renderer.height(), 20);
}
