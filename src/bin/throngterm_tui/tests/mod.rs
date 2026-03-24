use super::*;
use std::cell::Cell as TestCell;
use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use tempfile::tempdir;
use throngterm::types::{ThoughtSource, ThoughtState, TransportHealth};

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

#[derive(Default)]
struct MockApiState {
    fetch_sessions_results: VecDeque<Result<Vec<SessionSummary>, String>>,
    mermaid_artifact_results: VecDeque<Result<MermaidArtifactResponse, String>>,
    native_status_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    publish_selection_results: VecDeque<Result<(), String>>,
    open_session_results: VecDeque<Result<NativeDesktopOpenResponse, String>>,
    list_dirs_results: VecDeque<Result<DirListResponse, String>>,
    create_session_results: VecDeque<Result<CreateSessionResponse, String>>,
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
                    })
                })
        })
    }

    fn fetch_native_status(
        &self,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .native_status_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(NativeDesktopStatusResponse {
                        supported: true,
                        platform: Some("test".to_string()),
                        app: Some("test".to_string()),
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
const TEST_REPO_THRONGTERM: &str = "/tmp/repos/throngterm";

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

fn make_app(api: MockApi) -> App<MockApi> {
    App::new(test_runtime(), api)
}

fn make_app_with_artifact_opener(
    api: MockApi,
    artifact_opener: Arc<dyn ArtifactOpener>,
) -> App<MockApi> {
    App::with_artifact_opener(test_runtime(), api, artifact_opener)
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
    assert!(error.contains("backend unavailable at"));
    assert!(error.contains("Start `throngterm` or set THRONGTERM_TUI_URL."));
    assert!(!error.contains("error sending request for url"));
}

async fn spawn_delayed_api_server(
    sessions_delay: Option<Duration>,
    native_open_delay: Option<Duration>,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::routing::{get, post};
    use axum::{Json, Router};

    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move || async move {
                if let Some(delay) = sessions_delay {
                    tokio::time::sleep(delay).await;
                }
                Json(SessionListResponse {
                    sessions: vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
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
    let (base_url, handle) =
        spawn_delayed_api_server(None, Some(Duration::from_millis(150))).await;
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
async fn api_client_fetch_sessions_keeps_short_timeout_for_refresh() {
    let (base_url, handle) =
        spawn_delayed_api_server(Some(Duration::from_millis(150)), None).await;
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
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary(
        "sess-7",
        "7",
        TEST_REPO_THRONGTERM,
    )]));
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
        session_summary("sess-7", "7", TEST_REPO_THRONGTERM),
        session_summary("sess-8", "8", TEST_REPO_OPENSOURCE),
    ]));
    let mut app = make_app(api);

    app.manual_refresh(layout);

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("refreshed 2 sessions")
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

fn open_mermaid_test_viewer(
    source: &str,
    width: u16,
    height: u16,
) -> (App<MockApi>, Renderer, WorkspaceLayout) {
    let api = MockApi::new();
    let layout = test_layout(width, height);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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

#[test]
fn mermaid_compact_overview_text_prefers_numeric_prefix_and_keywords() {
    let compact = mermaid_compact_overview_text([
        "1. Verified Identity And",
        "/api/cfo/admin/* calls are not outside the hierarchy",
    ])
    .expect("compact overview text");

    assert_eq!(compact, "1. Verified Identity");
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
    MermaidArtifactResponse {
        session_id: session_id.to_string(),
        available: true,
        path: Some(path.to_string()),
        updated_at: Some(timestamp(updated_at)),
        source: Some(source.to_string()),
        error: None,
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
    let throngterm_dir = path.join(".throngterm");
    fs::create_dir_all(&throngterm_dir).expect("create .throngterm");
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
    fs::write(throngterm_dir.join("colors.json"), contents).expect("write colors.json");
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
        "widening the clawgs rail should shrink the throngterm field"
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
            TEST_REPO_THRONGTERM,
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
        .any(|chip| chip.label == "1xthrongterm"));
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
        vec!["9: indexing docs"]
    );
}

#[test]
fn render_header_filter_strip_shows_repo_chips_and_thought_rows() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let throngterm_theme_id = "/tmp/throngterm".to_string();
    let skills_theme_id = "/tmp/skills".to_string();
    let throngterm_color = Color::Rgb {
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
        .insert(throngterm_theme_id.clone(), repo_theme("#B89875"));
    app.repo_themes
        .insert(skills_theme_id.clone(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_THRONGTERM,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some(throngterm_theme_id.clone());

    let mut second = session_summary_with_thought(
        "sess-2",
        "2",
        TEST_REPO_THRONGTERM,
        "wiring filter state",
        "2026-03-08T14:00:06Z",
    );
    second.repo_theme_id = Some(throngterm_theme_id);

    let mut third = session_summary_with_thought(
        "sess-3",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    third.repo_theme_id = Some(skills_theme_id);

    app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "7: patching tui",
            "2: wiring filter state",
            "9: indexing docs",
        ]
    );

    let header = build_header_filter_layout(&app, 120);
    let throngterm_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "2xthrongterm")
        .expect("throngterm chip should exist");
    let skills_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist");
    assert_eq!(throngterm_chip.color, throngterm_color);
    assert_eq!(skills_chip.color, skills_color);

    let mut renderer = test_renderer(120, 32);
    render_header_filter_strip(&app, &mut renderer, 120);

    assert_eq!(
        cell_at(&renderer, throngterm_chip.rect.x, throngterm_chip.rect.y).fg,
        throngterm_color
    );
    assert_eq!(
        cell_at(&renderer, skills_chip.rect.x, skills_chip.rect.y).fg,
        skills_color
    );
    assert!(row_text(&renderer, 2).ends_with("1xskills  2xthrongterm"));
}

#[test]
fn active_repo_header_chip_maps_to_code_open_action() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.repo_themes
        .insert("/tmp/throngterm".to_string(), repo_theme("#B89875"));
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_THRONGTERM,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        test_layout(120, 32).thought_entry_capacity(),
    );
    app.set_thought_filter_cwd(TEST_REPO_THRONGTERM.to_string());

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
            TEST_REPO_THRONGTERM.to_string()
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
        .insert("/tmp/throngterm".to_string(), repo_theme("#B89875"));
    app.repo_themes
        .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_THRONGTERM,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some("/tmp/throngterm".to_string());

    let mut second = session_summary_with_thought(
        "sess-2",
        "2",
        TEST_REPO_THRONGTERM,
        "wiring filter state",
        "2026-03-08T14:00:06Z",
    );
    second.repo_theme_id = Some("/tmp/throngterm".to_string());

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
        .find(|chip| chip.label == "2xthrongterm")
        .expect("throngterm chip should exist")
        .clone();
    app.handle_header_filter_click(120, chip.rect.x, chip.rect.y);

    assert_eq!(
        app.thought_filter.cwd.as_deref(),
        Some(TEST_REPO_THRONGTERM)
    );
    assert_eq!(app.active_thought_filter_text(), "filter: pwd=throngterm");
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

    assert_eq!(
        app.thought_filter.cwd.as_deref(),
        Some(TEST_REPO_THRONGTERM)
    );
    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: pwd=throngterm");
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
        Some("focused 2")
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
fn clicking_thought_body_opens_that_session() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_THRONGTERM,
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
    let body_x = thought_content
        .x
        .saturating_add(display_width("7").saturating_add(3));
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
        Some("focused 7")
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
                TEST_REPO_THRONGTERM,
                "older",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-2",
                "9",
                TEST_REPO_THRONGTERM,
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
        vec!["9: latest", "thought", "stays at", "bottom"]
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
        vec![session_summary("sess-2", "9", TEST_REPO_THRONGTERM)],
        test_field(),
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-2",
            "9",
            TEST_REPO_THRONGTERM,
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
        Some("focused 9")
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
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_THRONGTERM,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let body_x = thought_content
        .x
        .saturating_add(display_width("7").saturating_add(3));

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
        TEST_REPO_THRONGTERM,
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
        .find(|chip| chip.label == "1xthrongterm")
        .expect("repo chip should exist");

    assert_eq!(panel.rows.len(), 1);
    assert_eq!(panel.rows[0].color, Color::Magenta);
    assert_eq!(chip.color, Color::Magenta);
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
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_THRONGTERM);
    session.state = SessionState::Attention;
    session.rest_state = RestState::Active;
    let entity = SessionEntity::new(session, field);
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, true, 0, &HashMap::new());

    assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, Color::Magenta);
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
    let theme_id = "/tmp/throngterm".to_string();
    let theme_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    let mut spawned_session = session_summary("sess-42", "42", TEST_REPO_THRONGTERM);
    spawned_session.repo_theme_id = Some(theme_id.clone());
    api.push_create_session(Ok(create_response_with_theme(
        spawned_session.clone(),
        repo_theme("#B89875"),
    )));
    let mut app = make_app(api);

    app.spawn_session(TEST_REPO_THRONGTERM, None, field);

    let mut thought_session = session_summary_with_thought(
        "sess-42",
        "42",
        TEST_REPO_THRONGTERM,
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
            TEST_REPO_THRONGTERM,
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
            TEST_REPO_THRONGTERM,
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
            TEST_REPO_THRONGTERM,
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
        TEST_REPO_THRONGTERM,
        RestState::Active,
        "2026-03-08T12:40:00Z",
    );
    let drowsy = attention_session(
        "sess-attn-drowsy",
        "8",
        TEST_REPO_THRONGTERM,
        RestState::Drowsy,
        "2026-03-08T12:20:00Z",
    );
    let sleeping = attention_session(
        "sess-attn-sleep",
        "9",
        TEST_REPO_THRONGTERM,
        RestState::Sleeping,
        "2026-03-08T12:00:00Z",
    );
    let deep_sleep = attention_session(
        "sess-attn-deep",
        "10",
        TEST_REPO_THRONGTERM,
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
    let colors_path = repo_root.join(".throngterm").join("colors.json");
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
    let repo_root = temp.path().join("throngterm");
    fs::create_dir_all(&repo_root).expect("create repo");
    write_repo_theme_file(&repo_root, "#4FA66A");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(
            temp.path().to_string_lossy().as_ref(),
            &[("throngterm", true)],
        ),
        true,
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
            sleeping_session(
                "sess-new",
                "8",
                TEST_REPO_THRONGTERM,
                "2026-03-08T12:20:00Z",
            ),
            sleeping_session(
                "sess-mid",
                "7",
                TEST_REPO_THRONGTERM,
                "2026-03-08T12:10:00Z",
            ),
            sleeping_session(
                "sess-old",
                "9",
                TEST_REPO_THRONGTERM,
                "2026-03-08T12:00:00Z",
            ),
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
            sleeping_session("sess-b", "8", TEST_REPO_THRONGTERM, "2026-03-08T12:00:00Z"),
            sleeping_session("sess-a", "7", TEST_REPO_THRONGTERM, "2026-03-08T12:00:00Z"),
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
            sleeping_session("sess-a", "7", TEST_REPO_THRONGTERM, "2026-03-08T12:00:00Z"),
            sleeping_session("sess-b", "8", TEST_REPO_THRONGTERM, "2026-03-08T12:10:00Z"),
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
                TEST_REPO_THRONGTERM,
                "2026-03-08T12:00:00Z",
            ),
            deep_sleep_session(
                "sess-deep-b",
                "8",
                TEST_REPO_THRONGTERM,
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
        &[("opensource", true), ("throngterm", true)],
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
        vec!["opensource", "throngterm"]
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
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_THRONGTERM)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("throngterm", false)]),
        true,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_THRONGTERM.to_string(),
        value: "add hidden spawn flow".to_string(),
    });

    app.submit_initial_request(field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_THRONGTERM.to_string(),
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
        cwd: TEST_REPO_THRONGTERM.to_string(),
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
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_THRONGTERM)));
    let field = test_field();
    let mut app = make_app(api.clone());
    let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_THRONGTERM.to_string(),
        value: String::new(),
    });

    app.handle_paste(pasted);
    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_THRONGTERM.to_string(),
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
        dir_response(TEST_REPOS_ROOT, &[("throngterm", false)]),
        true,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_THRONGTERM.to_string(),
        value: "fix tmux startup".to_string(),
    });

    app.submit_initial_request(field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_THRONGTERM.to_string(),
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
        cwd: TEST_REPO_THRONGTERM.to_string(),
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
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_THRONGTERM)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_THRONGTERM.to_string(),
        value: String::new(),
    });

    for ch in "add hidden spawn flow".chars() {
        app.handle_initial_request_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            field,
        );
    }
    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_THRONGTERM.to_string(),
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
        dir_response(TEST_REPOS_ROOT, &[("throngterm", false)]),
        true,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_THRONGTERM.to_string(),
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
fn clicking_existing_thronglet_still_opens_it_directly() {
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
fn filtered_out_thronglets_are_not_click_targets() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("throngterm", true)])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.entities
        .push(entity_at(field, "sess-1", "2", TEST_REPO_THRONGTERM, 12, 6));
    app.entities
        .push(entity_at(field, "sess-3", "9", TEST_REPO_SKILLS, 30, 8));
    app.selected_id = Some("sess-3".to_string());

    app.set_thought_filter_cwd(TEST_REPO_THRONGTERM.to_string());
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
            session_summary("sess-1", "7", TEST_REPO_THRONGTERM),
            session_summary("sess-2", "2", TEST_REPO_THRONGTERM),
        ],
        layout.overview_field,
    );
    app.selected_id = Some("sess-1".to_string());
    app.set_thought_filter_cwd(TEST_REPO_THRONGTERM.to_string());

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
        "sess-throngterm",
        "7",
        TEST_REPO_THRONGTERM,
    )]));
    let mut app = make_app(api.clone());

    app.refresh(layout);

    assert_eq!(app.selected_id.as_deref(), Some("sess-throngterm"));
    assert_eq!(
        api.publish_calls(),
        vec![Some("sess-throngterm".to_string())]
    );
}

