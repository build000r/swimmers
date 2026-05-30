use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    LocalTrust,
    TailnetTrust,
    Token,
}

impl AuthMode {
    pub fn as_env_value(self) -> &'static str {
        match self {
            Self::LocalTrust => "local_trust",
            Self::TailnetTrust => "tailnet_trust",
            Self::Token => "token",
        }
    }

    fn from_env_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local_trust" | "local" => Some(Self::LocalTrust),
            "token" => Some(Self::Token),
            "tailnet_trust" | "tailnet" | "tailscale" => Some(Self::TailnetTrust),
            _ => None,
        }
    }
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
    pub fn as_env_value(self) -> &'static str {
        match self {
            Self::Inproc => "inproc",
            Self::Daemon => "daemon",
        }
    }

    fn from_env_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "inproc" => Some(Self::Inproc),
            "daemon" => Some(Self::Daemon),
            _ => None,
        }
    }
}

pub const MIN_PORT: u16 = 1;
pub const MAX_PORT: u16 = u16::MAX;
pub const MIN_THOUGHT_TICK_MS: u64 = 250;
pub const MAX_THOUGHT_TICK_MS: u64 = 300_000;
pub const MIN_OUTBOUND_QUEUE_BOUND: usize = 64;
pub const MAX_OUTBOUND_QUEUE_BOUND: usize = 65_536;
pub const MIN_REPLAY_BUFFER_SIZE: usize = 4 * 1024;
pub const MAX_REPLAY_BUFFER_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDiagnosticLevel {
    Warning,
    Error,
}

impl ConfigDiagnosticLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub level: ConfigDiagnosticLevel,
    pub key: &'static str,
    pub message: String,
}

impl ConfigDiagnostic {
    fn warning(key: &'static str, message: impl Into<String>) -> Self {
        Self {
            level: ConfigDiagnosticLevel::Warning,
            key,
            message: message.into(),
        }
    }

    fn error(key: &'static str, message: impl Into<String>) -> Self {
        Self {
            level: ConfigDiagnosticLevel::Error,
            key,
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.level == ConfigDiagnosticLevel::Error
    }
}

#[derive(Debug, Clone)]
pub struct ConfigLoad {
    pub config: Config,
    pub diagnostics: Vec<ConfigDiagnostic>,
}

impl ConfigLoad {
    fn new(config: Config) -> Self {
        Self {
            config,
            diagnostics: Vec::new(),
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(ConfigDiagnostic::is_error)
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub bind: String,
    pub auth_mode: AuthMode,
    pub auth_token: Option<String>,
    pub observer_token: Option<String>,
    pub thought_tick_ms: u64,
    #[allow(dead_code)]
    // FIXME(2026-04-21): Session-level thought defaults are not yet surfaced in API payloads.
    pub thoughts_enabled_default: bool,
    #[allow(dead_code)]
    // FIXME(2026-04-21): API delete flows still use a fixed mode and do not expose this field.
    pub session_delete_mode: SessionDeleteMode,
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
            bind: "127.0.0.1".to_string(),
            auth_mode: AuthMode::LocalTrust,
            auth_token: None,
            observer_token: None,
            thought_tick_ms: 15000,
            thoughts_enabled_default: true,
            session_delete_mode: SessionDeleteMode::DetachBridge,
            replay_buffer_size: 512 * 1024, // 512KB replay ring
            // NOTE: empirical default — sized well above the 600-frame burst floor verified in tests.
            outbound_queue_bound: 4096,
            thought_backend: ThoughtBackend::Daemon,
            overload_window_ms: 1000,
        }
    }
}

fn push_warning(load: &mut ConfigLoad, key: &'static str, message: impl Into<String>) {
    load.diagnostics
        .push(ConfigDiagnostic::warning(key, message));
}

fn push_error(load: &mut ConfigLoad, key: &'static str, message: impl Into<String>) {
    load.diagnostics.push(ConfigDiagnostic::error(key, message));
}

fn env_value(load: &mut ConfigLoad, key: &'static str) -> Option<String> {
    match std::env::var(key) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            push_warning(load, key, "value is not valid Unicode; using the default");
            None
        }
    }
}

fn apply_env_port(load: &mut ConfigLoad) {
    let Some(raw) = env_value(load, "PORT") else {
        return;
    };
    let trimmed = raw.trim();
    match trimmed.parse::<u16>() {
        Ok(port) if port >= MIN_PORT => {
            load.config.port = port;
        }
        Ok(_) => push_warning(
            load,
            "PORT",
            format!(
                "value {trimmed:?} is below the minimum {MIN_PORT}; using default {}",
                Config::default().port
            ),
        ),
        Err(_) => push_warning(
            load,
            "PORT",
            format!(
                "value {trimmed:?} is not a valid port in {MIN_PORT}-{MAX_PORT}; using default {}",
                Config::default().port
            ),
        ),
    }
}

