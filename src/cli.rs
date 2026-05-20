//! Clap-based CLI shell for the `swimmers` and `swimmers-tui` binaries.
//!
//! This module exists for two reasons:
//!
//! 1. Provide standard `--help` and `--version` on both binaries so that
//!    `cargo install swimmers` produces a tool that behaves like a normal
//!    Unix CLI.
//! 2. Provide a `swimmers config` and `swimmers config doctor` subcommand
//!    that surfaces all environment variables and validates the active
//!    configuration before the user starts the server.
//!
//! Configuration of the running server itself is still purely through
//! environment variables — clap is intentionally only used for subcommands,
//! not as a replacement for env-var-based config. See README.md.

use std::io::ErrorKind;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};

use crate::config::{AuthMode, Config};
use crate::thought::emitter_client::resolve_clawgs_bin;
use crate::thought::runtime_config::DaemonDefaults;

/// Documented environment variables exposed via `swimmers config`.
///
/// Order matches the README env-var table so the two cannot drift in
/// presentation. Defaults are pulled from [`Config::default`] (or other
/// well-known defaults) so the table also cannot drift from runtime.
const ENV_VARS: &[&str] = &[
    "PORT",
    "SWIMMERS_BIND",
    "AUTH_MODE",
    "AUTH_TOKEN",
    "OBSERVER_TOKEN",
    "SWIMMERS_NATIVE_APP",
    "SWIMMERS_ATTENTION_GROUP_SIZE",
    "SWIMMERS_ATTENTION_GROUP_LAYOUT",
    "SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED",
    "SWIMMERS_THOUGHT_BACKEND",
    "CLAWGS_BIN",
    "SWIMMERS_REPLAY_BUFFER_SIZE",
    "SWIMMERS_DATA_DIR",
    "SWIMMERS_TUI_URL",
];

/// Variables whose values must never be printed in plaintext.
const SECRET_VARS: &[&str] = &["AUTH_TOKEN", "OBSERVER_TOKEN"];

const ENV_VAR_HELP: &str = "ENVIRONMENT VARIABLES:
  PORT                         Server listen port (default: 3210)
  SWIMMERS_BIND                Server bind address (default: 127.0.0.1)
  AUTH_MODE                    'local_trust', 'tailnet_trust', or 'token' (default: local_trust)
  AUTH_TOKEN                   Bearer token when AUTH_MODE=token
  OBSERVER_TOKEN               Read-only bearer token (optional)
  SWIMMERS_NATIVE_APP          'iterm' or 'ghostty' (default: iterm)
  SWIMMERS_ATTENTION_GROUP_SIZE
                               Number of panes in the attention group, 1-6 (default: 6)
  SWIMMERS_ATTENTION_GROUP_LAYOUT
                               tmux layout: tiled, even-horizontal, even-vertical, main-horizontal, main-vertical
  SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED
                               '1' to include non-numbered tmux sessions in attention groups
  SWIMMERS_THOUGHT_BACKEND     'daemon' or 'inproc' (default: daemon)
  CLAWGS_BIN                   Override path to the clawgs binary
  SWIMMERS_REPLAY_BUFFER_SIZE  Replay ring size in bytes (default: 524288)
  SWIMMERS_DATA_DIR            Override the data directory
  SWIMMERS_TUI_URL             API URL the TUI connects to

Run `swimmers config` to see resolved values.
Run `swimmers config doctor` to validate the active configuration.";