#[test]
fn picker_action_at_resolves_controls_and_entries() {
    let mut picker = PickerState::new(
        4,
        4,
        dir_response("/tmp", &[("alpha", true), ("beta", false)]),
        true,
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
fn picker_activate_selection_opens_initial_request_and_reloads_children() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        2,
        2,
        dir_response("/tmp", &[("child", true), ("leaf", false)]),
        true,
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
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_THRONGTERM,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let body_x = thought_content
        .x
        .saturating_add(display_width("7").saturating_add(3));
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
        Some("focused 7")
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
fn refresh_builds_synthetic_mermaid_row_and_preserves_text_click_behavior() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    api.push_fetch_sessions(Ok(vec![session_summary(
        "sess-1",
        "7",
        TEST_REPO_THRONGTERM,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "sess-1",
        "/tmp/repos/throngterm/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph TD\nA-->B\n",
    )));
    let mut app = make_app(api);

    app.refresh(layout);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 1);
    assert_eq!(panel.rows[0].line, "7: mermaid diagram ready");
    let mermaid_rect = panel.rows[0].mermaid_rect.expect("mermaid button");
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
            text_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenSession {
            session_id: "sess-1".to_string(),
            label: "7".to_string(),
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
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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
    assert!(zoom_after_plus > 1.0);

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
    let (zoom_after_reset, center_after_reset_x, center_after_reset_y) =
        match &app.fish_bowl_mode {
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
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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

    let start_column = content_rect.x + 4;
    let start_row = content_rect.y + 2;
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
        MERMAID_ZOOM_STEP,
    ));
    let zoom_after_scroll = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.zoom,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(zoom_after_scroll > zoom_before_scroll);
}

#[test]
fn mermaid_render_reuses_prepared_source_state_across_zoom_and_pan() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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
    let (prepare_after_first, viewport_after_first, first_lines_empty) =
        match &app.fish_bowl_mode {
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
    let sessions = vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)];
    app.merge_sessions(sessions.clone(), layout.overview_field);
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow-a.mmd",
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
        "/tmp/repos/throngterm/flow-b.mmd",
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
        Some("/tmp/repos/throngterm/flow-b.mmd")
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
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
}

