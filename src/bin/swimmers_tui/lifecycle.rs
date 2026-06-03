use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Clone)]
struct EnsureServerOpts {
    server_bin_override: Option<PathBuf>,
    log_path_override: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerHandle {
    External,
    Managed {
        pid: u32,
    },
    #[allow(dead_code)]
    InProcess,
}

pub(crate) async fn ensure_server(
    base_url: &str,
    timeout: Duration,
) -> Result<ServerHandle, String> {
    let opts = EnsureServerOpts {
        server_bin_override: std::env::var_os("SWIMMERS_SERVER_BIN").map(PathBuf::from),
        log_path_override: std::env::var_os("TUI_SERVER_LOG").map(PathBuf::from),
    };
    ensure_server_with_opts(base_url, timeout, opts).await
}

async fn ensure_server_with_opts(
    base_url: &str,
    timeout: Duration,
    opts: EnsureServerOpts,
) -> Result<ServerHandle, String> {
    let parsed_url = reqwest::Url::parse(base_url)
        .map_err(|err| format!("invalid base URL `{base_url}`: {err}"))?;

    if quick_probe_alive(base_url).await {
        return Ok(ServerHandle::External);
    }

    if !is_loopback_target(&parsed_url) {
        return Err(format!(
            "swimmers API at {base_url} is unreachable and auto-start is only supported for loopback targets"
        ));
    }

    #[cfg(not(unix))]
    {
        let _ = (timeout, opts);
        return Err(
            "local server auto-start via readiness fd is currently only implemented on unix targets"
                .to_string(),
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;

        let wait_timeout = if timeout.is_zero() {
            DEFAULT_READY_TIMEOUT
        } else {
            timeout
        };

        let server_bin = resolve_server_binary(opts.server_bin_override.as_deref())?;
        let log_path = resolve_server_log_path(&parsed_url, opts.log_path_override.as_deref())?;
        let (reader, writer) =
            os_pipe::pipe().map_err(|err| format!("failed to create readiness pipe: {err}"))?;
        let ready_fd = writer.as_raw_fd();

        let mut child = spawn_server_process(&server_bin, &ready_fd, &log_path)?;
        let pid = child.id();

        // Parent only reads readiness; child is sole writer.
        drop(writer);

        let read_task = tokio::task::spawn_blocking(move || read_ready_byte(reader));

        match tokio::time::timeout(wait_timeout, read_task).await {
            Ok(Ok(Ok(b'R'))) => {
                spawn_reaper(child, log_path.clone());
                Ok(ServerHandle::Managed { pid })
            }
            Ok(Ok(Ok(other))) => {
                terminate_child(&mut child);
                Err(format!(
                    "swimmers server signaled unexpected readiness byte {other} (expected 82). See log: {}",
                    log_path.display()
                ))
            }
            Ok(Ok(Err(err))) if err.kind() == io::ErrorKind::UnexpectedEof => Err(format!(
                "swimmers server exited before signaling readiness (EOF). See log: {}",
                log_path.display()
            )),
            Ok(Ok(Err(err))) => {
                terminate_child(&mut child);
                Err(format!(
                    "failed while reading swimmers readiness signal: {err}. See log: {}",
                    log_path.display()
                ))
            }
            Ok(Err(join_err)) => {
                terminate_child(&mut child);
                Err(format!(
                    "readiness wait task failed: {join_err}. See log: {}",
                    log_path.display()
                ))
            }
            Err(_) => {
                terminate_child(&mut child);
                Err(format!(
                    "timed out waiting {:?} for swimmers readiness signal. See log: {}",
                    wait_timeout,
                    log_path.display()
                ))
            }
        }
    }
}

async fn quick_probe_alive(base_url: &str) -> bool {
    // Probe `/health` rather than `/v1/sessions`: the latter fans out to all
    // configured remote launch targets (see `list_remote_sessions`) and can
    // take 900ms+ when a tailnet peer is unreachable. With PROBE_TIMEOUT
    // bounded at 500ms that race made the probe falsely conclude an existing
    // backend was dead, after which the TUI would try to spawn a duplicate
    // server, fail to bind the already-occupied port, and surface a confusing
    // startup error. `/health` is unauthenticated, cheap, and only checks
    // local subsystems.
    let probe_url = format!("{}/health", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .connect_timeout(PROBE_TIMEOUT)
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "failed to build startup probe client");
            return false;
        }
    };

