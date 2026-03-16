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
            replay_buffer_size: 512 * 1024, // 512KB replay ring
            outbound_queue_bound: 4096,
            thought_backend: ThoughtBackend::Daemon,
            overload_window_ms: 1000,
        }
    }
}

fn apply_env_port(config: &mut Config) {
    if let Ok(port) = std::env::var("PORT").and_then(|port| {
        port.parse()
            .map_err(|_| std::env::VarError::NotPresent)
    }) {
        config.port = port;
    }
}

fn apply_env_auth_mode(config: &mut Config) {
    if std::env::var("AUTH_MODE").as_deref() == Ok("token") {
        config.auth_mode = AuthMode::Token;
    }
}

fn apply_env_non_empty_string<F>(key: &str, apply: F)
where
    F: FnOnce(String),
{
    if let Some(value) = std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
    {
        apply(value);
    }
}

fn apply_env_usize<F>(key: &str, apply: F)
where
    F: FnOnce(usize),
{
    if let Some(value) = std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
    {
        apply(value);
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        apply_env_port(&mut config);
        apply_env_auth_mode(&mut config);
        apply_env_non_empty_string("AUTH_TOKEN", |token| config.auth_token = Some(token));
        apply_env_non_empty_string("OBSERVER_TOKEN", |token| {
            config.observer_token = Some(token);
        });
        apply_env_non_empty_string("THRONGTERM_THOUGHT_BACKEND", |backend| {
            config.thought_backend = ThoughtBackend::from_env_value(&backend);
        });
        apply_env_usize("THRONGTERM_OUTBOUND_QUEUE_BOUND", |value| {
            config.outbound_queue_bound = value;
        });
        apply_env_usize("THRONGTERM_REPLAY_BUFFER_SIZE", |value| {
            config.replay_buffer_size = value;
        });
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
}