const TUI_ENV_HELP: &str = "ENVIRONMENT VARIABLES:
  SWIMMERS_TUI_URL  API URL to connect to (default: http://127.0.0.1:3210)
  AUTH_MODE         'local_trust', 'tailnet_trust', or 'token'
  AUTH_TOKEN        Bearer token when AUTH_MODE=token
  CLAWGS_BIN        Override path to the clawgs binary in embedded mode";

const CLAWGS_DEFAULTS_DOCTOR_TIMEOUT: Duration = Duration::from_secs(3);

/// Top-level CLI for the `swimmers` server binary.
///
/// With no subcommand the binary runs the API server (preserving the
/// pre-clap behavior).
#[derive(Parser, Debug)]
#[command(
    name = "swimmers",
    bin_name = "swimmers",
    version,
    about = "Axum API server that turns tmux sessions into an animated aquarium",
    after_help = ENV_VAR_HELP,
)]
pub struct ServerCli {
    #[command(subcommand)]
    pub command: Option<ServerCommand>,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum ServerCommand {
    #[command(about = "Run the API server (same as bare `swimmers`).")]
    Serve,

    /// Show resolved configuration and run validation checks.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Helpers for launching tmux sessions that follow Swimmers numeric names.
    Tmux {
        #[command(subcommand)]
        action: TmuxAction,
    },
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum ConfigAction {
    /// Run validation checks against the active environment.
    ///
    /// Exits 0 if all checks pass, 1 otherwise. Doctor is advisory — the
    /// server itself also enforces trusted bind-address gates at startup.
    Doctor,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum TmuxAction {
    /// Print the next numeric tmux session name Swimmers would use.
    NextName,

    /// Create and attach a new tmux session with the next numeric name.
    New {
        /// Start the tmux session in this directory instead of the current directory.
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
}

/// Top-level CLI for the `swimmers-tui` client binary.
#[derive(Parser, Debug)]
#[command(
    name = "swimmers-tui",
    bin_name = "swimmers-tui",
    version,
    about = "Terminal UI client for the swimmers API server",
    after_help = TUI_ENV_HELP,
)]
pub struct TuiCli {}

/// One row in the `swimmers config` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVarRow {
    pub name: &'static str,
    pub default: String,
    pub current: String,
    pub source: &'static str,
}

/// Build the `swimmers config` table from the current environment.
///
/// Defaults are pulled from [`Config::default`] where possible so that the
/// table cannot silently drift from runtime defaults. Secret variables are
/// rendered as `***` when present.
pub fn env_var_rows() -> Vec<EnvVarRow> {
    let defaults = Config::default();
    ENV_VARS
        .iter()
        .map(|name| {
            let default = default_for(name, &defaults);
            let (current, source) = match std::env::var(name) {
                Ok(val) if !val.is_empty() => {
                    let rendered = if SECRET_VARS.contains(name) {
                        "***".to_string()
                    } else {
                        val
                    };
                    (rendered, "env")
                }
                _ => (default.clone(), "default"),
            };
            EnvVarRow {
                name,
                default,
                current,
                source,
            }
        })
        .collect()
}

fn default_for(name: &str, config: &Config) -> String {
    match name {
        "PORT" => config.port.to_string(),
        "SWIMMERS_BIND" => config.bind.clone(),
        "AUTH_MODE" => match config.auth_mode {
            AuthMode::LocalTrust => "local_trust".to_string(),
            AuthMode::TailnetTrust => "tailnet_trust".to_string(),
            AuthMode::Token => "token".to_string(),
        },
        "AUTH_TOKEN" | "OBSERVER_TOKEN" => "(unset)".to_string(),
        "SWIMMERS_NATIVE_APP" => "iterm".to_string(),
        "SWIMMERS_ATTENTION_GROUP_SIZE" => "6".to_string(),
        "SWIMMERS_ATTENTION_GROUP_LAYOUT" => "tiled".to_string(),
        "SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED" => "(unset)".to_string(),
        "SWIMMERS_THOUGHT_BACKEND" => "daemon".to_string(),
        "CLAWGS_BIN" => "(auto)".to_string(),
        "SWIMMERS_REPLAY_BUFFER_SIZE" => config.replay_buffer_size.to_string(),
        "SWIMMERS_DATA_DIR" => "(platform data dir)".to_string(),
        "SWIMMERS_TUI_URL" => "http://127.0.0.1:3210".to_string(),
        _ => "(unknown)".to_string(),
    }
}

/// Print the `swimmers config` table to stdout.
pub fn print_config_table() {
    let rows = env_var_rows();
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(0).max(4);
    let default_w = rows
        .iter()
        .map(|r| r.default.len())
        .max()
        .unwrap_or(0)
        .max(7);
    let current_w = rows
        .iter()
        .map(|r| r.current.len())
        .max()
        .unwrap_or(0)
        .max(7);

    println!(
        "{:<nw$}  {:<dw$}  {:<cw$}  SOURCE",
        "NAME",
        "DEFAULT",
        "CURRENT",
        nw = name_w,
        dw = default_w,
        cw = current_w,
    );
    for row in &rows {
        println!(
            "{:<nw$}  {:<dw$}  {:<cw$}  {}",
            row.name,
            row.default,
            row.current,
            row.source,
            nw = name_w,
            dw = default_w,
            cw = current_w,
        );
    }
}

/// Result of a single doctor check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorFinding {
    pub ok: bool,
    pub name: &'static str,
    pub detail: String,
}

