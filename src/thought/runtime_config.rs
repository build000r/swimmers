use serde::{Deserialize, Serialize};

pub use clawgs::emit::protocol::ThoughtConfig;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonDefaults {
    pub model: String,
    pub agent_prompt: String,
    pub terminal_prompt: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawgs::emit::protocol::CADENCE_HOT_MIN_MS;

    #[test]
    fn default_config_is_valid() {
        let config = ThoughtConfig::default();
        assert!(config.validate().is_ok());
        assert!(config.model.is_empty());
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
}
