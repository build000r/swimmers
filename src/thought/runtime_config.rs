use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

pub const CADENCE_HOT_MIN_MS: u64 = 5_000;
pub const CADENCE_HOT_MAX_MS: u64 = 300_000;
pub const CADENCE_WARM_MAX_MS: u64 = 600_000;
pub const CADENCE_COLD_MAX_MS: u64 = 1_800_000;
pub const MODEL_MAX_CHARS: usize = 200;
pub const PROMPT_MAX_CHARS: usize = 4_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default = "default_cadence_hot_ms")]
    pub cadence_hot_ms: u64,
    #[serde(default = "default_cadence_warm_ms")]
    pub cadence_warm_ms: u64,
    #[serde(default = "default_cadence_cold_ms")]
    pub cadence_cold_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_prompt: Option<String>,
}

impl Default for ThoughtConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            model: String::new(),
            backend: String::new(),
            cadence_hot_ms: default_cadence_hot_ms(),
            cadence_warm_ms: default_cadence_warm_ms(),
            cadence_cold_ms: default_cadence_cold_ms(),
            agent_prompt: None,
            terminal_prompt: None,
        }
    }
}

impl ThoughtConfig {
    pub fn normalize(&mut self) {
        self.model = self.model.trim().to_string();
        self.backend = self.backend.trim().to_string();
        self.agent_prompt = normalize_optional_prompt(self.agent_prompt.take());
        self.terminal_prompt = normalize_optional_prompt(self.terminal_prompt.take());
    }

    pub fn validate(&self) -> Result<(), ThoughtConfigValidationError> {
        if self.model.chars().count() > MODEL_MAX_CHARS {
            return Err(ThoughtConfigValidationError::new(
                "model",
                format!("must be <= {MODEL_MAX_CHARS} characters"),
            ));
        }

        if !self.backend.is_empty() {
            const VALID_BACKENDS: &[&str] = &[
                "openrouter", "codex", "codex_cli", "codex-cli",
                "claude", "claude_cli", "claude-cli",
            ];
            if !VALID_BACKENDS
                .iter()
                .any(|v| v.eq_ignore_ascii_case(&self.backend))
            {
                return Err(ThoughtConfigValidationError::new(
                    "backend",
                    format!(
                        "unrecognized backend {:?}; expected one of: openrouter, claude, codex",
                        self.backend
                    ),
                ));
            }
        }

        if !(CADENCE_HOT_MIN_MS..=CADENCE_HOT_MAX_MS).contains(&self.cadence_hot_ms) {
            return Err(ThoughtConfigValidationError::new(
                "cadence_hot_ms",
                format!(
                    "must be between {CADENCE_HOT_MIN_MS} and {CADENCE_HOT_MAX_MS} (inclusive)"
                ),
            ));
        }

        if self.cadence_warm_ms < self.cadence_hot_ms || self.cadence_warm_ms > CADENCE_WARM_MAX_MS
        {
            return Err(ThoughtConfigValidationError::new(
                "cadence_warm_ms",
                format!(
                    "must be between cadence_hot_ms ({}) and {} (inclusive)",
                    self.cadence_hot_ms, CADENCE_WARM_MAX_MS
                ),
            ));
        }

        if self.cadence_cold_ms < self.cadence_warm_ms || self.cadence_cold_ms > CADENCE_COLD_MAX_MS
        {
            return Err(ThoughtConfigValidationError::new(
                "cadence_cold_ms",
                format!(
                    "must be between cadence_warm_ms ({}) and {} (inclusive)",
                    self.cadence_warm_ms, CADENCE_COLD_MAX_MS
                ),
            ));
        }

        validate_optional_len("agent_prompt", self.agent_prompt.as_deref())?;
        validate_optional_len("terminal_prompt", self.terminal_prompt.as_deref())?;

        Ok(())
    }

    pub fn normalize_and_validate(mut self) -> Result<Self, ThoughtConfigValidationError> {
        self.normalize();
        self.validate()?;
        Ok(self)
    }

    pub fn model_override(&self) -> Option<&str> {
        let trimmed = self.model.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThoughtConfigValidationError {
    pub field: &'static str,
    pub message: String,
}

impl ThoughtConfigValidationError {
    fn new(field: &'static str, message: String) -> Self {
        Self { field, message }
    }
}

impl Display for ThoughtConfigValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.field, self.message)
    }
}

impl Error for ThoughtConfigValidationError {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonDefaults {
    pub model: String,
    #[serde(default)]
    pub backend: String,
    pub agent_prompt: String,
    pub terminal_prompt: String,
}

fn validate_optional_len(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), ThoughtConfigValidationError> {
    if value.map(|value| value.chars().count()).unwrap_or_default() > PROMPT_MAX_CHARS {
        return Err(ThoughtConfigValidationError::new(
            field,
            format!("must be <= {PROMPT_MAX_CHARS} characters"),
        ));
    }
    Ok(())
}

fn normalize_optional_prompt(value: Option<String>) -> Option<String> {
    match value {
        Some(value) if value.is_empty() => None,
        _ => value,
    }
}

const fn default_enabled() -> bool {
    true
}

const fn default_cadence_hot_ms() -> u64 {
    15_000
}

const fn default_cadence_warm_ms() -> u64 {
    45_000
}

const fn default_cadence_cold_ms() -> u64 {
    120_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = ThoughtConfig::default();
        assert!(config.validate().is_ok());
        assert!(config.model.is_empty());
        assert!(config.backend.is_empty());
    }

    #[test]
    fn normalize_converts_empty_prompts_to_none() {
        let mut config = ThoughtConfig {
            agent_prompt: Some(String::new()),
            terminal_prompt: Some(String::new()),
            ..ThoughtConfig::default()
        };

        config.normalize();
        assert!(config.agent_prompt.is_none());
        assert!(config.terminal_prompt.is_none());
    }

    #[test]
    fn hot_cadence_must_be_in_range() {
        let mut config = ThoughtConfig::default();
        config.cadence_hot_ms = CADENCE_HOT_MIN_MS - 1;

        let err = config
            .validate()
            .expect_err("hot cadence should be rejected");
        assert_eq!(err.field, "cadence_hot_ms");
    }

    #[test]
    fn warm_cadence_must_be_at_least_hot() {
        let mut config = ThoughtConfig::default();
        config.cadence_hot_ms = 10_000;
        config.cadence_warm_ms = 9_999;

        let err = config
            .validate()
            .expect_err("warm cadence below hot cadence should be rejected");
        assert_eq!(err.field, "cadence_warm_ms");
    }

    #[test]
    fn cold_cadence_must_be_at_least_warm() {
        let mut config = ThoughtConfig::default();
        config.cadence_warm_ms = 50_000;
        config.cadence_cold_ms = 49_000;

        let err = config
            .validate()
            .expect_err("cold cadence below warm cadence should be rejected");
        assert_eq!(err.field, "cadence_cold_ms");
    }

    #[test]
    fn backend_validation_accepts_known_values() {
        for backend in ["openrouter", "claude", "codex", ""] {
            let config = ThoughtConfig {
                backend: backend.to_string(),
                ..ThoughtConfig::default()
            };
            assert!(
                config.validate().is_ok(),
                "backend {:?} should be valid",
                backend
            );
        }
    }

    #[test]
    fn backend_validation_rejects_unknown_values() {
        let config = ThoughtConfig {
            backend: "gemini".to_string(),
            ..ThoughtConfig::default()
        };
        let err = config
            .validate()
            .expect_err("unknown backend should be rejected");
        assert_eq!(err.field, "backend");
        assert!(err.message.contains("gemini"));
    }
}