/// Run all doctor checks. Returns one finding per check (passing or failing).
///
/// Pure function over its inputs so it can be unit-tested without touching
/// the real environment.
pub fn run_doctor_checks(
    config: &Config,
    tmux_present: bool,
    clawgs_defaults: Result<String, String>,
    data_dir_writable: Result<PathBuf, String>,
) -> Vec<DoctorFinding> {
    let mut findings = Vec::new();

    // Check 1: trusted auth modes must stay on their matching trusted network.
    let bind_loopback = is_loopback_bind(&config.bind);
    let bind_tailnet = is_tailnet_bind(&config.bind);
    if matches!(config.auth_mode, AuthMode::LocalTrust) && !bind_loopback {
        findings.push(DoctorFinding {
            ok: false,
            name: "auth/bind",
            detail: format!(
                "SWIMMERS_BIND={} is non-loopback while AUTH_MODE=local_trust. \
                 This exposes the API to the network with no authentication. \
                 Bind to 127.0.0.1, use AUTH_MODE=tailnet_trust with a Tailscale bind address, \
                 or set AUTH_MODE=token AUTH_TOKEN=<secret>.",
                config.bind
            ),
        });
    } else if matches!(config.auth_mode, AuthMode::TailnetTrust) && !bind_tailnet {
        findings.push(DoctorFinding {
            ok: false,
            name: "auth/bind",
            detail: format!(
                "SWIMMERS_BIND={} is not a Tailscale address while AUTH_MODE=tailnet_trust. \
                 Bind to a Tailscale IP in 100.64.0.0/10 or fd7a:115c:a1e0::/48, \
                 or use AUTH_MODE=token AUTH_TOKEN=<secret> for non-tailnet exposure.",
                config.bind
            ),
        });
    } else {
        findings.push(DoctorFinding {
            ok: true,
            name: "auth/bind",
            detail: format!(
                "bind={} auth_mode={} (safe)",
                config.bind,
                match config.auth_mode {
                    AuthMode::LocalTrust => "local_trust",
                    AuthMode::TailnetTrust => "tailnet_trust",
                    AuthMode::Token => "token",
                }
            ),
        });
    }

    // Check 2: AUTH_MODE=token requires AUTH_TOKEN
    if matches!(config.auth_mode, AuthMode::Token) && config.auth_token.is_none() {
        findings.push(DoctorFinding {
            ok: false,
            name: "auth/token",
            detail: "AUTH_MODE=token but AUTH_TOKEN is not set. Set AUTH_TOKEN=<secret>."
                .to_string(),
        });
    } else {
        findings.push(DoctorFinding {
            ok: true,
            name: "auth/token",
            detail: match config.auth_mode {
                AuthMode::Token => "token configuration ok",
                AuthMode::TailnetTrust => "token not required in tailnet_trust mode",
                AuthMode::LocalTrust => "token not required in local_trust mode",
            }
            .to_string(),
        });
    }

    // Check 3: tmux on PATH
    if tmux_present {
        findings.push(DoctorFinding {
            ok: true,
            name: "tmux",
            detail: "tmux found on PATH".to_string(),
        });
    } else {
        findings.push(DoctorFinding {
            ok: false,
            name: "tmux",
            detail: "tmux not found on PATH. Install with: brew install tmux (macOS) \
                     or apt install tmux (Debian/Ubuntu)."
                .to_string(),
        });
    }

    // Check 4: clawgs defaults available for the thought rail
    match clawgs_defaults {
        Ok(detail) => findings.push(DoctorFinding {
            ok: true,
            name: "clawgs",
            detail,
        }),
        Err(reason) => findings.push(DoctorFinding {
            ok: false,
            name: "clawgs",
            detail: reason,
        }),
    }

    // Check 5: data dir creatable / writable
    match data_dir_writable {
        Ok(path) => findings.push(DoctorFinding {
            ok: true,
            name: "data_dir",
            detail: format!("writable: {}", path.display()),
        }),
        Err(reason) => findings.push(DoctorFinding {
            ok: false,
            name: "data_dir",
            detail: format!("data dir not writable: {reason}"),
        }),
    }

    findings
}

pub(crate) fn bind_host(bind: &str) -> &str {
    let bind = bind.trim();

    if let Some(rest) = bind.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            if tail.is_empty() || tail.starts_with(':') {
                return host;
            }
        }
    }

    if let Some((host, port)) = bind.rsplit_once(':') {
        if !host.is_empty()
            && !port.is_empty()
            && port.chars().all(|ch| ch.is_ascii_digit())
            && !host.contains(':')
        {
            return host;
        }
    }

    bind
}