fn apply_env_auth_mode(load: &mut ConfigLoad) {
    let Some(raw) = env_value(load, "AUTH_MODE") else {
        return;
    };
    if let Some(mode) = AuthMode::from_env_value(&raw) {
        load.config.auth_mode = mode;
        return;
    }
    push_error(
        load,
        "AUTH_MODE",
        format!(
            "unsupported value {:?}; use local_trust, tailnet_trust, or token",
            raw.trim()
        ),
    );
}

fn parse_env_non_empty_string(load: &mut ConfigLoad, key: &'static str) -> Option<String> {
    let Some(value) = env_value(load, key) else {
        return None;
    };
    if value.is_empty() {
        push_warning(load, key, "empty value ignored; using the default");
        None
    } else {
        Some(value)
    }
}

fn parse_env_usize_bounded(
    load: &mut ConfigLoad,
    key: &'static str,
    default: usize,
    min: usize,
    max: usize,
) -> Option<usize> {
    let raw = env_value(load, key)?;
    let trimmed = raw.trim();
    match trimmed.parse::<usize>() {
        Ok(value) if (min..=max).contains(&value) => Some(value),
        Ok(value) if value < min => {
            push_warning(
                load,
                key,
                format!("value {value} is below the minimum {min}; using default {default}"),
            );
            None
        }
        Ok(value) => {
            push_warning(
                load,
                key,
                format!("value {value} exceeds the maximum {max}; clamping to {max}"),
            );
            Some(max)
        }
        Err(_) => {
            push_warning(
                load,
                key,
                format!(
                    "value {trimmed:?} is not a valid positive integer; using default {default}"
                ),
            );
            None
        }
    }
}

