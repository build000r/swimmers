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

use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use glob::glob;
use serde::Serialize;

use crate::config::{
    bool_env_value, AuthMode, Config, ConfigDiagnostic, ConfigDiagnosticLevel, ConfigLoad,
};
use crate::thought::emitter_client::resolve_clawgs_bin;
use crate::thought::runtime_config::DaemonDefaults;
use crate::types::{DependencyHealthSnapshot, DependencyHealthStatus};

/// Single source of truth for a documented environment variable.
///
/// The config table (names, secrets, defaults, and resolved current values) is
/// derived from `ENV_VAR_SPECS` so adding a variable is one entry rather than
/// several parallel edits. The `current` hook is `Some(..)` only for variables
/// whose resolved value lives on [`Config`]; the rest fall back to the raw env
/// value (or default) in [`env_var_rows_from_load`], matching prior behavior.
/// `ENV_VAR_HELP` below is kept as a hand-reviewed literal mirror of this list
/// so clap's `after_help` can stay a `&'static str`.
struct EnvVarSpec {
    name: &'static str,
    secret: bool,
    default: fn(&Config) -> String,
    current: Option<fn(&ConfigLoad) -> String>,
}

/// Documented environment variables exposed via `swimmers config`.
///
/// Order matches the README env-var table (and `ENV_VAR_HELP`) so the surfaces
/// cannot drift in presentation. Defaults are pulled from [`Config::default`]
/// (or other well-known defaults) so the table also cannot drift from runtime.
const ENV_VAR_SPECS: &[EnvVarSpec] = &[
    EnvVarSpec {
        name: "PORT",
        secret: false,
        default: |config| config.port.to_string(),
        current: Some(|load| load.config.port.to_string()),
    },
    EnvVarSpec {
        name: "SWIMMERS_BIND",
        secret: false,
        default: |config| config.bind.clone(),
        current: Some(|load| load.config.bind.clone()),
    },
    EnvVarSpec {
        name: "AUTH_MODE",
        secret: false,
        default: |config| config.auth_mode.as_env_value().to_string(),
        current: Some(|load| load.config.auth_mode.as_env_value().to_string()),
    },
    EnvVarSpec {
        name: "AUTH_TOKEN",
        secret: true,
        default: |_| "(unset)".to_string(),
        current: Some(|load| redacted_token(load.config.auth_token.as_ref())),
    },
    EnvVarSpec {
        name: "OBSERVER_TOKEN",
        secret: true,
        default: |_| "(unset)".to_string(),
        current: Some(|load| redacted_token(load.config.observer_token.as_ref())),
    },
    EnvVarSpec {
        name: "SWIMMERS_GROK_BIN",
        secret: false,
        default: |_| "grok".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_PERSONAL_WORKFLOWS",
        secret: false,
        default: |config| bool_env_value(config.personal_workflows_enabled).to_string(),
        current: Some(|load| bool_env_value(load.config.personal_workflows_enabled).to_string()),
    },
    EnvVarSpec {
        name: "SWIMMERS_NATIVE_APP",
        secret: false,
        default: |_| "iterm".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_GHOSTTY_MODE",
        secret: false,
        default: |_| "swap".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_NATIVE_SCRIPT_ROOT",
        secret: false,
        default: |_| "(bundled)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_ATTENTION_GROUP_SIZE",
        secret: false,
        default: |_| "6".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_ATTENTION_GROUP_LAYOUT",
        secret: false,
        default: |_| "tiled".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED",
        secret: false,
        default: |_| "(unset)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_THOUGHT_BACKEND",
        secret: false,
        default: |_| "daemon".to_string(),
        current: Some(|load| load.config.thought_backend.as_env_value().to_string()),
    },
    EnvVarSpec {
        name: "CLAWGS_BIN",
        secret: false,
        default: |_| "(auto)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_THOUGHT_TICK_MS",
        secret: false,
        default: |config| config.thought_tick_ms.to_string(),
        current: Some(|load| load.config.thought_tick_ms.to_string()),
    },
    EnvVarSpec {
        name: "SWIMMERS_OUTBOUND_QUEUE_BOUND",
        secret: false,
        default: |config| config.outbound_queue_bound.to_string(),
        current: Some(|load| load.config.outbound_queue_bound.to_string()),
    },
    EnvVarSpec {
        name: "SWIMMERS_REPLAY_BUFFER_SIZE",
        secret: false,
        default: |config| config.replay_buffer_size.to_string(),
        current: Some(|load| load.config.replay_buffer_size.to_string()),
    },
    EnvVarSpec {
        name: "SWIMMERS_DATA_DIR",
        secret: false,
        default: |_| "(platform data dir)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_TUI_URL",
        secret: false,
        default: |_| "(unset)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_TUI_REUSE_SERVER",
        secret: false,
        default: |_| "(unset)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_REPO_SEARCH_ROOTS",
        secret: false,
        default: |_| "~/repos:~/hard".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_REPO_SEARCH_MAX_DEPTH",
        secret: false,
        default: |_| "8".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_VOICE_MODEL",
        secret: false,
        default: |_| "(unset)".to_string(),
        current: None,
    },
    EnvVarSpec {
        name: "SWIMMERS_VOICE_LANGUAGE",
        secret: false,
        default: |_| "auto".to_string(),
        current: None,
    },
];

