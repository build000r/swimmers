// TODO: re-evaluate when the daemon binary itself calls model-rotation logic;
// all public functions here are consumed by swimmers-tui, not by the daemon,
// so the daemon's dead-code pass flags them even though they are load-bearing.
#![allow(dead_code)]

use std::cmp::Ordering;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const MAX_ROTATOR_MODELS: usize = 12;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenRouterModelCache {
    pub generated_at_epoch_ms: u64,
    pub models: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModelEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct OpenRouterModelEntry {
    id: String,
    name: Option<String>,
}

pub fn default_openrouter_candidates() -> Vec<String> {
    vec![
        "openrouter/free".to_string(),
        "liquid/lfm-2.5-1.2b-instruct:free".to_string(),
        "google/gemma-3-4b-it:free".to_string(),
        "nvidia/nemotron-3-nano-30b-a3b:free".to_string(),
        "arcee-ai/trinity-large-preview:free".to_string(),
        "nvidia/nemotron-3-super-120b-a12b:free".to_string(),
    ]
}

pub fn cached_or_default_openrouter_candidates() -> Vec<String> {
    load_openrouter_model_cache()
        .map(|cache| cache.models)
        .filter(|models| !models.is_empty())
        .unwrap_or_else(default_openrouter_candidates)
}

pub fn should_rotate_openrouter_model(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    [
        "not a valid model id",
        "no endpoints available",
        "temporarily rate-limited upstream",
        "returned empty",
        "json parse failed",
        "404 not found",
        "privacy",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub async fn refresh_openrouter_model_cache(
    client: &reqwest::Client,
) -> Result<OpenRouterModelCache, String> {
    let response = client
        .get(OPENROUTER_MODELS_URL)
        .send()
        .await
        .map_err(|err| format!("failed to fetch OpenRouter model catalog: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "failed to fetch OpenRouter model catalog: request failed: {status}"
        ));
    }

    let payload = response
        .json::<OpenRouterModelsResponse>()
        .await
        .map_err(|err| format!("failed to parse OpenRouter model catalog: {err}"))?;

    let mut models = payload
        .data
        .into_iter()
        .filter(is_rotator_candidate)
        .collect::<Vec<_>>();
    models.sort_by(compare_rotator_candidates);

    let mut ordered = vec!["openrouter/free".to_string()];
    for entry in models
        .into_iter()
        .take(MAX_ROTATOR_MODELS.saturating_sub(1))
    {
        if !ordered
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&entry.id))
        {
            ordered.push(entry.id);
        }
    }

    let cache = OpenRouterModelCache {
        generated_at_epoch_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default(),
        models: ordered,
    };
    persist_openrouter_model_cache(&cache)?;
    Ok(cache)
}

pub fn load_openrouter_model_cache() -> Option<OpenRouterModelCache> {
    let path = openrouter_model_cache_path()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn persist_openrouter_model_cache(cache: &OpenRouterModelCache) -> Result<(), String> {
    let Some(path) = openrouter_model_cache_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create OpenRouter cache dir: {err}"))?;
    }
    let raw = serde_json::to_string_pretty(cache)
        .map_err(|err| format!("failed to serialize OpenRouter cache: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("failed to write OpenRouter cache: {err}"))?;
    Ok(())
}

fn openrouter_model_cache_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".swimmers").join("openrouter-free-models.json"))
        .or_else(|| {
            Some(
                PathBuf::from(".")
                    .join("data")
                    .join("swimmers")
                    .join("openrouter-free-models.json"),
            )
        })
}

fn is_rotator_candidate(entry: &OpenRouterModelEntry) -> bool {
    if !entry.id.ends_with(":free") {
        return false;
    }

    let lowered_id = entry.id.to_ascii_lowercase();
    let lowered_name = entry
        .name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    !lowered_id.contains("-vl")
        && !lowered_id.contains(":vl")
        && !lowered_name.contains("vision")
        && !lowered_name.contains("image")
}

fn compare_rotator_candidates(
    left: &OpenRouterModelEntry,
    right: &OpenRouterModelEntry,
) -> Ordering {
    let left_preview = model_is_preview(&left.id, left.name.as_deref());
    let right_preview = model_is_preview(&right.id, right.name.as_deref());
    left_preview
        .cmp(&right_preview)
        .then_with(|| {
            estimated_model_size(&left.id, left.name.as_deref())
                .cmp(&estimated_model_size(&right.id, right.name.as_deref()))
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn model_is_preview(id: &str, name: Option<&str>) -> bool {
    let lowered_id = id.to_ascii_lowercase();
    let lowered_name = name.unwrap_or_default().to_ascii_lowercase();
    lowered_id.contains("preview") || lowered_name.contains("preview")
}

fn estimated_model_size(id: &str, name: Option<&str>) -> u32 {
    parse_billion_size(id)
        .or_else(|| name.and_then(parse_billion_size))
        .unwrap_or(u32::MAX)
}

fn parse_billion_size(text: &str) -> Option<u32> {
    let regex = Regex::new(r"(?i)(\d+(?:\.\d+)?)b").ok()?;
    let captures = regex.captures(text)?;
    let value = captures.get(1)?.as_str().parse::<f32>().ok()?;
    Some((value * 100.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: Option<&str>) -> OpenRouterModelEntry {
        OpenRouterModelEntry {
            id: id.to_string(),
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn is_rotator_candidate_rejects_non_free() {
        assert!(!is_rotator_candidate(&entry("meta/llama-3:plus", None)));
        assert!(!is_rotator_candidate(&entry("openai/gpt-4o", None)));
    }

    #[test]
    fn is_rotator_candidate_rejects_vision_by_id() {
        assert!(!is_rotator_candidate(&entry("model-vl:free", None)));
        assert!(!is_rotator_candidate(&entry("model:vl:free", None)));
    }

    #[test]
    fn is_rotator_candidate_rejects_vision_by_name() {
        assert!(!is_rotator_candidate(&entry(
            "llama:free",
            Some("Vision Model")
        )));
        assert!(!is_rotator_candidate(&entry(
            "llama:free",
            Some("Image Generator")
        )));
    }

    #[test]
    fn is_rotator_candidate_accepts_text_free_models() {
        assert!(is_rotator_candidate(&entry("llama-3:free", None)));
        assert!(is_rotator_candidate(&entry(
            "gemma-3:free",
            Some("Gemma 3")
        )));
        assert!(is_rotator_candidate(&entry(
            "nemotron:free",
            Some("Nemotron")
        )));
    }

    #[test]
    fn parse_billion_size_extracts_decimal_sizes() {
        assert_eq!(parse_billion_size("lfm-2.5-1.2b-instruct"), Some(120));
        assert_eq!(parse_billion_size("llama-3.2-3b"), Some(300));
    }

    #[test]
    fn should_rotate_openrouter_model_matches_invalid_or_stale_catalog_errors() {
        assert!(should_rotate_openrouter_model(
            "haiku is not a valid model ID"
        ));
        assert!(should_rotate_openrouter_model(
            "No endpoints available matching your guardrail restrictions"
        ));
        assert!(should_rotate_openrouter_model(
            "temporarily rate-limited upstream"
        ));
        assert!(!should_rotate_openrouter_model(
            "OPENROUTER_API_KEY not set"
        ));
    }

    #[test]
    fn default_candidates_start_with_router_alias() {
        let models = default_openrouter_candidates();
        assert_eq!(models.first().map(String::as_str), Some("openrouter/free"));
    }
}