/// Returns true when `bind` resolves to a loopback host literal.
pub fn is_loopback_bind(bind: &str) -> bool {
    let host = bind_host(bind);
    if host == "localhost" {
        return true; // Keep localhost as a DNS-free local-dev shorthand.
    }

    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Returns true when `bind` is an IP literal from Tailscale's tailnet ranges.
pub fn is_tailnet_bind(bind: &str) -> bool {
    let host = bind_host(bind);
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let octets = ip.octets();
            octets[0] == 100 && (64..=127).contains(&octets[1])
        }
        Ok(IpAddr::V6(ip)) => {
            let segments = ip.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
        Err(_) => false,
    }
}

/// Synchronously check whether `tmux` is on PATH.
pub fn tmux_on_path() -> bool {
    ProcessCommand::new("tmux")
        .arg("-V")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

pub fn list_tmux_session_names() -> Result<Vec<String>, String> {
    let output = tmux_command()
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .map_err(|err| format!("failed to run `tmux list-sessions`: {err}"))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect());
    }

    let stderr = compact_command_text(&output.stderr);
    if tmux_list_has_no_server(&stderr) {
        return Ok(Vec::new());
    }

    Err(format!("`tmux list-sessions` failed: {stderr}"))
}

pub fn next_numeric_tmux_name() -> Result<String, String> {
    let names = list_tmux_session_names()?;
    next_numeric_tmux_name_from_names(names.iter().map(String::as_str))
}

pub fn next_numeric_tmux_name_from_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<String, String> {
    let next = names
        .into_iter()
        .filter_map(numbered_tmux_name)
        .max()
        .map(|highest| {
            highest
                .checked_add(1)
                .ok_or_else(|| "numeric tmux session counter exhausted".to_string())
        })
        .transpose()?
        .unwrap_or(0);
    Ok(next.to_string())
}

pub fn create_numbered_tmux_session(cwd: Option<&std::path::Path>) -> Result<String, String> {
    const MAX_ATTEMPTS: usize = 64;

    for _ in 0..MAX_ATTEMPTS {
        let name = next_numeric_tmux_name()?;
        let mut command = tmux_command();
        command.args(["new-session", "-d", "-s", &name]);
        if let Some(cwd) = cwd {
            command.arg("-c").arg(cwd);
        }

        let output = command
            .output()
            .map_err(|err| format!("failed to run `tmux new-session`: {err}"))?;
        if output.status.success() {
            return Ok(name);
        }

        let stderr = compact_command_text(&output.stderr);
        if tmux_session_already_exists(&stderr) {
            continue;
        }
        return Err(format!("`tmux new-session` failed: {stderr}"));
    }

    Err(format!(
        "failed to allocate a numeric tmux session after {MAX_ATTEMPTS} attempts"
    ))
}

pub fn attach_tmux_session(tmux_name: &str) -> Result<i32, String> {
    let target = format!("={tmux_name}");
    let status = tmux_command()
        .args(["attach-session", "-t", &target])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| format!("failed to run `tmux attach-session`: {err}"))?;
    Ok(status.code().unwrap_or(1))
}

fn tmux_command() -> ProcessCommand {
    let mut command = ProcessCommand::new("tmux");
    command.env_remove("TMUX");
    command.env_remove("TMUX_PANE");
    command
}

fn numbered_tmux_name(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<u64>().ok()
}

fn tmux_list_has_no_server(stderr: &str) -> bool {
    stderr.contains("no server running") || stderr.contains("failed to connect to server")
}

fn tmux_session_already_exists(stderr: &str) -> bool {
    stderr.contains("duplicate session") || stderr.contains("session exists")
}

/// Verify that the resolved `clawgs` binary can provide daemon defaults.
///
/// The TUI thought rail depends on this command during startup. Doctor keeps
/// the same binary resolution path and bounds execution so a broken external
/// tool cannot hang configuration checks.
pub fn check_clawgs_defaults() -> Result<String, String> {
    check_clawgs_defaults_for_bin(&resolve_clawgs_bin(), CLAWGS_DEFAULTS_DOCTOR_TIMEOUT)
}

