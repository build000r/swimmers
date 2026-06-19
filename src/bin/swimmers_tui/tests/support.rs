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

const EMBEDDED_FIRST_FRAME_BUDGET: Duration = Duration::from_millis(80);

fn p95_duration(mut samples: Vec<Duration>) -> Duration {
    assert!(!samples.is_empty(), "p95 requires at least one sample");
    samples.sort_unstable();
    let index = samples
        .len()
        .saturating_mul(95)
        .div_ceil(100)
        .saturating_sub(1);
    samples[index]
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
    result: Option<CommitGrokLaunch>,
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
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitGrokLaunch> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(session.clone());
        if let Some(message) = state.error.clone() {
            return Err(io::Error::other(message));
        }
        Ok(state.result.clone().unwrap_or(CommitGrokLaunch {
            session_name: "commit-7-123".to_string(),
            watch_command: "tmux a -t commit-7-123".to_string(),
        }))
    }
}

fn make_app(api: MockApi) -> App<MockApi> {
    let mut app = App::new(test_runtime(), api);
    app.thought_show_all = true;
    app
}

fn make_app_with_artifact_opener(
    api: MockApi,
    artifact_opener: Arc<dyn ArtifactOpener>,
) -> App<MockApi> {
    let mut app = App::with_artifact_opener(test_runtime(), api, artifact_opener);
    app.thought_show_all = true;
    app
}

fn make_app_with_commit_launcher(
    api: MockApi,
    commit_launcher: Arc<dyn CommitLauncher>,
) -> App<MockApi> {
    let mut app = App::with_helpers(
        test_runtime(),
        api,
        Arc::new(SystemArtifactOpener),
        commit_launcher,
    );
    app.thought_show_all = true;
    app
}

fn test_http_client(timeout: Duration) -> Client {
    Client::builder()
        .connect_timeout(Duration::from_millis(50))
        .timeout(timeout)
        .build()
        .expect("http client")
}

fn test_api_client(base_url: String, auth_token: Option<&str>) -> ApiClient {
    ApiClient {
        http: test_http_client(Duration::from_millis(100)),
        startup_http: test_http_client(Duration::from_millis(250)),
        base_url,
        auth_token: auth_token.map(str::to_string),
        startup_wait_timeout: Duration::from_millis(400),
        startup_retry_interval: Duration::from_millis(10),
    }
}

fn restore_env_var(key: &str, value: Option<String>) {
    match value {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
}

struct EnvVarGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = env::var(key).ok();
        env::set_var(key, value);
        Self { key, original }
    }

    fn remove(key: &'static str) -> Self {
        let original = env::var(key).ok();
        env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        restore_env_var(self.key, self.original.take());
    }
}

fn restore_os_env_var(key: &str, value: Option<OsString>) {
    match value {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
}

fn prepend_test_path(bin_dir: &Path, original_path: Option<&OsStr>) {
    let mut entries = vec![bin_dir.as_os_str().to_os_string()];
    if let Some(existing) = original_path {
        entries.extend(env::split_paths(existing).map(|path| path.into_os_string()));
    }
    env::set_var("PATH", env::join_paths(entries).expect("join fake PATH"));
}

fn install_fake_tmux(script: &str) -> (tempfile::TempDir, Option<OsString>) {
    let dir = tempdir().expect("fake tmux tempdir");
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("fake tmux bin dir");
    let tmux_path = bin_dir.join("tmux");
    fs::write(&tmux_path, script).expect("write fake tmux");
    let mut perms = fs::metadata(&tmux_path)
        .expect("fake tmux metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&tmux_path, perms).expect("mark fake tmux executable");

    let original_path = env::var_os("PATH");
    prepend_test_path(&bin_dir, original_path.as_deref());
    (dir, original_path)
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

/// Poll `pending_refresh` until the background result arrives and is applied.
fn poll_until_refresh(app: &mut App<MockApi>, layout: WorkspaceLayout) {
    for _ in 0..200 {
        app.poll_refresh(layout);
        if app.pending_refresh.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("background refresh did not complete within timeout");
}

/// Poll `pending_interaction` until the background result arrives and is applied.
fn poll_until_interaction(app: &mut App<MockApi>) {
    for _ in 0..200 {
        app.poll_pending_interaction();
        if app.pending_interaction.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("background interaction did not complete within timeout");
}

fn poll_until_picker_repo_search(app: &mut App<MockApi>) {
    for _ in 0..200 {
        app.poll_pending_picker_repo_search();
        if app.pending_picker_repo_search.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("picker repository search did not complete within timeout");
}

/// Poll selection publication until there is no in-flight or queued publish left.
fn poll_until_selection_publication(app: &mut App<MockApi>) {
    for _ in 0..200 {
        app.poll_pending_selection_publication();
        if app.pending_selection_publication.is_none() && app.queued_selection_publication.is_none()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("selection publication did not complete within timeout");
}