fn redacted_token(token: Option<&String>) -> String {
    token.map(|_| "***").unwrap_or("(unset)").to_string()
}

fn env_var_spec(name: &str) -> Option<&'static EnvVarSpec> {
    ENV_VAR_SPECS.iter().find(|spec| spec.name == name)
}

const ENV_VAR_HELP: &str = "ENVIRONMENT VARIABLES:
  PORT                         Server listen port (default: 3210)
  SWIMMERS_BIND                Server bind address (default: 127.0.0.1)
  AUTH_MODE                    'local_trust', 'tailnet_trust', or 'token' (default: local_trust)
  AUTH_TOKEN                   Bearer token when AUTH_MODE=token
  OBSERVER_TOKEN               Read-only bearer token (optional)
  SWIMMERS_GROK_BIN            Override the Grok executable (default: grok)
  SWIMMERS_PERSONAL_WORKFLOWS  '1' to expose local repo, skill, and commit-helper routes
  SWIMMERS_NATIVE_APP          'iterm' or 'ghostty' (default: iterm)
  SWIMMERS_GHOSTTY_MODE        'swap', 'add', or 'window' (default: swap)
  SWIMMERS_NATIVE_SCRIPT_ROOT  Override bundled native handoff script root
  SWIMMERS_ATTENTION_GROUP_SIZE
                               Number of panes in the attention group, 1-6 (default: 6)
  SWIMMERS_ATTENTION_GROUP_LAYOUT
                               tmux layout: tiled, even-horizontal, even-vertical, main-horizontal, main-vertical
  SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED
                               '1' to include non-numbered tmux sessions in attention groups
  SWIMMERS_THOUGHT_BACKEND     'daemon' or 'inproc' (default: daemon)
  CLAWGS_BIN                   Override path to the clawgs binary
  SWIMMERS_THOUGHT_TICK_MS     Thought polling tick in milliseconds, 250-300000 (default: 15000)
  SWIMMERS_OUTBOUND_QUEUE_BOUND
                               WebSocket outbound queue bound, 64-65536 (default: 4096)
  SWIMMERS_REPLAY_BUFFER_SIZE  Replay ring size in bytes (default: 524288)
  SWIMMERS_DATA_DIR            Override the data directory
  SWIMMERS_TUI_URL             API URL the TUI connects to
  SWIMMERS_TUI_REUSE_SERVER    '1' to keep an existing loopback backend
  SWIMMERS_REPO_SEARCH_ROOTS   Path-list roots for repo search (default: ~/repos:~/hard)
  SWIMMERS_REPO_SEARCH_MAX_DEPTH
                               Max repo-search depth (default: 8)
  SWIMMERS_VOICE_MODEL         Whisper model path for the voice feature
  SWIMMERS_VOICE_LANGUAGE      Voice language hint (default: auto)

Run `swimmers config` to see resolved values.
Run `swimmers config doctor` to validate the active configuration.";

const TUI_ENV_HELP: &str = "ENVIRONMENT VARIABLES:
  SWIMMERS_TUI_URL  API URL to connect to; unset uses embedded mode
  AUTH_MODE         'local_trust', 'tailnet_trust', or 'token'
  AUTH_TOKEN        Bearer token when AUTH_MODE=token
  CLAWGS_BIN        Override path to the clawgs binary in embedded mode
  SWIMMERS_VOICE_MODEL
                   Whisper model path for the voice feature
  SWIMMERS_VOICE_LANGUAGE
                   Voice language hint (default: auto)";

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

    /// Propose untrusted ssh_only overlay targets from SSH config.
    ///
    /// This command is dry-run only: it parses Host blocks, emits candidate
    /// overlay snippets, and never connects to remote hosts or writes files.
    #[command(name = "ssh-import")]
    SshImport {
        /// Required safety gate; without it the command refuses to run.
        #[arg(long)]
        dry_run: bool,

        /// SSH config file to inspect. Defaults to ~/.ssh/config.
        #[arg(long = "ssh-config")]
        ssh_config: Option<PathBuf>,
    },
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

fn has_diagnostic(
    diagnostics: &[ConfigDiagnostic],
    name: &str,
    level: ConfigDiagnosticLevel,
) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.key == name && diagnostic.level == level)
}