#[test]
fn mermaid_er_overview_hides_attribute_detail_until_zoomed() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(semantic_texts.contains(&"ORDER".to_string()));
    assert!(!semantic_texts.contains(&"email".to_string()));
    assert!(!semantic_texts.contains(&"user_id".to_string()));
    assert!(!semantic_texts.contains(&"uuid".to_string()));
    assert!(!semantic_texts.contains(&"places".to_string()));
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
}

#[test]
fn mermaid_flowchart_overview_hides_edge_labels_until_zoomed() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    assert!(find_text_position(&renderer, "Group One").is_some());
    assert!(find_text_position(&renderer, "Producer").is_some());
    assert!(find_text_position(&renderer, "Consumer").is_some());
    assert!(find_text_position(&renderer, "ships data").is_none());
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
}

#[test]
fn mermaid_flowchart_overview_compacts_long_node_labels() {
    let source =
        "graph TD\nA[1. Verified Identity And api cfo admin hierarchy role restricted]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts
            .iter()
            .any(|text| text.starts_with("1. Ver")),
        "{semantic_texts:?}"
    );
    assert!(!semantic_texts.iter().any(|text| text.contains("hierarchy")));
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
}

#[test]
fn mermaid_zoom_reveals_edge_labels_at_detail_l2() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
    ));
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        find_text_position(&renderer, "ships data").is_some(),
        "status row: {}; semantic_texts: {:?}",
        row_text(&renderer, layout.overview_field.y),
        semantic_texts
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("detail L2"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_er_zoom_reveals_attribute_names_before_types() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..3 {
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
    }
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(
        semantic_texts.contains(&"id".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        semantic_texts.contains(&"email".to_string()),
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
        row_text(&renderer, layout.overview_field.y).contains("detail L2"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_er_zoom_reveals_attribute_types_at_detail_l3() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..5 {
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
    }
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.contains(&"uuid".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        semantic_texts.contains(&"string".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("detail L3"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_reset_fit_hides_subordinate_detail() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..5 {
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
    }
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("detail L3"),
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
    assert!(!semantic_texts.contains(&"id".to_string()));
    assert!(!semantic_texts.contains(&"email".to_string()));
    assert!(!semantic_texts.contains(&"uuid".to_string()));
    assert!(!semantic_texts.contains(&"string".to_string()));
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
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

    app.zoom_mermaid_viewer(MERMAID_ZOOM_STEP, None, content_rect);
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
fn mermaid_sequence_diagram_falls_back_to_braille_only() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    app.render(&mut renderer, layout);

    let (render_error, cached_lines_empty, cached_semantic_lines_empty) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                viewer.render_error.clone(),
                viewer.cached_lines.is_empty(),
                viewer.cached_semantic_lines.is_empty(),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
    assert_eq!(render_error, None);
    assert!(!cached_lines_empty);
    assert!(cached_semantic_lines_empty);
    assert!(find_text_position(&renderer, "hello").is_none());
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
            owner_width: 20.0,
            owner_height: 8.0,
        }],
        MermaidViewportTransform {
            scale: 1.0,
            tx: -4.0,
            ty: 0.0,
        },
        content_rect,
        MermaidDetailLevel::L1,
    );

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].x, content_rect.x);
    assert_eq!(projected[0].y, content_rect.y + 1);
    assert_eq!(projected[0].text, "Alpha Node");
}

#[test]
fn mermaid_resize_reprojects_semantic_labels() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);
    let group_before = find_text_position(&renderer, "Group One").expect("Group One before");
    let producer_before = find_text_position(&renderer, "Producer").expect("Producer before");

    let resized_layout = test_layout(160, 48);
    let mut resized_renderer = test_renderer(160, 48);
    app.render(&mut resized_renderer, resized_layout);

    let group_after =
        find_text_position(&resized_renderer, "Group One").expect("Group One after");
    let producer_after =
        find_text_position(&resized_renderer, "Producer").expect("Producer after");
    assert_ne!(group_after, group_before);
    assert_ne!(producer_after, producer_before);
}

#[test]
fn mermaid_open_shortcut_uses_artifact_path_and_stays_in_viewer() {
    let api = MockApi::new();
    let opener = Arc::new(MockArtifactOpener::default());
    let layout = test_layout(120, 32);
    let mut app = make_app_with_artifact_opener(api, opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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
        vec!["/tmp/repos/throngterm/flow.mmd".to_string()]
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
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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
        vec!["/tmp/repos/throngterm/flow.mmd".to_string()]
    );

    let opener = Arc::new(MockArtifactOpener::default());
    let mut app = make_app_with_artifact_opener(MockApi::new(), opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
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