    match client.get(probe_url).send().await {
        Ok(response) => {
            tracing::debug!(status = %response.status(), "startup probe saw an existing server");
            true
        }
        Err(err) => {
            tracing::debug!(error = %err, "startup probe could not reach server");
            false
        }
    }
}

fn is_loopback_target(url: &reqwest::Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false),
        None => false,
    }
}

fn resolve_server_binary(override_path: Option<&Path>) -> Result<PathBuf, String> {
    match override_path {
        Some(path) => resolve_override_binary_path(path),
        None => resolve_default_server_binary(),
    }
}

fn resolve_default_server_binary() -> Result<PathBuf, String> {
    current_exe_sibling_server_binary().and_then(resolve_default_server_binary_with_sibling)
}

fn resolve_default_server_binary_with_sibling(
    current_exe_sibling: Option<PathBuf>,
) -> Result<PathBuf, String> {
    resolve_default_server_binary_from(current_exe_sibling, || find_binary_on_path("swimmers"))
}

fn resolve_default_server_binary_from(
    current_exe_sibling: Option<PathBuf>,
    path_search: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, String> {
    current_exe_sibling
        .or_else(path_search)
        .ok_or_else(default_server_binary_not_found_error)
}

fn default_server_binary_not_found_error() -> String {
    "could not locate `swimmers` server binary; set SWIMMERS_SERVER_BIN to an absolute executable path"
        .to_string()
}

fn current_exe_sibling_server_binary() -> Result<Option<PathBuf>, String> {
    let current_exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve current executable path: {err}"))?;

    Ok(current_exe
        .parent()
        .map(|parent| parent.join("swimmers"))
        .filter(|sibling| is_executable_file(sibling)))
}

fn resolve_override_binary_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "SWIMMERS_SERVER_BIN must be an absolute path, got `{}`",
            path.display()
        ));
    }
    if !path.exists() {
        return Err(format!(
            "SWIMMERS_SERVER_BIN points to `{}` but that file does not exist",
            path.display()
        ));
    }
    if !is_executable_file(path) {
        return Err(format!(
            "SWIMMERS_SERVER_BIN points to `{}` but it is not executable",
            path.display()
        ));
    }
    Ok(path.to_path_buf())
}

fn is_executable_file(path: &Path) -> bool {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn find_binary_on_path(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| find_binary_in_path_value(binary, &path))
}

fn find_binary_in_path_value(binary: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|candidate_dir| candidate_dir.join(binary))
        .find(|candidate| is_executable_file(candidate))
}

fn resolve_server_log_path(
    base_url: &reqwest::Url,
    log_override: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit_server_log_path(log_override) {
        return Ok(PathBuf::from(path));
    }

    let port = startup_url_port(base_url)?;
    Ok(default_server_log_dir().join(format!("swimmers-tui-server-{port}.log")))
}

fn explicit_server_log_path(log_override: Option<&Path>) -> Option<PathBuf> {
    log_override
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("TUI_SERVER_LOG").map(PathBuf::from))
}

fn default_server_log_dir() -> PathBuf {
    default_server_log_dir_from(
        std::env::var_os("TUI_SERVER_LOG_DIR"),
        std::env::var_os("TMPDIR"),
    )
}

fn default_server_log_dir_from(
    tui_server_log_dir: Option<impl Into<OsString>>,
    tmpdir: Option<impl Into<OsString>>,
) -> PathBuf {
    tui_server_log_dir
        .map(Into::into)
        .or_else(|| tmpdir.map(Into::into))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn startup_url_port(base_url: &reqwest::Url) -> Result<u16, String> {
    base_url
        .port_or_known_default()
        .ok_or_else(|| format!("could not determine port for startup URL `{base_url}`"))
}

#[cfg(unix)]
fn spawn_server_process(
    server_bin: &Path,
    ready_fd: &i32,
    log_path: &Path,
) -> Result<Child, String> {
    use std::os::unix::process::CommandExt;

    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create server log directory `{}`: {err}",
                parent.display()
            )
        })?;
    }

    let log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
        .map_err(|err| {
            format!(
                "failed to open server log file `{}`: {err}",
                log_path.display()
            )
        })?;
    let stdout = log_file
        .try_clone()
        .map_err(|err| format!("failed to clone server log file handle: {err}"))?;
    let stderr = log_file
        .try_clone()
        .map_err(|err| format!("failed to clone server log file handle: {err}"))?;

    let ready_fd_for_child = *ready_fd;
    let ready_fd_env = ready_fd_for_child.to_string();
    let mut command = Command::new(server_bin);

    // os_pipe intentionally creates non-inheritable fds. The parent copy should
    // stay close-on-exec; only the forked child copy is made inheritable so the
    // server can consume SWIMMERS_READY_FD after exec.
    unsafe {
        command.pre_exec(move || clear_fd_cloexec(ready_fd_for_child));
    }

    command
        .env("SWIMMERS_READY_FD", ready_fd_env)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|err| {
            format!(
                "failed to spawn swimmers server `{}`: {err}. See log: {}",
                server_bin.display(),
                log_path.display()
            )
        })
}