fn source_for(name: &str, diagnostics: &[ConfigDiagnostic]) -> &'static str {
    if std::env::var(name).is_err() {
        return "default";
    }
    if has_diagnostic(diagnostics, name, ConfigDiagnosticLevel::Error) {
        "env (error)"
    } else if has_diagnostic(diagnostics, name, ConfigDiagnosticLevel::Warning) {
        "env (warning)"
    } else {
        "env"
    }
}

fn current_for(name: &str, load: &ConfigLoad) -> Option<String> {
    let current = env_var_spec(name)?.current?;
    Some(current(load))
}

/// Build the `swimmers config` table from the current environment.
///
/// Defaults are pulled from [`Config::default`] where possible so that the
/// table cannot silently drift from runtime defaults. Secret variables are
/// rendered as `***` when present.
pub fn env_var_rows() -> Vec<EnvVarRow> {
    let load = Config::from_env_report();
    env_var_rows_from_load(&load)
}

pub fn env_var_rows_from_load(load: &ConfigLoad) -> Vec<EnvVarRow> {
    let defaults = Config::default();
    ENV_VAR_SPECS
        .iter()
        .map(|spec| {
            let name = spec.name;
            let default = default_for(name, &defaults);
            let current = current_for(name, load).unwrap_or_else(|| match std::env::var(name) {
                Ok(val) if !val.is_empty() => {
                    if spec.secret {
                        "***".to_string()
                    } else {
                        val
                    }
                }
                _ => default.clone(),
            });
            let source = source_for(name, &load.diagnostics);
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
    match env_var_spec(name) {
        Some(spec) => (spec.default)(config),
        None => "(unknown)".to_string(),
    }
}

/// Print the `swimmers config` table to stdout.
pub fn print_config_table() {
    let load = Config::from_env_report();
    print_config_table_for_load(&load);
    print_config_diagnostics(&load.diagnostics);
}

pub fn print_config_table_for_load(load: &ConfigLoad) {
    let rows = env_var_rows_from_load(load);
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

pub fn print_config_diagnostics(diagnostics: &[ConfigDiagnostic]) {
    for diagnostic in diagnostics {
        eprintln!(
            "config {}: {}: {}",
            diagnostic.level.as_str(),
            diagnostic.key,
            diagnostic.message
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SshImportReport {
    pub mode: &'static str,
    pub source_path: String,
    pub writes_files: bool,
    pub connects_to_hosts: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub proposals: Vec<SshImportProposal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SshImportProposal {
    pub id: String,
    pub label: String,
    pub kind: &'static str,
    pub trust: &'static str,
    pub source: String,
    pub attach_hint: String,
    pub bootstrap_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub overlay_snippet: String,
}

#[derive(Debug, Default)]
struct SshHostBlock {
    aliases: Vec<String>,
    host_name: Option<String>,
    user: Option<String>,
    source_label: String,
    line_number: usize,
}

#[derive(Debug, Default)]
struct SshImportParser {
    proposals: Vec<SshImportProposal>,
    current: SshHostBlock,
    inside_match_block: bool,
}

pub fn default_ssh_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".ssh").join("config"))
}

pub fn ssh_import_report_from_config(
    source_path: impl Into<String>,
    config: &str,
) -> SshImportReport {
    let mut parser = SshImportParser::default();
    parser.parse_config(
        "ssh_config",
        config,
        None,
        &mut HashSet::new(),
        &mut Vec::new(),
    );
    SshImportReport {
        mode: "dry_run",
        source_path: source_path.into(),
        writes_files: false,
        connects_to_hosts: false,
        warnings: Vec::new(),
        proposals: parser.proposals,
    }
}

pub fn ssh_import_proposals_from_config(config: &str) -> Vec<SshImportProposal> {
    ssh_import_report_from_config("ssh_config", config).proposals
}

pub fn ssh_import_report_from_path(path: &Path) -> Result<SshImportReport, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|err| format!("ssh-import failed to read {}: {err}", path.display()))?;
    let mut parser = SshImportParser::default();
    let mut visited = HashSet::new();
    let mut warnings = Vec::new();
    let source_path = path.display().to_string();
    parser.parse_file_contents(path, &contents, &mut visited, &mut warnings);
    Ok(SshImportReport {
        mode: "dry_run",
        source_path,
        writes_files: false,
        connects_to_hosts: false,
        warnings,
        proposals: parser.proposals,
    })
}

impl SshImportParser {
    fn parse_file_contents(
        &mut self,
        path: &Path,
        config: &str,
        visited: &mut HashSet<PathBuf>,
        warnings: &mut Vec<String>,
    ) {
        let identity = canonical_or_original(path);
        if !visited.insert(identity) {
            return;
        }
        let source_label = path.display().to_string();
        let base_dir = path.parent();
        self.parse_config(&source_label, config, base_dir, visited, warnings);
    }

    fn parse_config(
        &mut self,
        source_label: &str,
        config: &str,
        base_dir: Option<&Path>,
        visited: &mut HashSet<PathBuf>,
        warnings: &mut Vec<String>,
    ) {
        for (line_index, raw_line) in config.lines().enumerate() {
            let line_number = line_index + 1;
            let is_indented = raw_line
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_whitespace());
            let line = strip_ssh_config_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            let Some((keyword, arguments)) = split_ssh_config_directive(line) else {
                continue;
            };
            let mut parts = arguments.split_whitespace();
            if keyword.eq_ignore_ascii_case("Host") {
                if self.inside_match_block && is_indented {
                    continue;
                }
                self.inside_match_block = false;
                self.append_current_block();
                self.current.aliases = parts.map(ToOwned::to_owned).collect();
                self.current.source_label = source_label.to_string();
                self.current.line_number = line_number;
            } else if keyword.eq_ignore_ascii_case("Match") {
                self.append_current_block();
                self.inside_match_block = true;
            } else if self.inside_match_block {
                continue;
            } else if keyword.eq_ignore_ascii_case("Include") {
                self.append_current_block();
                if let Some(base_dir) = base_dir {
                    for include_path in ssh_include_paths(parts, Some(base_dir), warnings) {
                        match std::fs::read_to_string(&include_path) {
                            Ok(contents) => {
                                self.parse_file_contents(
                                    &include_path,
                                    &contents,
                                    visited,
                                    warnings,
                                );
                            }
                            Err(err) => warnings.push(format!(
                                "could not read included SSH config {}: {err}",
                                include_path.display()
                            )),
                        }
                    }
                }
            } else if keyword.eq_ignore_ascii_case("HostName") {
                self.current.host_name = parts.next().map(ToOwned::to_owned);
            } else if keyword.eq_ignore_ascii_case("User") {
                self.current.user = parts.next().map(ToOwned::to_owned);
            }
        }

        self.append_current_block();
    }

    fn append_current_block(&mut self) {
        append_ssh_import_block(&mut self.proposals, std::mem::take(&mut self.current));
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn strip_ssh_config_comment(line: &str) -> &str {
    line.split_once('#')
        .map(|(before, _)| before)
        .unwrap_or(line)
}

fn split_ssh_config_directive(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let first_ws = line.find(|ch: char| ch.is_ascii_whitespace());
    let first_eq = line.find('=');

    let (keyword, arguments) = match (first_ws, first_eq) {
        (Some(ws_index), Some(eq_index)) if eq_index < ws_index => {
            (&line[..eq_index], &line[eq_index + 1..])
        }
        (None, Some(eq_index)) => (&line[..eq_index], &line[eq_index + 1..]),
        (Some(ws_index), _) => {
            let rest = line[ws_index..].trim_start();
            let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
            (&line[..ws_index], rest)
        }
        (None, None) => (line, ""),
    };

    let keyword = keyword.trim();
    if keyword.is_empty() {
        None
    } else {
        Some((keyword, arguments.trim()))
    }
}

fn ssh_include_paths<'a>(
    patterns: impl Iterator<Item = &'a str>,
    base_dir: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for pattern in patterns {
        let resolved = resolve_ssh_include_pattern(pattern, base_dir);
        let Some(pattern_text) = resolved.to_str() else {
            warnings.push(format!(
                "could not expand non-utf8 SSH Include pattern {}",
                resolved.display()
            ));
            continue;
        };
        match glob(pattern_text) {
            Ok(matches) => {
                paths.extend(matches.filter_map(|entry| match entry {
                    Ok(path) => Some(path),
                    Err(err) => {
                        warnings.push(format!("could not expand SSH Include entry: {err}"));
                        None
                    }
                }));
            }
            Err(err) => warnings.push(format!(
                "invalid SSH Include pattern {}: {err}",
                resolved.display()
            )),
        }
    }
    paths.sort_by_key(|path| path.to_string_lossy().to_string());
    paths
}

fn resolve_ssh_include_pattern(pattern: &str, base_dir: Option<&Path>) -> PathBuf {
    if let Some(rest) = pattern.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(pattern);
    if path.is_absolute() {
        path
    } else {
        base_dir
            .map(|base| base.join(path))
            .unwrap_or_else(|| PathBuf::from(pattern))
    }
}

fn append_ssh_import_block(proposals: &mut Vec<SshImportProposal>, block: SshHostBlock) {
    let SshHostBlock {
        aliases,
        host_name,
        user,
        source_label,
        line_number,
    } = block;
    for alias in aliases
        .into_iter()
        .filter(|alias| is_importable_ssh_alias(alias))
    {
        proposals.push(ssh_import_proposal(
            alias,
            host_name.as_deref(),
            user.as_deref(),
            &source_label,
            line_number,
        ));
    }
}

fn ssh_import_proposal(
    alias: String,
    host_name: Option<&str>,
    user: Option<&str>,
    source_label: &str,
    line_number: usize,
) -> SshImportProposal {
    let label = ssh_import_label(&alias, host_name, user);
    SshImportProposal {
        attach_hint: format!("ssh {alias}"),
        bootstrap_hint: format!("ssh {alias} 'swimmers serve'"),
        overlay_snippet: ssh_import_overlay_snippet(&alias, &label),
        source: format!("{source_label}:{line_number}"),
        id: alias,
        label,
        kind: "ssh_only",
        trust: "untrusted",
        host_name: host_name.map(ToOwned::to_owned),
        user: user.map(ToOwned::to_owned),
    }
}

fn ssh_import_label(alias: &str, host_name: Option<&str>, user: Option<&str>) -> String {
    match (user, host_name) {
        (Some(user), Some(host)) => format!("{user}@{host}"),
        (_, Some(host)) => host.to_string(),
        _ => alias.to_string(),
    }
}

fn ssh_import_overlay_snippet(alias: &str, label: &str) -> String {
    let snippet = SshImportOverlaySnippet {
        dev_sanity: SshImportDevSanity {
            agent_launch: SshImportAgentLaunch {
                targets: vec![SshImportOverlayTarget {
                    id: alias,
                    label,
                    kind: "ssh_only",
                }],
            },
        },
    };
    serde_yaml::to_string(&snippet).unwrap_or_else(|_| {
        format!(
            "dev_sanity:\n  agent_launch:\n    targets:\n      - id: {alias}\n        label: {label}\n        kind: ssh_only\n"
        )
    })
}

#[derive(Serialize)]
struct SshImportOverlaySnippet<'a> {
    dev_sanity: SshImportDevSanity<'a>,
}

#[derive(Serialize)]
struct SshImportDevSanity<'a> {
    agent_launch: SshImportAgentLaunch<'a>,
}

