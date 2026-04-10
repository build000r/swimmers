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

use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use clap::{Parser, Subcommand};

use crate::config::{AuthMode, Config};

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
    "SWIMMERS_THOUGHT_BACKEND",
    "SWIMMERS_REPLAY_BUFFER_SIZE",
    "SWIMMERS_DATA_DIR",
    "SWIMMERS_TUI_URL",
];

/// Variables whose values must never be printed in plaintext.
const SECRET_VARS: &[&str] = &["AUTH_TOKEN", "OBSERVER_TOKEN"];

const ENV_VAR_HELP: &str = "ENVIRONMENT VARIABLES:
  PORT                         Server listen port (default: 3210)
  SWIMMERS_BIND                Server bind address (default: 127.0.0.1)
  AUTH_MODE                    'local_trust' or 'token' (default: local_trust)
  AUTH_TOKEN                   Bearer token when AUTH_MODE=token
  OBSERVER_TOKEN               Read-only bearer token (optional)
  SWIMMERS_NATIVE_APP          'iterm' or 'ghostty' (default: iterm)
  SWIMMERS_THOUGHT_BACKEND     'daemon' or 'inproc' (default: daemon)
  SWIMMERS_REPLAY_BUFFER_SIZE  Replay ring size in bytes (default: 524288)
  SWIMMERS_DATA_DIR            Override the data directory
  SWIMMERS_TUI_URL             API URL the TUI connects to

Run `swimmers config` to see resolved values.
Run `swimmers config doctor` to validate the active configuration.";