#[cfg(unix)]
fn clear_fd_cloexec(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }

    let updated = flags & !libc::FD_CLOEXEC;
    if unsafe { libc::fcntl(fd, libc::F_SETFD, updated) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn read_ready_byte(mut reader: os_pipe::PipeReader) -> io::Result<u8> {
    let mut byte = [0_u8; 1];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn terminate_child(child: &mut Child) {
    match child.kill() {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::InvalidInput => {}
        Err(err) => {
            tracing::warn!(error = %err, pid = child.id(), "failed to kill managed swimmers server");
        }
    }
    let _ = child.wait();
}

fn spawn_reaper(mut child: Child, log_path: PathBuf) {
    let pid = child.id();
    let _ = std::thread::Builder::new()
        .name(format!("swimmers-server-reaper-{pid}"))
        .spawn(move || match child.wait() {
            Ok(status) => {
                tracing::warn!(
                    pid,
                    status = %status,
                    log_path = %log_path.display(),
                    "managed swimmers server exited"
                );
            }
            Err(err) => {
                tracing::warn!(
                    pid,
                    error = %err,
                    log_path = %log_path.display(),
                    "failed while reaping managed swimmers server"
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::OsString;
    use std::io;

    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn closed_loopback_base_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback socket");
        let port = listener
            .local_addr()
            .expect("resolve loopback socket addr")
            .port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn ensure_server_probe_existing_server_returns_external() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe fixture");
        let addr = listener.local_addr().expect("fixture addr");

        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept probe request");
            let mut req_buf = [0_u8; 1024];
            let _ = stream.read(&mut req_buf).await;
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("write 401 response");
        });

        let base_url = format!("http://{addr}");
        let result = ensure_server_with_opts(
            &base_url,
            Duration::from_secs(1),
            EnsureServerOpts::default(),
        )
        .await;
        assert!(matches!(result, Ok(ServerHandle::External)));

        server_task.await.expect("fixture task join");
    }

    #[tokio::test]
    async fn ensure_server_non_loopback_unreachable_returns_error_without_spawn() {
        let err = ensure_server_with_opts(
            "http://not-a-real-host.invalid:3210",
            Duration::from_secs(1),
            EnsureServerOpts::default(),
        )
        .await
        .expect_err("non-loopback unreachable target should error");
        assert!(err.contains("loopback"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn ensure_server_spawn_eof_returns_clean_error() {
        let temp_dir = tempdir().expect("temp dir");
        let override_bin = ["/bin/true", "/usr/bin/true"]
            .iter()
            .map(PathBuf::from)
            .find(|path| path.exists())
            .expect("true binary should exist");
        let override_log = temp_dir.path().join("server-eof.log");
        let opts = EnsureServerOpts {
            server_bin_override: Some(override_bin.clone()),
            log_path_override: Some(override_log.clone()),
        };

        let err =
            ensure_server_with_opts(&closed_loopback_base_url(), Duration::from_secs(1), opts)
                .await
                .expect_err("/bin/true should exit before readiness signal");

        assert!(
            err.contains("EOF") || err.contains("before signaling readiness"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains(&override_log.display().to_string()),
            "error should include log path: {err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_server_spawn_passes_ready_fd_to_child() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir().expect("temp dir");
        let script_path = temp_dir.path().join("ready-server.sh");
        fs::write(
            &script_path,
            "#!/bin/sh\nprintf R >\"/dev/fd/${SWIMMERS_READY_FD}\"\n",
        )
        .expect("write ready script");

        let mut permissions = fs::metadata(&script_path)
            .expect("ready script metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod ready script");

        let override_log = temp_dir.path().join("server-ready.log");
        let opts = EnsureServerOpts {
            server_bin_override: Some(script_path),
            log_path_override: Some(override_log),
        };

        let result =
            ensure_server_with_opts(&closed_loopback_base_url(), Duration::from_secs(1), opts)
                .await
                .expect("ready script should inherit SWIMMERS_READY_FD and signal readiness");

        assert!(matches!(result, ServerHandle::Managed { .. }));
    }

    #[tokio::test]
    async fn ensure_server_loopback_missing_override_binary_errors_before_spawn() {
        let temp_dir = tempdir().expect("temp dir");
        let missing_bin = temp_dir.path().join("definitely-not-here-swimmers");
        let opts = EnsureServerOpts {
            server_bin_override: Some(missing_bin),
            log_path_override: None,
        };

        let err =
            ensure_server_with_opts(&closed_loopback_base_url(), Duration::from_secs(1), opts)
                .await
                .expect_err("missing override binary should fail fast");

        assert!(
            err.contains("SWIMMERS_SERVER_BIN") && err.contains("does not exist"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn is_loopback_target_detects_local_hosts() {
        let localhost = reqwest::Url::parse("http://localhost:3210").expect("localhost url");
        let loopback_v4 = reqwest::Url::parse("http://127.0.0.1:3210").expect("ipv4 url");
        let loopback_v6 = reqwest::Url::parse("http://[::1]:3210").expect("ipv6 url");
        let remote = reqwest::Url::parse("http://example.com:3210").expect("remote url");

        assert!(is_loopback_target(&localhost));
        assert!(is_loopback_target(&loopback_v4));
        assert!(is_loopback_target(&loopback_v6));
        assert!(!is_loopback_target(&remote));
    }

    #[test]
    fn resolve_override_binary_path_requires_absolute_and_existing_executable() {
        let relative = resolve_override_binary_path(Path::new("swimmers"))
            .expect_err("relative override should fail");
        assert!(relative.contains("absolute path"));

        let missing = resolve_override_binary_path(Path::new("/definitely/missing/swimmers"))
            .expect_err("missing override should fail");
        assert!(missing.contains("does not exist"));
    }

    fn write_executable_fixture(path: &Path) {
        fs::write(path, "#!/bin/sh\nexit 0\n").expect("write executable fixture");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(path)
                .expect("executable fixture metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("chmod executable fixture");
        }
    }

    #[test]
    fn resolve_default_server_binary_prefers_current_exe_sibling() {
        let sibling = PathBuf::from("/tmp/current-exe-sibling-swimmers");
        let resolved = resolve_default_server_binary_from(Some(sibling.clone()), || {
            panic!("PATH search should not run when current-exe sibling exists")
        })
        .expect("sibling candidate should resolve");

        assert_eq!(resolved, sibling);
    }

    #[test]
    fn resolve_default_server_binary_falls_back_to_path_search() {
        let path_binary = PathBuf::from("/tmp/path-swimmers");
        let resolved = resolve_default_server_binary_from(None, || Some(path_binary.clone()))
            .expect("PATH candidate should resolve");

        assert_eq!(resolved, path_binary);
    }

    #[test]
    fn resolve_default_server_binary_reports_missing_candidates() {
        let err = resolve_default_server_binary_from(None, || None)
            .expect_err("missing candidates should fail");

        assert_eq!(
            err,
            "could not locate `swimmers` server binary; set SWIMMERS_SERVER_BIN to an absolute executable path"
        );
    }

    #[test]
    fn resolve_default_server_binary_uses_path_when_no_sibling_exists() {
        let temp_dir = tempdir().expect("temp dir");
        let path_binary = temp_dir.path().join("swimmers");
        write_executable_fixture(&path_binary);

        let current_sibling =
            current_exe_sibling_server_binary().expect("current executable path should resolve");

        with_env_var("PATH", temp_dir.path().as_os_str(), || {
            let resolved =
                resolve_default_server_binary().expect("default server binary should resolve");
            assert_eq!(resolved, current_sibling.unwrap_or(path_binary));
        });
    }

    #[test]
    fn find_binary_on_path_returns_none_when_path_is_missing() {
        with_env_var_removed("PATH", || {
            assert_eq!(find_binary_on_path("swimmers"), None);
        });
    }

    #[test]
    fn find_binary_on_path_skips_missing_and_non_executable_candidates() {
        let first_dir = tempdir().expect("first temp dir");
        let second_dir = tempdir().expect("second temp dir");
        let third_dir = tempdir().expect("third temp dir");

        fs::write(first_dir.path().join("swimmers"), "#!/bin/sh\nexit 1\n")
            .expect("write non-executable candidate");
        let executable = second_dir.path().join("swimmers");
        write_executable_fixture(&executable);

        let path = std::env::join_paths([first_dir.path(), third_dir.path(), second_dir.path()])
            .expect("join fixture PATH");

        with_env_var("PATH", path, || {
            assert_eq!(find_binary_on_path("swimmers"), Some(executable));
        });
    }

    #[test]
    fn find_binary_in_path_value_returns_none_without_executable_candidate() {
        let first_dir = tempdir().expect("first temp dir");
        let second_dir = tempdir().expect("second temp dir");
        fs::write(second_dir.path().join("swimmers"), "#!/bin/sh\nexit 1\n")
            .expect("write non-executable candidate");
        let path =
            std::env::join_paths([first_dir.path(), second_dir.path()]).expect("join fixture PATH");

        assert_eq!(find_binary_in_path_value("swimmers", &path), None);
    }

    fn with_env_var_removed<T>(key: &str, run: impl FnOnce() -> T) -> T {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        let result = run();
        restore_env_var(key, previous);
        result
    }

    fn with_env_var<T>(key: &str, value: impl Into<OsString>, run: impl FnOnce() -> T) -> T {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value.into());
        let result = run();
        restore_env_var(key, previous);
        result
    }

    fn restore_env_var(key: &str, value: Option<OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn resolve_server_log_path_prefers_explicit_override() {
        let url = reqwest::Url::parse("http://127.0.0.1:3210").expect("startup url");
        let override_path = Path::new("/tmp/explicit-swimmers.log");

        with_env_var("TUI_SERVER_LOG", "/tmp/env-swimmers.log", || {
            let path = resolve_server_log_path(&url, Some(override_path))
                .expect("explicit override should resolve");
            assert_eq!(path, override_path);
        });
    }

    #[test]
    fn resolve_server_log_path_uses_env_then_default_dir() {
        let url = reqwest::Url::parse("http://127.0.0.1:4321").expect("startup url");
        let temp_dir = tempdir().expect("temp dir");
        let env_log = temp_dir.path().join("env.log");

        with_env_var("TUI_SERVER_LOG", env_log.as_os_str(), || {
            let path = resolve_server_log_path(&url, None).expect("env log path should resolve");
            assert_eq!(path, env_log);
        });

        with_env_var_removed("TUI_SERVER_LOG", || {
            with_env_var("TUI_SERVER_LOG_DIR", temp_dir.path().as_os_str(), || {
                let path =
                    resolve_server_log_path(&url, None).expect("default log path should resolve");
                assert_eq!(path, temp_dir.path().join("swimmers-tui-server-4321.log"));
            });
        });
    }

    #[test]
    fn default_server_log_dir_prefers_tui_dir_then_tmpdir_then_tmp() {
        with_env_var("TUI_SERVER_LOG_DIR", "/tmp/tui-log-dir", || {
            with_env_var("TMPDIR", "/tmp/tmpdir-log-dir", || {
                assert_eq!(default_server_log_dir(), PathBuf::from("/tmp/tui-log-dir"));
            });
        });

        with_env_var_removed("TUI_SERVER_LOG_DIR", || {
            with_env_var("TMPDIR", "/tmp/tmpdir-log-dir", || {
                assert_eq!(
                    default_server_log_dir(),
                    PathBuf::from("/tmp/tmpdir-log-dir")
                );
            });
        });

        with_env_var_removed("TUI_SERVER_LOG_DIR", || {
            with_env_var_removed("TMPDIR", || {
                assert_eq!(default_server_log_dir(), PathBuf::from("/tmp"));
            });
        });
    }

    #[test]
    fn default_server_log_dir_from_preserves_precedence() {
        assert_eq!(
            default_server_log_dir_from(Some("/tmp/tui"), Some("/tmp/tmpdir")),
            PathBuf::from("/tmp/tui")
        );
        assert_eq!(
            default_server_log_dir_from(None::<OsString>, Some("/tmp/tmpdir")),
            PathBuf::from("/tmp/tmpdir")
        );
        assert_eq!(
            default_server_log_dir_from(None::<OsString>, None::<OsString>),
            PathBuf::from("/tmp")
        );
    }

    #[test]
    fn read_ready_byte_reports_eof() {
        let (reader, writer) = os_pipe::pipe().expect("pipe");
        drop(writer);
        let err = read_ready_byte(reader).expect_err("closed writer should produce EOF");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