#[derive(Serialize)]
struct SshImportAgentLaunch<'a> {
    targets: Vec<SshImportOverlayTarget<'a>>,
}

#[derive(Serialize)]
struct SshImportOverlayTarget<'a> {
    id: &'a str,
    label: &'a str,
    kind: &'a str,
}

fn is_importable_ssh_alias(alias: &str) -> bool {
    !alias.is_empty()
        && !alias.starts_with('!')
        && !alias.contains('*')
        && !alias.contains('?')
        && alias.bytes().all(|byte| {
            matches!(
                byte,
                b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'.'
                    | b'_'
                    | b'-'
                    | b'@'
                    | b':'
            )
        })
}

/// Severity of a single doctor finding.
///
/// Carried structurally so rendering does not have to sniff the detail string
/// for a `warning:` prefix. `Warn` findings are advisory: they still count as
/// passing (`ok`) but render with a distinct `WARN` marker on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorLevel {
    Ok,
    Warn,
    Fail,
}

/// Result of a single doctor check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorFinding {
    pub ok: bool,
    pub level: DoctorLevel,
    pub name: &'static str,
    pub detail: String,
}

fn doctor_ok(name: &'static str, detail: impl Into<String>) -> DoctorFinding {
    DoctorFinding {
        ok: true,
        level: DoctorLevel::Ok,
        name,
        detail: detail.into(),
    }
}

