use serde::{Deserialize, Serialize};

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
            replay_buffer_size: 64 * 1024, // 64KB replay ring
            outbound_queue_bound: 512,
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
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