fn check_clawgs_defaults_for_bin(bin: &str, timeout: Duration) -> Result<String, String> {
    let mut child = ProcessCommand::new(bin)
        .arg("defaults")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| match err.kind() {
            ErrorKind::NotFound => format!(
                "clawgs not found at `{bin}`. Install clawgs or set CLAWGS_BIN=/path/to/clawgs; \
                 the thought rail will run in degraded mode until this works."
            ),
            _ => format!(
                "failed to run `{bin} defaults`: {err}. Set CLAWGS_BIN=/path/to/clawgs if needed."
            ),
        })?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`{bin} defaults` timed out after {}ms. Run it manually or set \
                     CLAWGS_BIN=/path/to/clawgs; the thought rail will run in degraded mode \
                     until this works.",
                    timeout.as_millis()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to inspect `{bin} defaults`: {err}"));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| format!("failed to collect `{bin} defaults` output: {err}"))?;

    if !output.status.success() {
        let stderr = compact_command_text(&output.stderr);
        let detail = if stderr.is_empty() {
            output.status.to_string()
        } else {
            format!("{}: {stderr}", output.status)
        };
        return Err(format!(
            "`{bin} defaults` failed ({detail}). Install or rebuild clawgs, or set \
             CLAWGS_BIN=/path/to/clawgs."
        ));
    }

    let defaults: DaemonDefaults = serde_json::from_slice(&output.stdout).map_err(|err| {
        format!("`{bin} defaults` returned invalid JSON: {err}. Rebuild clawgs or set CLAWGS_BIN.")
    })?;
    let backend = if defaults.backend.trim().is_empty() {
        "unknown"
    } else {
        defaults.backend.as_str()
    };
    Ok(format!(
        "`{bin} defaults` ok (backend={backend}, model={})",
        defaults.model
    ))
}

fn compact_command_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(240)
        .collect()
}

/// Try to create the resolved data dir and write a temp file in it.
pub fn check_data_dir_writable(path: &std::path::Path) -> Result<PathBuf, String> {
    if let Err(err) = std::fs::create_dir_all(path) {
        return Err(format!("create_dir_all({}) failed: {err}", path.display()));
    }
    let probe = path.join(".swimmers-doctor-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(path.to_path_buf())
        }
        Err(err) => Err(format!("write {} failed: {err}", probe.display())),
    }
}

/// Format and print doctor findings, returning the appropriate exit code.
pub fn print_doctor_findings(findings: &[DoctorFinding]) -> i32 {
    let failed = findings
        .iter()
        .filter(|finding| !print_doctor_finding(finding))
        .count();
    print_doctor_summary(failed)
}

fn print_doctor_finding(finding: &DoctorFinding) -> bool {
    let mark = if finding.ok { "ok " } else { "FAIL" };
    let line = format!("[{mark}] {}: {}", finding.name, finding.detail);
    if finding.ok {
        println!("{line}");
    } else {
        eprintln!("{line}");
    }
    finding.ok
}

fn print_doctor_summary(failed: usize) -> i32 {
    if failed == 0 {
        println!("\ndoctor: all checks passed");
        0
    } else {
        eprintln!("\ndoctor: {failed} check(s) failed");
        1
    }
}

/// Sysexits-style exit code for configuration errors.
///
/// Used by the server's startup gate when auth mode and bind address are an
/// unsafe pair. Matches `EX_CONFIG` from `sysexits.h` so systemd and monitoring
/// scripts can distinguish a config refusal from a generic crash.
pub const EXIT_CONFIG: i32 = 78;

/// Returns `Err(message)` if the active configuration would expose the API
/// outside the trusted network for the selected auth mode.
pub fn enforce_trust_bind_safety(config: &Config) -> Result<(), String> {
    if matches!(config.auth_mode, AuthMode::LocalTrust) && !is_loopback_bind(&config.bind) {
        return Err(format!(
            "refusing to start: SWIMMERS_BIND={} is non-loopback while AUTH_MODE=local_trust. \
             This would expose the API to the network with no authentication. \
             Bind to 127.0.0.1, use AUTH_MODE=tailnet_trust with a Tailscale bind address, \
             or set AUTH_MODE=token AUTH_TOKEN=<secret>.",
            config.bind
        ));
    }
    if matches!(config.auth_mode, AuthMode::TailnetTrust) && !is_tailnet_bind(&config.bind) {
        return Err(format!(
            "refusing to start: SWIMMERS_BIND={} is not a Tailscale address while \
             AUTH_MODE=tailnet_trust. Bind to a Tailscale IP in 100.64.0.0/10 or \
             fd7a:115c:a1e0::/48, or use AUTH_MODE=token AUTH_TOKEN=<secret>.",
            config.bind
        ));
    }
    Ok(())
}

