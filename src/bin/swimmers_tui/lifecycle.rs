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
        let ready_fd = writer.as_raw_fd().to_string();

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
    let probe_url = format!("{}/v1/sessions", base_url.trim_end_matches('/'));
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
    if let Some(override_path) = override_path {
        return resolve_override_binary_path(override_path);
    }

    let current_exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve current executable path: {err}"))?;
    if let Some(parent) = current_exe.parent() {
        let sibling = parent.join("swimmers");
        if is_executable_file(&sibling) {
            return Ok(sibling);
        }
    }

    if let Some(path_binary) = find_binary_on_path("swimmers") {
        return Ok(path_binary);
    }

    Err(
        "could not locate `swimmers` server binary; set SWIMMERS_SERVER_BIN to an absolute executable path"
            .to_string(),
    )
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
    if !is_executable_file(&path) {
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
    let path = std::env::var_os("PATH")?;
    for candidate_dir in std::env::split_paths(&path) {
        let candidate = candidate_dir.join(binary);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn resolve_server_log_path(
    base_url: &reqwest::Url,
    log_override: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(path) = log_override {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = std::env::var_os("TUI_SERVER_LOG") {
        return Ok(PathBuf::from(path));
    }

    let dir = std::env::var_os("TUI_SERVER_LOG_DIR")
        .or_else(|| std::env::var_os("TMPDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));

    let port = base_url
        .port_or_known_default()
        .ok_or_else(|| format!("could not determine port for startup URL `{base_url}`"))?;

    Ok(dir.join(format!("swimmers-tui-server-{port}.log")))
}

fn spawn_server_process(
    server_bin: &Path,
    ready_fd: &str,
    log_path: &Path,
) -> Result<Child, String> {
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

    Command::new(server_bin)
        .env("SWIMMERS_READY_FD", ready_fd)
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

    #[test]
    fn read_ready_byte_reports_eof() {
        let (reader, writer) = os_pipe::pipe().expect("pipe");
        drop(writer);
        let err = read_ready_byte(reader).expect_err("closed writer should produce EOF");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
