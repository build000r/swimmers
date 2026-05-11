// Thought-config UI helpers shared by daemon API and swimmers-tui.
// NOTE(2026-04-21): Keep this module focused on presentation-layer metadata and presets.

use crate::openrouter_models::cached_or_default_openrouter_candidates;
use crate::thought::runtime_config::DEFAULT_CODEX_MODEL;
use crate::types::{ThoughtConfigBackendMetadata, ThoughtConfigUiMetadata};

const THOUGHT_BACKEND_OPTIONS: [&str; 3] = ["", "openrouter", "codex"];

pub fn thought_backend_cycle_options() -> &'static [&'static str] {
    &THOUGHT_BACKEND_OPTIONS
}

pub fn canonical_thought_backend_key(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => "",
        "claude" | "claude-cli" | "claude_cli" | "openrouter" => "openrouter",
        "codex" | "codex-cli" | "codex_cli" => "codex",
        _ => "custom",
    }
}

pub fn thought_backend_label(value: &str) -> &'static str {
    match canonical_thought_backend_key(value) {
        "" => "auto",
        "openrouter" => "openrouter",
        "codex" => "codex",
        _ => "custom",
    }
}

pub fn thought_model_presets_hint(value: &str) -> &'static str {
    match canonical_thought_backend_key(value) {
        "openrouter" => "presets: auto  router  cached free models",
        "codex" => "preset: gpt-5.4-mini",
        _ => "auto backend uses daemon default model",
    }
}

pub fn thought_model_presets(value: &str, openrouter_model_presets: &[String]) -> Vec<String> {
    match canonical_thought_backend_key(value) {
        "openrouter" => {
            let mut values = vec![String::new()];
            if openrouter_model_presets.is_empty() {
                values.extend(cached_or_default_openrouter_candidates());
            } else {
                values.extend(openrouter_model_presets.iter().cloned());
            }
            values
        }
        "codex" => vec![DEFAULT_CODEX_MODEL.to_string()],
        _ => vec![String::new()],
    }
}

pub fn normalize_thought_model_for_backend(backend: &str, model: &str) -> String {
    let trimmed = model.trim();
    if canonical_thought_backend_key(backend) == "codex" && trimmed != DEFAULT_CODEX_MODEL {
        return DEFAULT_CODEX_MODEL.to_string();
    }
    if trimmed.is_empty() {
        return String::new();
    }

    let keep = match canonical_thought_backend_key(backend) {
        "" => false,
        "openrouter" => trimmed.contains('/'),
        "codex" => trimmed == DEFAULT_CODEX_MODEL,
        _ => true,
    };

    if keep {
        trimmed.to_string()
    } else {
        String::new()
    }
}

pub fn thought_config_ui_metadata(openrouter_model_presets: &[String]) -> ThoughtConfigUiMetadata {
    let openrouter_model_presets = if openrouter_model_presets.is_empty() {
        cached_or_default_openrouter_candidates()
    } else {
        openrouter_model_presets.to_vec()
    };

    ThoughtConfigUiMetadata {
        backends: vec![
            ThoughtConfigBackendMetadata {
                key: String::new(),
                label: "auto".to_string(),
                model_presets_hint: thought_model_presets_hint("").to_string(),
                model_presets: vec![String::new()],
            },
            ThoughtConfigBackendMetadata {
                key: "openrouter".to_string(),
                label: "openrouter".to_string(),
                model_presets_hint: thought_model_presets_hint("openrouter").to_string(),
                model_presets: thought_model_presets("openrouter", &openrouter_model_presets),
            },
            ThoughtConfigBackendMetadata {
                key: "codex".to_string(),
                label: "codex".to_string(),
                model_presets_hint: thought_model_presets_hint("codex").to_string(),
                model_presets: thought_model_presets("codex", &[]),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_backend_key_maps_aliases() {
        assert_eq!(canonical_thought_backend_key(""), "");
        assert_eq!(canonical_thought_backend_key("claude-cli"), "openrouter");
        assert_eq!(canonical_thought_backend_key("codex_cli"), "codex");
        assert_eq!(canonical_thought_backend_key("openrouter"), "openrouter");
        assert_eq!(canonical_thought_backend_key("custom-backend"), "custom");
    }

    #[test]
    fn normalize_model_clears_incompatible_values() {
        assert!(normalize_thought_model_for_backend("claude", "haiku").is_empty());
        assert_eq!(
            normalize_thought_model_for_backend("codex", "gpt-5.4-mini"),
            "gpt-5.4-mini"
        );
        assert_eq!(
            normalize_thought_model_for_backend("codex", "gpt-5.1-codex-mini"),
            "gpt-5.4-mini"
        );
        assert!(normalize_thought_model_for_backend("", "haiku").is_empty());
    }

    #[test]
    fn codex_presets_only_offer_current_low_cost_model() {
        assert_eq!(thought_model_presets("codex", &[]), vec!["gpt-5.4-mini"]);
        assert_eq!(thought_model_presets_hint("codex"), "preset: gpt-5.4-mini");
    }

    #[test]
    fn ui_metadata_includes_openrouter_presets() {
        let metadata = thought_config_ui_metadata(&["openrouter/free".to_string()]);
        let openrouter = metadata
            .backends
            .iter()
            .find(|backend| backend.key == "openrouter")
            .expect("openrouter backend metadata");

        assert_eq!(openrouter.label, "openrouter");
        assert_eq!(
            openrouter.model_presets.first().map(String::as_str),
            Some("")
        );
        assert!(openrouter
            .model_presets
            .iter()
            .any(|preset| preset == "openrouter/free"));
    }
}