fn doctor_fail(name: &'static str, detail: impl Into<String>) -> DoctorFinding {
    DoctorFinding {
        ok: false,
        level: DoctorLevel::Fail,
        name,
        detail: detail.into(),
    }
}

fn doctor_warn(name: &'static str, detail: impl Into<String>) -> DoctorFinding {
    DoctorFinding {
        ok: true,
        level: DoctorLevel::Warn,
        name,
        detail: detail.into(),
    }
}

pub fn config_diagnostic_findings(diagnostics: &[ConfigDiagnostic]) -> Vec<DoctorFinding> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let level = if diagnostic.is_error() {
                DoctorLevel::Fail
            } else {
                DoctorLevel::Warn
            };
            DoctorFinding {
                ok: !diagnostic.is_error(),
                level,
                name: "config/env",
                detail: format!(
                    "{}: {}: {}",
                    diagnostic.level.as_str(),
                    diagnostic.key,
                    diagnostic.message
                ),
            }
        })
        .collect()
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
    vec![
        doctor_auth_bind_finding(config),
        doctor_auth_token_finding(config),
        doctor_tmux_finding(tmux_present),
        doctor_clawgs_finding(clawgs_defaults),
        doctor_data_dir_finding(data_dir_writable),
    ]
}

fn doctor_auth_bind_finding(config: &Config) -> DoctorFinding {
    let bind_loopback = is_loopback_bind(&config.bind);
    let bind_tailnet = is_tailnet_bind(&config.bind);
    if matches!(config.auth_mode, AuthMode::LocalTrust) && !bind_loopback {
        doctor_fail(
            "auth/bind",
            format!(
                "SWIMMERS_BIND={} is non-loopback while AUTH_MODE=local_trust. \
                 This exposes the API to the network with no authentication. \
                 Bind to 127.0.0.1, use AUTH_MODE=tailnet_trust with a Tailscale bind address, \
                 or set AUTH_MODE=token AUTH_TOKEN=<secret>.",
                config.bind
            ),
        )
    } else if matches!(config.auth_mode, AuthMode::TailnetTrust) && !bind_tailnet {
        doctor_fail(
            "auth/bind",
            format!(
                "SWIMMERS_BIND={} is not a Tailscale address while AUTH_MODE=tailnet_trust. \
                 Bind to a Tailscale IP in 100.64.0.0/10 or fd7a:115c:a1e0::/48, \
                 or use AUTH_MODE=token AUTH_TOKEN=<secret> for non-tailnet exposure.",
                config.bind
            ),
        )
    } else {
        doctor_ok(
            "auth/bind",
            format!(
                "bind={} auth_mode={} (safe)",
                config.bind,
                config.auth_mode.as_env_value()
            ),
        )
    }
}