fn parse_env_u64_bounded(
    load: &mut ConfigLoad,
    key: &'static str,
    default: u64,
    min: u64,
    max: u64,
) -> Option<u64> {
    let raw = env_value(load, key)?;
    let trimmed = raw.trim();
    match trimmed.parse::<u64>() {
        Ok(value) if (min..=max).contains(&value) => Some(value),
        Ok(value) if value < min => {
            push_warning(
                load,
                key,
                format!("value {value} is below the minimum {min}; using default {default}"),
            );
            None
        }
        Ok(value) => {
            push_warning(
                load,
                key,
                format!("value {value} exceeds the maximum {max}; clamping to {max}"),
            );
            Some(max)
        }
        Err(_) => {
            push_warning(
                load,
                key,
                format!(
                    "value {trimmed:?} is not a valid positive integer; using default {default}"
                ),
            );
            None
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_env_report().config
    }

    pub fn from_env_report() -> ConfigLoad {
        let defaults = Self::default();
        let mut load = ConfigLoad::new(defaults.clone());
        apply_env_port(&mut load);
        apply_env_auth_mode(&mut load);
        if let Some(token) = parse_env_non_empty_string(&mut load, "AUTH_TOKEN") {
            load.config.auth_token = Some(token);
        }
        if let Some(token) = parse_env_non_empty_string(&mut load, "OBSERVER_TOKEN") {
            load.config.observer_token = Some(token);
        }
        if let Some(addr) = parse_env_non_empty_string(&mut load, "SWIMMERS_BIND") {
            load.config.bind = addr;
        }

        if let Some(backend) = env_value(&mut load, "SWIMMERS_THOUGHT_BACKEND") {
            if let Some(parsed) = ThoughtBackend::from_env_value(&backend) {
                load.config.thought_backend = parsed;
            } else {
                push_warning(
                    &mut load,
                    "SWIMMERS_THOUGHT_BACKEND",
                    format!(
                        "unsupported value {:?}; using default {}",
                        backend.trim(),
                        defaults.thought_backend.as_env_value()
                    ),
                );
            }
        }

        if let Some(value) = parse_env_u64_bounded(
            &mut load,
            "SWIMMERS_THOUGHT_TICK_MS",
            defaults.thought_tick_ms,
            MIN_THOUGHT_TICK_MS,
            MAX_THOUGHT_TICK_MS,
        ) {
            load.config.thought_tick_ms = value;
        }
        if let Some(value) = parse_env_usize_bounded(
            &mut load,
            "SWIMMERS_OUTBOUND_QUEUE_BOUND",
            defaults.outbound_queue_bound,
            MIN_OUTBOUND_QUEUE_BOUND,
            MAX_OUTBOUND_QUEUE_BOUND,
        ) {
            load.config.outbound_queue_bound = value;
        }
        if let Some(value) = parse_env_usize_bounded(
            &mut load,
            "SWIMMERS_REPLAY_BUFFER_SIZE",
            defaults.replay_buffer_size,
            MIN_REPLAY_BUFFER_SIZE,
            MAX_REPLAY_BUFFER_SIZE,
        ) {
            load.config.replay_buffer_size = value;
        }

        if matches!(load.config.auth_mode, AuthMode::Token) && load.config.auth_token.is_none() {
            push_error(
                &mut load,
                "AUTH_TOKEN",
                "AUTH_MODE=token requires AUTH_TOKEN=<secret>; refusing token mode without an operator token",
            );
        }

        load
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    const CONFIG_ENV_KEYS: &[&str] = &[
        "PORT",
        "AUTH_MODE",
        "AUTH_TOKEN",
        "OBSERVER_TOKEN",
        "SWIMMERS_BIND",
        "SWIMMERS_THOUGHT_BACKEND",
        "SWIMMERS_THOUGHT_TICK_MS",
        "SWIMMERS_OUTBOUND_QUEUE_BOUND",
        "SWIMMERS_REPLAY_BUFFER_SIZE",
    ];

    fn clear_config_env() {
        for key in CONFIG_ENV_KEYS {
            std::env::remove_var(key);
        }
    }

    struct EnvSnapshot(Vec<(&'static str, Option<OsString>)>);

    impl EnvSnapshot {
        fn capture() -> Self {
            Self(
                CONFIG_ENV_KEYS
                    .iter()
                    .map(|key| (*key, std::env::var_os(key)))
                    .collect(),
            )
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn load_with_env(vars: &[(&str, &str)]) -> ConfigLoad {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _snapshot = EnvSnapshot::capture();
        clear_config_env();
        for (key, value) in vars {
            std::env::set_var(key, value);
        }
        let load = Config::from_env_report();
        load
    }

    fn diagnostic_for<'a>(load: &'a ConfigLoad, key: &str) -> &'a ConfigDiagnostic {
        load.diagnostics
            .iter()
            .find(|diagnostic| diagnostic.key == key)
            .expect("diagnostic for key")
    }

    #[test]
    fn unknown_backend_defaults_to_daemon_with_warning() {
        let load = load_with_env(&[("SWIMMERS_THOUGHT_BACKEND", "something-unrecognized")]);
        assert_eq!(load.config.thought_backend, ThoughtBackend::Daemon);
        assert_eq!(
            diagnostic_for(&load, "SWIMMERS_THOUGHT_BACKEND").level,
            ConfigDiagnosticLevel::Warning
        );
    }

    #[test]
    fn inproc_backend_stays_available_for_compatibility() {
        assert_eq!(
            ThoughtBackend::from_env_value("inproc"),
            Some(ThoughtBackend::Inproc)
        );
    }

    #[test]
    fn default_config_uses_daemon_backend() {
        assert_eq!(Config::default().thought_backend, ThoughtBackend::Daemon);
    }

    #[test]
    fn config_reads_thought_tick_from_env() {
        let load = load_with_env(&[("SWIMMERS_THOUGHT_TICK_MS", "2500")]);
        assert_eq!(load.config.thought_tick_ms, 2500);
        assert!(load.diagnostics.is_empty());
    }

    #[test]
    fn burst_of_600_frames_fits_in_default_outbound_queue() {
        let config = Config::default();
        // NOTE: empirical default — revisit if production bursts exceed 600 frames.
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
    fn auth_mode_token_parsing_is_case_insensitive() {
        for value in ["token", "Token", "TOKEN", "  token  "] {
            let load = load_with_env(&[("AUTH_MODE", value), ("AUTH_TOKEN", "secret")]);
            assert!(
                matches!(load.config.auth_mode, AuthMode::Token),
                "AUTH_MODE={value:?} should enable Token mode"
            );
            assert!(load.diagnostics.is_empty());
        }
        let load = load_with_env(&[("AUTH_MODE", "local_trust")]);
        assert!(matches!(load.config.auth_mode, AuthMode::LocalTrust));
        assert!(load.diagnostics.is_empty());
    }

    #[test]
    fn auth_mode_tailnet_trust_parsing_accepts_aliases() {
        for value in ["tailnet_trust", "Tailnet_Trust", "tailnet", "tailscale"] {
            let load = load_with_env(&[("AUTH_MODE", value)]);
            assert!(
                matches!(load.config.auth_mode, AuthMode::TailnetTrust),
                "AUTH_MODE={value:?} should enable TailnetTrust mode"
            );
            assert!(load.diagnostics.is_empty());
        }
    }

    #[test]
    fn unknown_auth_mode_is_a_config_error_not_silent_localtrust() {
        let load = load_with_env(&[("AUTH_MODE", "oauth")]);
        assert!(matches!(load.config.auth_mode, AuthMode::LocalTrust));
        assert!(load.has_errors());
        let diagnostic = diagnostic_for(&load, "AUTH_MODE");
        assert_eq!(diagnostic.level, ConfigDiagnosticLevel::Error);
        assert!(diagnostic.message.contains("unsupported value"));
    }

    #[test]
    fn token_mode_without_auth_token_is_a_config_error() {
        let load = load_with_env(&[("AUTH_MODE", "token")]);
        assert!(matches!(load.config.auth_mode, AuthMode::Token));
        assert!(load.has_errors());
        let diagnostic = diagnostic_for(&load, "AUTH_TOKEN");
        assert_eq!(diagnostic.level, ConfigDiagnosticLevel::Error);
        assert!(diagnostic.message.contains("requires AUTH_TOKEN"));
    }

    #[test]
    fn invalid_port_falls_back_with_warning() {
        let load = load_with_env(&[("PORT", "not-a-port")]);
        assert_eq!(load.config.port, Config::default().port);
        let diagnostic = diagnostic_for(&load, "PORT");
        assert_eq!(diagnostic.level, ConfigDiagnosticLevel::Warning);
        assert!(diagnostic.message.contains("using default"));
    }

    #[test]
    fn zero_port_falls_back_with_warning() {
        let load = load_with_env(&[("PORT", "0")]);
        assert_eq!(load.config.port, Config::default().port);
        let diagnostic = diagnostic_for(&load, "PORT");
        assert_eq!(diagnostic.level, ConfigDiagnosticLevel::Warning);
        assert!(diagnostic.message.contains("minimum"));
    }

    #[test]
    fn resource_caps_accept_valid_values() {
        let load = load_with_env(&[
            ("SWIMMERS_REPLAY_BUFFER_SIZE", "1048576"),
            ("SWIMMERS_OUTBOUND_QUEUE_BOUND", "1024"),
            ("SWIMMERS_THOUGHT_TICK_MS", "1000"),
        ]);
        assert_eq!(load.config.replay_buffer_size, 1_048_576);
        assert_eq!(load.config.outbound_queue_bound, 1024);
        assert_eq!(load.config.thought_tick_ms, 1000);
        assert!(load.diagnostics.is_empty());
    }

    #[test]
    fn zero_resource_caps_fall_back_with_warnings() {
        let load = load_with_env(&[
            ("SWIMMERS_REPLAY_BUFFER_SIZE", "0"),
            ("SWIMMERS_OUTBOUND_QUEUE_BOUND", "0"),
            ("SWIMMERS_THOUGHT_TICK_MS", "0"),
        ]);
        let defaults = Config::default();
        assert_eq!(load.config.replay_buffer_size, defaults.replay_buffer_size);
        assert_eq!(
            load.config.outbound_queue_bound,
            defaults.outbound_queue_bound
        );
        assert_eq!(load.config.thought_tick_ms, defaults.thought_tick_ms);
        assert_eq!(
            diagnostic_for(&load, "SWIMMERS_REPLAY_BUFFER_SIZE").level,
            ConfigDiagnosticLevel::Warning
        );
        assert_eq!(
            diagnostic_for(&load, "SWIMMERS_OUTBOUND_QUEUE_BOUND").level,
            ConfigDiagnosticLevel::Warning
        );
        assert_eq!(
            diagnostic_for(&load, "SWIMMERS_THOUGHT_TICK_MS").level,
            ConfigDiagnosticLevel::Warning
        );
    }

    #[test]
    fn invalid_resource_caps_fall_back_with_warnings() {
        let load = load_with_env(&[
            ("SWIMMERS_REPLAY_BUFFER_SIZE", "big"),
            ("SWIMMERS_OUTBOUND_QUEUE_BOUND", "wide"),
            ("SWIMMERS_THOUGHT_TICK_MS", "slow"),
        ]);
        let defaults = Config::default();
        assert_eq!(load.config.replay_buffer_size, defaults.replay_buffer_size);
        assert_eq!(
            load.config.outbound_queue_bound,
            defaults.outbound_queue_bound
        );
        assert_eq!(load.config.thought_tick_ms, defaults.thought_tick_ms);
        assert_eq!(load.diagnostics.len(), 3);
    }

    #[test]
    fn oversized_resource_caps_are_clamped_with_warnings() {
        let load = load_with_env(&[
            ("SWIMMERS_REPLAY_BUFFER_SIZE", "999999999999"),
            ("SWIMMERS_OUTBOUND_QUEUE_BOUND", "999999999999"),
            ("SWIMMERS_THOUGHT_TICK_MS", "999999999999"),
        ]);
        assert_eq!(load.config.replay_buffer_size, MAX_REPLAY_BUFFER_SIZE);
        assert_eq!(load.config.outbound_queue_bound, MAX_OUTBOUND_QUEUE_BOUND);
        assert_eq!(load.config.thought_tick_ms, MAX_THOUGHT_TICK_MS);
        assert_eq!(load.diagnostics.len(), 3);
        assert!(load
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.message.contains("clamping")));
    }
}
