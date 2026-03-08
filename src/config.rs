use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const THRONGTERM_TMUX_EMIT_SOCKET_ENV: &str = "THRONGTERM_TMUX_EMIT_SOCKET";
pub const DEFAULT_TMUX_SOCKET_DIR: &str = ".tmux";
pub const DEFAULT_TMUX_SOCKET_FILE: &str = "clawgs-tmux.sock";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    LocalTrust,
    Token,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionDeleteMode {
    DetachBridge,
    KillTmux,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtBackend {
    Inproc,
    Daemon,
}

impl ThoughtBackend {
    fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "daemon" => Self::Daemon,
            "inproc" => Self::Inproc,
            _ => Self::Daemon,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub auth_mode: AuthMode,
    pub auth_token: Option<String>,
    pub observer_token: Option<String>,
    pub thought_tick_ms: u64,
    pub thoughts_enabled_default: bool,
    pub terminal_cache_ttl_ms: u64,
    pub session_delete_mode: SessionDeleteMode,
    pub poll_fallback_ms: u64,
    pub replay_buffer_size: usize,
    pub outbound_queue_bound: usize,
    pub thought_backend: ThoughtBackend,
    #[allow(dead_code)]
    pub overload_window_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 3210,
            auth_mode: AuthMode::LocalTrust,
            auth_token: None,
            observer_token: None,
            thought_tick_ms: 15000,
            thoughts_enabled_default: true,
            terminal_cache_ttl_ms: 300_000,
            session_delete_mode: SessionDeleteMode::DetachBridge,
            poll_fallback_ms: 2000,
            replay_buffer_size: 512 * 1024, // 512KB replay ring
            outbound_queue_bound: 4096,
            thought_backend: ThoughtBackend::Daemon,
            overload_window_ms: 1000,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(port) = std::env::var("PORT") {
            if let Ok(p) = port.parse() {
                config.port = p;
            }
        }
        if let Ok(mode) = std::env::var("AUTH_MODE") {
            if mode == "token" {
                config.auth_mode = AuthMode::Token;
            }
        }
        if let Ok(token) = std::env::var("AUTH_TOKEN") {
            if !token.is_empty() {
                config.auth_token = Some(token);
            }
        }
        if let Ok(token) = std::env::var("OBSERVER_TOKEN") {
            if !token.is_empty() {
                config.observer_token = Some(token);
            }
        }
        if let Ok(backend) = std::env::var("THRONGTERM_THOUGHT_BACKEND") {
            config.thought_backend = ThoughtBackend::from_env_value(&backend);
        }
        if let Ok(val) = std::env::var("THRONGTERM_OUTBOUND_QUEUE_BOUND") {
            if let Ok(n) = val.parse::<usize>() {
                if n > 0 {
                    config.outbound_queue_bound = n;
                }
            }
        }
        if let Ok(val) = std::env::var("THRONGTERM_REPLAY_BUFFER_SIZE") {
            if let Ok(n) = val.parse::<usize>() {
                if n > 0 {
                    config.replay_buffer_size = n;
                }
            }
        }
        config
    }
}

pub fn resolve_tmux_emit_socket() -> PathBuf {
    resolve_tmux_emit_socket_from(
        std::env::var(THRONGTERM_TMUX_EMIT_SOCKET_ENV)
            .ok()
            .as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

fn resolve_tmux_emit_socket_from(socket_override: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(socket_override) = non_empty_trimmed(socket_override) {
        return PathBuf::from(socket_override);
    }

    default_tmux_emit_socket_for_home(home)
}

fn default_tmux_emit_socket_for_home(home: Option<&str>) -> PathBuf {
    match non_empty_trimmed(home) {
        Some(home) => PathBuf::from(home)
            .join(DEFAULT_TMUX_SOCKET_DIR)
            .join(DEFAULT_TMUX_SOCKET_FILE),
        None => PathBuf::from(DEFAULT_TMUX_SOCKET_DIR).join(DEFAULT_TMUX_SOCKET_FILE),
    }
}

fn non_empty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TMUX_HOOK_EVENTS: [&str; 9] = [
        "session-created",
        "session-closed",
        "pane-exited",
        "pane-died",
        "after-new-window",
        "after-split-window",
        "client-session-changed",
        "window-linked",
        "window-unlinked",
    ];

    #[test]
    fn unknown_backend_defaults_to_daemon() {
        assert_eq!(
            ThoughtBackend::from_env_value("something-unrecognized"),
            ThoughtBackend::Daemon
        );
    }

    #[test]
    fn inproc_backend_stays_available_for_compatibility() {
        assert_eq!(
            ThoughtBackend::from_env_value("inproc"),
            ThoughtBackend::Inproc
        );
    }

    #[test]
    fn default_config_uses_daemon_backend() {
        assert_eq!(Config::default().thought_backend, ThoughtBackend::Daemon);
    }

    #[test]
    fn burst_of_600_frames_fits_in_default_outbound_queue() {
        let config = Config::default();
        // A burst of 600 frames (e.g. rapid AI agent output) must fit within
        // the outbound queue bound without causing subscriber eviction.
        assert!(
            config.outbound_queue_bound >= 600,
            "outbound_queue_bound ({}) must be >= 600 to tolerate high-throughput bursts",
            config.outbound_queue_bound,
        );
    }

    #[test]
    fn default_replay_buffer_is_512kb() {
        let config = Config::default();
        assert_eq!(config.replay_buffer_size, 512 * 1024);
    }

    #[test]
    fn default_tmux_emit_socket_uses_home_directory_contract() {
        assert_eq!(
            resolve_tmux_emit_socket_from(None, Some("/tmp/throng-home")),
            PathBuf::from("/tmp/throng-home/.tmux/clawgs-tmux.sock")
        );
    }

    #[test]
    fn blank_tmux_emit_socket_override_falls_back_to_home_default() {
        assert_eq!(
            resolve_tmux_emit_socket_from(Some("   "), Some("/tmp/throng-home")),
            PathBuf::from("/tmp/throng-home/.tmux/clawgs-tmux.sock")
        );
    }

    #[test]
    fn explicit_tmux_emit_socket_override_is_used_verbatim() {
        assert_eq!(
            resolve_tmux_emit_socket_from(
                Some("/tmp/custom/clawgs.sock"),
                Some("/tmp/throng-home")
            ),
            PathBuf::from("/tmp/custom/clawgs.sock")
        );
    }

    #[test]
    fn tmux_hook_snippet_matches_default_socket_contract() {
        let home = "/tmp/throng-home";
        let expected_socket = resolve_tmux_emit_socket_from(None, Some(home));
        let snippet = include_str!("../tmux/throngterm-clawgs-hooks.conf");

        assert!(
            !snippet
                .lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .any(|line| line.contains("clawgs tmux-emit")),
            "throngterm owns the tmux-emit daemon; hook snippet must stay notify-only"
        );

        let notify_lines: Vec<&str> = snippet
            .lines()
            .filter(|line| line.contains("clawgs tmux-notify"))
            .collect();
        assert_eq!(notify_lines.len(), TMUX_HOOK_EVENTS.len());

        for event in TMUX_HOOK_EVENTS {
            let line = notify_lines
                .iter()
                .find(|line| line.contains(&format!("--event {event}")))
                .unwrap_or_else(|| panic!("missing tmux hook for `{event}`"));
            let rendered = line.replace("$HOME", home);
            assert!(
                rendered.contains(&format!("--socket {}", expected_socket.display())),
                "hook line for `{event}` must target {}: {rendered}",
                expected_socket.display()
            );
        }
    }
}