fn doctor_auth_token_finding(config: &Config) -> DoctorFinding {
    if matches!(config.auth_mode, AuthMode::Token) && config.auth_token.is_none() {
        doctor_fail(
            "auth/token",
            "AUTH_MODE=token but AUTH_TOKEN is not set. Set AUTH_TOKEN=<secret>.",
        )
    } else {
        doctor_ok(
            "auth/token",
            match config.auth_mode {
                AuthMode::Token => "token configuration ok",
                AuthMode::TailnetTrust => "token not required in tailnet_trust mode",
                AuthMode::LocalTrust => "token not required in local_trust mode",
            },
        )
    }
}

fn doctor_tmux_finding(tmux_present: bool) -> DoctorFinding {
    if tmux_present {
        doctor_ok("tmux", "tmux found on PATH")
    } else {
        doctor_fail(
            "tmux",
            "tmux not found on PATH. Install with: brew install tmux (macOS) \
             or apt install tmux (Debian/Ubuntu).",
        )
    }
}

fn doctor_clawgs_finding(clawgs_defaults: Result<String, String>) -> DoctorFinding {
    match clawgs_defaults {
        Ok(detail) => doctor_ok("clawgs", detail),
        Err(reason) => doctor_fail("clawgs", reason),
    }
}

fn doctor_data_dir_finding(data_dir_writable: Result<PathBuf, String>) -> DoctorFinding {
    match data_dir_writable {
        Ok(path) => doctor_ok("data_dir", format!("writable: {}", path.display())),
        Err(reason) => doctor_fail("data_dir", format!("data dir not writable: {reason}")),
    }
}

pub fn doctor_remote_targets_finding(snapshot: &DependencyHealthSnapshot) -> DoctorFinding {
    let configured = snapshot
        .details
        .get("configured_targets")
        .map(String::as_str)
        .unwrap_or("unknown");
    let detail = snapshot
        .last_error
        .as_deref()
        .map(|error| format!(": {error}"))
        .unwrap_or_default();
    let summary = format!(
        "status={} configured={} swimmers_api={} ssh_only={} handoff={} attach_hint_missing={} auth_env_missing={} targets_without_path_mappings={}{}",
        dependency_status_label(snapshot.status),
        configured,
        snapshot
            .details
            .get("swimmers_api_targets")
            .map(String::as_str)
            .unwrap_or("0"),
        snapshot
            .details
            .get("ssh_only_targets")
            .map(String::as_str)
            .unwrap_or("0"),
        snapshot
            .details
            .get("handoff_targets")
            .map(String::as_str)
            .unwrap_or("0"),
        snapshot
            .details
            .get("attach_hint_missing")
            .map(String::as_str)
            .unwrap_or("0"),
        snapshot
            .details
            .get("auth_env_missing")
            .map(String::as_str)
            .unwrap_or("0"),
        snapshot
            .details
            .get("targets_without_path_mappings")
            .map(String::as_str)
            .unwrap_or("0"),
        detail
    );
    match snapshot.status {
        DependencyHealthStatus::Healthy | DependencyHealthStatus::NotConfigured => {
            doctor_ok("remote_targets", summary)
        }
        DependencyHealthStatus::Unknown
        | DependencyHealthStatus::Degraded
        | DependencyHealthStatus::Unavailable => doctor_warn("remote_targets", summary),
    }
}

