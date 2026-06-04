use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

pub(super) const TMUX_BIN_ENV: &str = "SWIMMERS_TMUX_BIN";
pub(super) const TMUX_BIN_FALLBACKS: &[&str] = &[
    "/opt/homebrew/bin/tmux",
    "/usr/local/bin/tmux",
    "/usr/bin/tmux",
    "/bin/tmux",
];

pub(super) fn resolve_tmux_binary() -> Result<PathBuf> {
    configured_tmux_binary()
        .map(PathBuf::from)
        .map(validate_configured_tmux_binary)
        .unwrap_or_else(resolve_unconfigured_tmux_binary)
}

fn validate_configured_tmux_binary(path: PathBuf) -> Result<PathBuf> {
    validate_tmux_binary(path)
        .with_context(|| format!("{TMUX_BIN_ENV} must point to an absolute tmux binary"))
}

fn resolve_unconfigured_tmux_binary() -> Result<PathBuf> {
    find_tmux_binary_without_override().ok_or_else(|| {
        anyhow!("unable to locate tmux; set {TMUX_BIN_ENV} to an absolute tmux binary path")
    })
}

fn configured_tmux_binary() -> Option<OsString> {
    std::env::var_os(TMUX_BIN_ENV)
}

fn find_tmux_binary_without_override() -> Option<PathBuf> {
    find_binary_in_path("tmux").or_else(find_tmux_binary_fallback)
}

fn find_tmux_binary_fallback() -> Option<PathBuf> {
    TMUX_BIN_FALLBACKS
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

pub(super) fn validate_tmux_binary(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(anyhow!("path is not absolute"));
    }
    if !path.is_file() {
        return Err(anyhow!("binary not found at {}", path.display()));
    }
    Ok(path)
}

fn find_binary_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_binary_in_path_os(binary, &path)
}

pub(super) fn find_binary_in_path_os(binary: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_absolute() && candidate.is_file())
}