/// Backward-compatible wrapper for callers that still use the old gate name.
pub fn enforce_localtrust_loopback(config: &Config) -> Result<(), String> {
    enforce_trust_bind_safety(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(bind: &str, mode: AuthMode, token: Option<&str>) -> Config {
        Config {
            bind: bind.to_string(),
            auth_mode: mode,
            auth_token: token.map(|s| s.to_string()),
            ..Config::default()
        }
    }

    #[test]
    fn loopback_strings_are_loopback() {
        assert!(is_loopback_bind("127.0.0.1"));
        assert!(is_loopback_bind("127.0.0.2"));
        assert!(is_loopback_bind("::1"));
        assert!(is_loopback_bind("localhost"));
        assert!(!is_loopback_bind("0.0.0.0"));
        assert!(!is_loopback_bind("192.168.1.1"));
        assert!(!is_loopback_bind("10.0.0.1"));
    }

    #[test]
    fn loopback_bind_parses_host_port_forms() {
        assert!(is_loopback_bind("127.0.0.1:3210"));
        assert!(is_loopback_bind("[::1]:3210"));
        assert!(!is_loopback_bind("0.0.0.0:3210"));
    }

    #[test]
    fn tailnet_bind_detects_tailscale_ranges() {
        assert!(is_tailnet_bind("100.64.0.1"));
        assert!(is_tailnet_bind("100.86.253.9:3210"));
        assert!(is_tailnet_bind("[fd7a:115c:a1e0::1]:3210"));
        assert!(!is_tailnet_bind("100.63.255.255"));
        assert!(!is_tailnet_bind("100.128.0.1"));
        assert!(!is_tailnet_bind("10.0.0.1"));
        assert!(!is_tailnet_bind("localhost"));
    }

    #[test]
    fn bind_host_passes_through_plain_hosts() {
        assert_eq!(bind_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(bind_host("0.0.0.0"), "0.0.0.0");
        assert_eq!(bind_host("localhost"), "localhost");
        assert_eq!(bind_host("example.internal"), "example.internal");
    }

    #[test]
    fn bind_host_strips_numeric_port_from_host_port_forms() {
        assert_eq!(bind_host("127.0.0.1:3210"), "127.0.0.1");
        assert_eq!(bind_host("0.0.0.0:8080"), "0.0.0.0");
        assert_eq!(bind_host("localhost:80"), "localhost");
    }

    #[test]
    fn bind_host_strips_brackets_from_ipv6_host_port_forms() {
        assert_eq!(bind_host("[::1]:3210"), "::1");
        assert_eq!(bind_host("[fe80::1]:8080"), "fe80::1");
    }

    #[test]
    fn bind_host_keeps_plain_ipv6_literal_intact() {
        // Bare `::1` has multiple `:` so the host:port path bails out and the
        // whole string is returned, letting `is_loopback_bind` parse it as IP.
        assert_eq!(bind_host("::1"), "::1");
        assert_eq!(bind_host("fe80::1"), "fe80::1");
    }

    #[test]
    fn bind_host_handles_bracketed_host_without_port() {
        assert_eq!(bind_host("[::1]"), "::1");
    }

    #[test]
    fn bind_host_rejects_malformed_bracketed_input() {
        // No closing `]` falls through to the rsplit path; trailing junk after
        // the bracket must not be silently dropped.
        assert_eq!(bind_host("[::1"), "[::1");
        assert_eq!(bind_host("[::1]extra"), "[::1]extra");
    }

    #[test]
    fn bind_host_rejects_non_numeric_or_empty_ports() {
        // Empty port, alphabetic port, or empty host all fall through to the
        // whole-string return so callers see the original (malformed) input.
        assert_eq!(bind_host("127.0.0.1:"), "127.0.0.1:");
        assert_eq!(bind_host("127.0.0.1:abc"), "127.0.0.1:abc");
        assert_eq!(bind_host(":3210"), ":3210");
    }

    #[test]
    fn bind_host_trims_surrounding_whitespace() {
        assert_eq!(bind_host("  127.0.0.1  "), "127.0.0.1");
        assert_eq!(bind_host("\t[::1]:3210\n"), "::1");
    }

    #[test]
    fn bind_host_handles_empty_string() {
        assert_eq!(bind_host(""), "");
        assert_eq!(bind_host("   "), "");
    }

    #[test]
    fn parse_serve_subcommand() {
        let cli = ServerCli::parse_from(["swimmers", "serve"]);
        assert_eq!(cli.command, Some(ServerCommand::Serve));
    }

    #[test]
    fn parse_bare_invocation_without_subcommand() {
        let cli = ServerCli::parse_from(["swimmers"]);
        assert_eq!(cli.command, None);
    }

    #[test]
    fn parse_config_subcommand_without_action() {
        let cli = ServerCli::parse_from(["swimmers", "config"]);
        assert_eq!(cli.command, Some(ServerCommand::Config { action: None }));
    }

    #[test]
    fn parse_tmux_next_name_subcommand() {
        let cli = ServerCli::parse_from(["swimmers", "tmux", "next-name"]);
        assert_eq!(
            cli.command,
            Some(ServerCommand::Tmux {
                action: TmuxAction::NextName
            })
        );
    }

    #[test]
    fn parse_tmux_new_subcommand_with_cwd() {
        let cli = ServerCli::parse_from(["swimmers", "tmux", "new", "--cwd", "/tmp/project"]);
        assert_eq!(
            cli.command,
            Some(ServerCommand::Tmux {
                action: TmuxAction::New {
                    cwd: Some(PathBuf::from("/tmp/project"))
                }
            })
        );
    }

    #[test]
    fn next_numeric_tmux_name_uses_highest_existing_number_plus_one() {
        let names = [
            "6",
            "8",
            "dac-cyclechef-wave-01",
            "swimmers-attention",
            "08",
        ];

        assert_eq!(
            next_numeric_tmux_name_from_names(names).expect("next numeric name"),
            "9"
        );
    }

    #[test]
    fn next_numeric_tmux_name_starts_at_zero_without_numbered_sessions() {
        let names = ["swimmers-attention", "dac-cyclechef-wave-01", "alpha"];

        assert_eq!(
            next_numeric_tmux_name_from_names(names).expect("next numeric name"),
            "0"
        );
    }

    #[test]
    fn localtrust_loopback_passes_gate() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        assert!(enforce_localtrust_loopback(&c).is_ok());
    }

    #[test]
    fn localtrust_non_loopback_refused() {
        let c = cfg("0.0.0.0", AuthMode::LocalTrust, None);
        let err = enforce_trust_bind_safety(&c).unwrap_err();
        assert!(err.contains("SWIMMERS_BIND=0.0.0.0"));
        assert!(err.contains("AUTH_MODE=tailnet_trust"));
    }

    #[test]
    fn token_mode_non_loopback_allowed() {
        let c = cfg("0.0.0.0", AuthMode::Token, Some("secret"));
        assert!(enforce_trust_bind_safety(&c).is_ok());
    }

    #[test]
    fn tailnet_trust_tailscale_bind_allowed() {
        let c = cfg("100.86.253.9", AuthMode::TailnetTrust, None);
        assert!(enforce_trust_bind_safety(&c).is_ok());
    }

    #[test]
    fn tailnet_trust_public_bind_refused() {
        let c = cfg("0.0.0.0", AuthMode::TailnetTrust, None);
        let err = enforce_trust_bind_safety(&c).unwrap_err();
        assert!(err.contains("AUTH_MODE=tailnet_trust"));
        assert!(err.contains("Tailscale"));
    }

    #[test]
    fn doctor_flags_localtrust_non_loopback() {
        let c = cfg("0.0.0.0", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        let auth_bind = findings.iter().find(|f| f.name == "auth/bind").unwrap();
        assert!(!auth_bind.ok);
        assert!(auth_bind.detail.contains("non-loopback"));
    }

    #[test]
    fn doctor_flags_token_mode_without_token() {
        let c = cfg("127.0.0.1", AuthMode::Token, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        let auth_token = findings.iter().find(|f| f.name == "auth/token").unwrap();
        assert!(!auth_token.ok);
        assert!(auth_token.detail.contains("AUTH_TOKEN"));
    }

    #[test]
    fn doctor_flags_tailnet_trust_without_tailnet_bind() {
        let c = cfg("0.0.0.0", AuthMode::TailnetTrust, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        let auth_bind = findings.iter().find(|f| f.name == "auth/bind").unwrap();
        assert!(!auth_bind.ok);
        assert!(auth_bind.detail.contains("tailnet_trust"));
    }

    #[test]
    fn doctor_all_pass_with_tailnet_trust_tailscale_bind() {
        let c = cfg("100.86.253.9", AuthMode::TailnetTrust, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        assert!(findings.iter().all(|f| f.ok));
    }

    #[test]
    fn doctor_flags_missing_tmux() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(
            &c,
            false,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        let tmux = findings.iter().find(|f| f.name == "tmux").unwrap();
        assert!(!tmux.ok);
        assert!(tmux.detail.contains("brew install tmux"));
    }

    #[test]
    fn doctor_flags_missing_clawgs() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Err("clawgs not found at `clawgs`".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        let clawgs = findings.iter().find(|f| f.name == "clawgs").unwrap();
        assert!(!clawgs.ok);
        assert!(clawgs.detail.contains("clawgs not found"));
    }

    #[test]
    fn doctor_flags_unwritable_data_dir() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Err("permission denied".to_string()),
        );
        let dd = findings.iter().find(|f| f.name == "data_dir").unwrap();
        assert!(!dd.ok);
        assert!(dd.detail.contains("permission denied"));
    }

    #[test]
    fn doctor_all_pass_with_safe_config() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(
            &c,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        );
        assert!(findings.iter().all(|f| f.ok));
    }

    #[test]
    fn env_table_includes_all_documented_vars() {
        let rows = env_var_rows();
        for name in ENV_VARS {
            assert!(rows.iter().any(|r| r.name == *name), "missing {name}");
        }
    }

    #[test]
    fn env_table_redacts_secrets() {
        // Set then unset around the test to avoid polluting other tests.
        std::env::set_var("AUTH_TOKEN", "supersecret");
        let rows = env_var_rows();
        let auth = rows.iter().find(|r| r.name == "AUTH_TOKEN").unwrap();
        assert_eq!(auth.current, "***");
        assert_ne!(auth.current, "supersecret");
        std::env::remove_var("AUTH_TOKEN");
    }

    fn fast_timeout() -> Duration {
        Duration::from_secs(2)
    }

    fn write_executable_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write script");
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path
    }

    #[test]
    fn check_clawgs_defaults_reports_not_found_for_missing_bin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("does-not-exist-clawgs");
        let err = check_clawgs_defaults_for_bin(missing.to_str().unwrap(), fast_timeout())
            .expect_err("missing bin must error");
        assert!(
            err.contains("clawgs not found"),
            "expected NotFound branch, got: {err}"
        );
    }

    #[test]
    fn check_clawgs_defaults_reports_failure_when_bin_exits_non_zero() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(tmp.path(), "fail-clawgs", "#!/bin/sh\nexit 7\n");
        let err = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
            .expect_err("non-zero exit must error");
        assert!(
            err.contains("failed"),
            "expected non-zero failure branch, got: {err}"
        );
    }

    #[test]
    fn check_clawgs_defaults_reports_invalid_json_for_garbage_stdout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            tmp.path(),
            "garbage-clawgs",
            "#!/bin/sh\nprintf 'not json\\n'\n",
        );
        let err = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
            .expect_err("invalid JSON must error");
        assert!(
            err.contains("invalid JSON"),
            "expected JSON parse branch, got: {err}"
        );
    }

    #[test]
    fn check_clawgs_defaults_reports_timeout_for_slow_bin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(tmp.path(), "slow-clawgs", "#!/bin/sh\nsleep 5\n");
        let err =
            check_clawgs_defaults_for_bin(script.to_str().unwrap(), Duration::from_millis(100))
                .expect_err("slow bin must time out");
        assert!(
            err.contains("timed out"),
            "expected timeout branch, got: {err}"
        );
    }

    #[test]
    fn check_clawgs_defaults_returns_summary_for_valid_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            tmp.path(),
            "ok-clawgs",
            "#!/bin/sh\nprintf '{\"model\":\"sonnet-4\",\"backend\":\"claude\",\
                \"agent_prompt\":\"a\",\"terminal_prompt\":\"t\"}\\n'\n",
        );
        let ok = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
            .expect("valid bin must succeed");
        assert!(
            ok.contains("backend=claude"),
            "summary missing backend: {ok}"
        );
        assert!(ok.contains("model=sonnet-4"), "summary missing model: {ok}");
    }

    #[test]
    fn check_clawgs_defaults_uses_unknown_for_blank_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_executable_script(
            tmp.path(),
            "blank-backend-clawgs",
            "#!/bin/sh\nprintf '{\"model\":\"m\",\"backend\":\"   \",\
                \"agent_prompt\":\"a\",\"terminal_prompt\":\"t\"}\\n'\n",
        );
        let ok = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
            .expect("valid bin must succeed");
        assert!(
            ok.contains("backend=unknown"),
            "blank backend should fall back to 'unknown': {ok}"
        );
    }
}