fn dependency_status_label(status: DependencyHealthStatus) -> &'static str {
    match status {
        DependencyHealthStatus::Unknown => "unknown",
        DependencyHealthStatus::Healthy => "healthy",
        DependencyHealthStatus::Degraded => "degraded",
        DependencyHealthStatus::Unavailable => "unavailable",
        DependencyHealthStatus::NotConfigured => "not_configured",
    }
}

pub(crate) fn bind_host(bind: &str) -> &str {
    let bind = bind.trim();

    bracketed_bind_host(bind)
        .or_else(|| host_port_bind_host(bind))
        .unwrap_or(bind)
}

fn bracketed_bind_host(bind: &str) -> Option<&str> {
    let rest = bind.strip_prefix('[')?;
    let (host, tail) = rest.split_once(']')?;
    valid_bracketed_bind_tail(tail).then_some(host)
}

fn valid_bracketed_bind_tail(tail: &str) -> bool {
    tail.is_empty() || tail.starts_with(':')
}

fn host_port_bind_host(bind: &str) -> Option<&str> {
    let (host, port) = bind.rsplit_once(':')?;
    valid_host_port_bind(host, port).then_some(host)
}

fn valid_host_port_bind(host: &str, port: &str) -> bool {
    !host.is_empty()
        && !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && !host.contains(':')
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
    create_numbered_tmux_session_with(cwd, 64, next_numeric_tmux_name, run_tmux_new_session)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TmuxNewSessionError {
    AlreadyExists,
    Failed(String),
}

fn run_tmux_new_session(
    name: &str,
    cwd: Option<&std::path::Path>,
) -> Result<(), TmuxNewSessionError> {
    let mut command = tmux_command();
    command.args(["new-session", "-d", "-s", name]);
    if let Some(cwd) = cwd {
        command.arg("-c").arg(cwd);
    }

    let output = command.output().map_err(|err| {
        TmuxNewSessionError::Failed(format!("failed to run `tmux new-session`: {err}"))
    })?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = compact_command_text(&output.stderr);
    if tmux_session_already_exists(&stderr) {
        return Err(TmuxNewSessionError::AlreadyExists);
    }
    Err(TmuxNewSessionError::Failed(format!(
        "`tmux new-session` failed: {stderr}"
    )))
}

fn create_numbered_tmux_session_with(
    cwd: Option<&std::path::Path>,
    max_attempts: usize,
    mut next_name: impl FnMut() -> Result<String, String>,
    mut create_session: impl FnMut(&str, Option<&std::path::Path>) -> Result<(), TmuxNewSessionError>,
) -> Result<String, String> {
    for _ in 0..max_attempts {
        let name = next_name()?;
        match create_session(&name, cwd) {
            Ok(()) => return Ok(name),
            Err(TmuxNewSessionError::AlreadyExists) => continue,
            Err(TmuxNewSessionError::Failed(err)) => return Err(err),
        }
    }

    Err(format!(
        "failed to allocate a numeric tmux session after {max_attempts} attempts"
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
    is_exact_ascii_digits(value)
        .then(|| value.parse::<u64>().ok())
        .flatten()
}

fn is_exact_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
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
    let mut child = spawn_clawgs_defaults(bin)?;
    wait_for_clawgs_defaults(bin, &mut child, timeout)?;
    let output = collect_clawgs_defaults_output(bin, child)?;
    summarize_clawgs_defaults_output(bin, output)
}

fn spawn_clawgs_defaults(bin: &str) -> Result<Child, String> {
    ProcessCommand::new(bin)
        .arg("defaults")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format_clawgs_spawn_error(bin, err))
}

fn format_clawgs_spawn_error(bin: &str, err: std::io::Error) -> String {
    match err.kind() {
        ErrorKind::NotFound => format!(
            "clawgs not found at `{bin}`. Install clawgs or set CLAWGS_BIN=/path/to/clawgs; \
             the thought rail will run in degraded mode until this works."
        ),
        _ => {
            format!(
                "failed to run `{bin} defaults`: {err}. Set CLAWGS_BIN=/path/to/clawgs if needed."
            )
        }
    }
}

fn wait_for_clawgs_defaults(bin: &str, child: &mut Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let finished = match clawgs_defaults_finished(child) {
            Ok(finished) => finished,
            Err(err) => {
                stop_clawgs_defaults_child(child);
                return Err(format_clawgs_inspect_error(bin, err));
            }
        };
        if finished {
            return Ok(());
        }
        if Instant::now() >= deadline {
            stop_clawgs_defaults_child(child);
            return Err(format_clawgs_timeout_error(bin, timeout));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn clawgs_defaults_finished(child: &mut Child) -> Result<bool, std::io::Error> {
    child.try_wait().map(|status| status.is_some())
}

fn stop_clawgs_defaults_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn format_clawgs_timeout_error(bin: &str, timeout: Duration) -> String {
    format!(
        "`{bin} defaults` timed out after {}ms. Run it manually or set \
         CLAWGS_BIN=/path/to/clawgs; the thought rail will run in degraded mode \
         until this works.",
        timeout.as_millis()
    )
}

fn format_clawgs_inspect_error(bin: &str, err: std::io::Error) -> String {
    format!("failed to inspect `{bin} defaults`: {err}")
}

fn collect_clawgs_defaults_output(bin: &str, child: Child) -> Result<Output, String> {
    child
        .wait_with_output()
        .map_err(|err| format_clawgs_collect_error(bin, err))
}

fn format_clawgs_collect_error(bin: &str, err: std::io::Error) -> String {
    format!("failed to collect `{bin} defaults` output: {err}")
}

fn summarize_clawgs_defaults_output(bin: &str, output: Output) -> Result<String, String> {
    if !output.status.success() {
        return Err(format_clawgs_nonzero_error(bin, &output));
    }

    summarize_successful_clawgs_defaults(bin, &output.stdout)
}

fn format_clawgs_nonzero_error(bin: &str, output: &Output) -> String {
    let stderr = compact_command_text(&output.stderr);
    let detail = clawgs_exit_detail(&output.status, &stderr);
    format!(
        "`{bin} defaults` failed ({detail}). Install or rebuild clawgs, or set \
         CLAWGS_BIN=/path/to/clawgs."
    )
}

fn clawgs_exit_detail(status: &ExitStatus, stderr: &str) -> String {
    if stderr.is_empty() {
        status.to_string()
    } else {
        format!("{status}: {stderr}")
    }
}

fn summarize_successful_clawgs_defaults(bin: &str, stdout: &[u8]) -> Result<String, String> {
    let defaults: DaemonDefaults = serde_json::from_slice(stdout).map_err(|err| {
        format!("`{bin} defaults` returned invalid JSON: {err}. Rebuild clawgs or set CLAWGS_BIN.")
    })?;

    Ok(format_clawgs_defaults_success(bin, &defaults))
}

fn format_clawgs_defaults_success(bin: &str, defaults: &DaemonDefaults) -> String {
    let backend = clawgs_defaults_backend(defaults);
    format!(
        "`{bin} defaults` ok (backend={backend}, model={})",
        defaults.model
    )
}

fn clawgs_defaults_backend(defaults: &DaemonDefaults) -> &str {
    if defaults.backend.trim().is_empty() {
        "unknown"
    } else {
        defaults.backend.as_str()
    }
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
    create_data_dir(path)?;
    write_data_dir_probe(path)?;
    Ok(path.to_path_buf())
}

fn create_data_dir(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|err| format!("create_dir_all({}) failed: {err}", path.display()))
}

fn write_data_dir_probe(path: &std::path::Path) -> Result<(), String> {
    let probe = path.join(".swimmers-doctor-probe");
    std::fs::write(&probe, b"ok")
        .map_err(|err| format!("write {} failed: {err}", probe.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
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
    let line = format_doctor_finding(finding);
    match doctor_output_stream(finding.level) {
        DoctorOutputStream::Stderr => eprintln!("{line}"),
        DoctorOutputStream::Stdout => println!("{line}"),
    }
    finding.ok
}

fn format_doctor_finding(finding: &DoctorFinding) -> String {
    format!(
        "[{}] {}: {}",
        doctor_level_mark(finding.level),
        finding.name,
        finding.detail
    )
}

fn doctor_level_mark(level: DoctorLevel) -> &'static str {
    match level {
        DoctorLevel::Fail => "FAIL",
        DoctorLevel::Warn => "WARN",
        DoctorLevel::Ok => "ok ",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoctorOutputStream {
    Stdout,
    Stderr,
}

fn doctor_output_stream(level: DoctorLevel) -> DoctorOutputStream {
    match level {
        DoctorLevel::Fail | DoctorLevel::Warn => DoctorOutputStream::Stderr,
        DoctorLevel::Ok => DoctorOutputStream::Stdout,
    }
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
pub fn enforce_startup_config(
    config: &Config,
    diagnostics: &[ConfigDiagnostic],
) -> Result<(), String> {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| format!("{}: {}", diagnostic.key, diagnostic.message))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(format!(
            "refusing to start due to invalid configuration: {}",
            errors.join("; ")
        ));
    }
    enforce_trust_bind_safety(config)
}

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
mod tests;
