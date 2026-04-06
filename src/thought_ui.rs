// All public items in this module are consumed by swimmers-tui; the daemon
// binary includes the module but does not call these functions directly.
// TODO: re-evaluate when the daemon exposes thought-config UI endpoints.
#![allow(dead_code)]

use crate::openrouter_models::cached_or_default_openrouter_candidates;
use crate::types::{ThoughtConfigBackendMetadata, ThoughtConfigUiMetadata};

const THOUGHT_BACKEND_OPTIONS: [&str; 3] = ["", "openrouter", "codex"];

pub fn thought_backend_cycle_options() -> &'static [&'static str] {
    &THOUGHT_BACKEND_OPTIONS
}

pub fn canonical_thought_backend_key(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => "",
        "claude" | "claude-cli" | "claude_cli" => "openrouter",
        "codex" | "codex-cli" | "codex_cli" => "codex",
        "openrouter" => "openrouter",
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
        "codex" => "presets: auto  5.1-mini  5.3-codex  5.4",
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
        "codex" => vec![
            String::new(),
            "gpt-5.1-codex-mini".to_string(),
            "gpt-5.3-codex".to_string(),
            "gpt-5.4".to_string(),
        ],
        _ => vec![String::new()],
    }
}

pub fn normalize_thought_model_for_backend(backend: &str, model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let keep = match canonical_thought_backend_key(backend) {
        "" => false,
        "openrouter" => trimmed.contains('/'),
        "codex" => trimmed.starts_with("gpt-"),
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
            normalize_thought_model_for_backend("codex", "gpt-5.4"),
            "gpt-5.4"
        );
        assert!(normalize_thought_model_for_backend("", "haiku").is_empty());
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