const TUI_ENV_HELP: &str = "ENVIRONMENT VARIABLES:
  SWIMMERS_TUI_URL  API URL to connect to (default: http://127.0.0.1:3210)
  AUTH_MODE         'local_trust' or 'token'
  AUTH_TOKEN        Bearer token when AUTH_MODE=token";

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

#[derive(Subcommand, Debug)]
pub enum ServerCommand {
    /// Show resolved configuration and run validation checks.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Run validation checks against the active environment.
    ///
    /// Exits 0 if all checks pass, 1 otherwise. Doctor is advisory — the
    /// server itself also enforces the LocalTrust loopback gate at startup.
    Doctor,
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
            AuthMode::Token => "token".to_string(),
        },
        "AUTH_TOKEN" => "(unset)".to_string(),
        "OBSERVER_TOKEN" => "(unset)".to_string(),
        "SWIMMERS_NATIVE_APP" => "iterm".to_string(),
        "SWIMMERS_THOUGHT_BACKEND" => "daemon".to_string(),
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
        "{:<nw$}  {:<dw$}  {:<cw$}  {}",
        "NAME",
        "DEFAULT",
        "CURRENT",
        "SOURCE",
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
    data_dir_writable: Result<PathBuf, String>,
) -> Vec<DoctorFinding> {
    let mut findings = Vec::new();

    // Check 1: LocalTrust + non-loopback bind
    let bind_loopback = is_loopback_bind(&config.bind);
    if matches!(config.auth_mode, AuthMode::LocalTrust) && !bind_loopback {
        findings.push(DoctorFinding {
            ok: false,
            name: "auth/bind",
            detail: format!(
                "SWIMMERS_BIND={} is non-loopback while AUTH_MODE=local_trust. \
                 This exposes the API to the network with no authentication. \
                 Set AUTH_MODE=token AUTH_TOKEN=<secret> or bind to 127.0.0.1.",
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
            detail: "token configuration ok".to_string(),
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

    // Check 4: data dir creatable / writable
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

/// Returns true when `bind` is one of the well-known loopback strings.
///
/// Conservative on purpose: it only matches the canonical strings the
/// server actually uses. `127.0.0.2` etc. would also be loopback in theory,
/// but treating those as non-loopback is a safe false-positive — the
/// LocalTrust gate will refuse to start, which is the correct failure mode.
pub fn is_loopback_bind(bind: &str) -> bool {
    matches!(bind, "127.0.0.1" | "::1" | "localhost")
}

/// Synchronously check whether `tmux` is on PATH.
pub fn tmux_on_path() -> bool {
    ProcessCommand::new("tmux")
        .arg("-V")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
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
    let mut failed = 0usize;
    for f in findings {
        let mark = if f.ok { "ok " } else { "FAIL" };
        let line = format!("[{mark}] {}: {}", f.name, f.detail);
        if f.ok {
            println!("{line}");
        } else {
            eprintln!("{line}");
            failed += 1;
        }
    }
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
/// Used by the server's startup gate when LocalTrust is paired with a
/// non-loopback bind. Matches `EX_CONFIG` from `sysexits.h` so systemd and
/// monitoring scripts can distinguish a config refusal from a generic crash.
pub const EXIT_CONFIG: i32 = 78;

/// Returns `Err(message)` if the active configuration would expose the API
/// to the network with no authentication. Used both by the server startup
/// gate (which exits 78) and by `config doctor` (which exits 1).
pub fn enforce_localtrust_loopback(config: &Config) -> Result<(), String> {
    if matches!(config.auth_mode, AuthMode::LocalTrust) && !is_loopback_bind(&config.bind) {
        Err(format!(
            "refusing to start: SWIMMERS_BIND={} is non-loopback while AUTH_MODE=local_trust. \
             This would expose the API to the network with no authentication. \
             Set AUTH_MODE=token AUTH_TOKEN=<secret>, or bind to 127.0.0.1.",
            config.bind
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(bind: &str, mode: AuthMode, token: Option<&str>) -> Config {
        let mut c = Config::default();
        c.bind = bind.to_string();
        c.auth_mode = mode;
        c.auth_token = token.map(|s| s.to_string());
        c
    }

    #[test]
    fn loopback_strings_are_loopback() {
        assert!(is_loopback_bind("127.0.0.1"));
        assert!(is_loopback_bind("::1"));
        assert!(is_loopback_bind("localhost"));
        assert!(!is_loopback_bind("0.0.0.0"));
        assert!(!is_loopback_bind("100.101.123.63"));
    }

    #[test]
    fn localtrust_loopback_passes_gate() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        assert!(enforce_localtrust_loopback(&c).is_ok());
    }

    #[test]
    fn localtrust_non_loopback_refused() {
        let c = cfg("0.0.0.0", AuthMode::LocalTrust, None);
        let err = enforce_localtrust_loopback(&c).unwrap_err();
        assert!(err.contains("SWIMMERS_BIND=0.0.0.0"));
        assert!(err.contains("AUTH_MODE=token"));
    }

    #[test]
    fn token_mode_non_loopback_allowed() {
        let c = cfg("0.0.0.0", AuthMode::Token, Some("secret"));
        assert!(enforce_localtrust_loopback(&c).is_ok());
    }

    #[test]
    fn doctor_flags_localtrust_non_loopback() {
        let c = cfg("0.0.0.0", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(&c, true, Ok(PathBuf::from("/tmp")));
        let auth_bind = findings.iter().find(|f| f.name == "auth/bind").unwrap();
        assert!(!auth_bind.ok);
        assert!(auth_bind.detail.contains("non-loopback"));
    }

    #[test]
    fn doctor_flags_token_mode_without_token() {
        let c = cfg("127.0.0.1", AuthMode::Token, None);
        let findings = run_doctor_checks(&c, true, Ok(PathBuf::from("/tmp")));
        let auth_token = findings.iter().find(|f| f.name == "auth/token").unwrap();
        assert!(!auth_token.ok);
        assert!(auth_token.detail.contains("AUTH_TOKEN"));
    }

    #[test]
    fn doctor_flags_missing_tmux() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(&c, false, Ok(PathBuf::from("/tmp")));
        let tmux = findings.iter().find(|f| f.name == "tmux").unwrap();
        assert!(!tmux.ok);
        assert!(tmux.detail.contains("brew install tmux"));
    }

    #[test]
    fn doctor_flags_unwritable_data_dir() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(&c, true, Err("permission denied".to_string()));
        let dd = findings.iter().find(|f| f.name == "data_dir").unwrap();
        assert!(!dd.ok);
        assert!(dd.detail.contains("permission denied"));
    }

    #[test]
    fn doctor_all_pass_with_safe_config() {
        let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
        let findings = run_doctor_checks(&c, true, Ok(PathBuf::from("/tmp")));
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
}
