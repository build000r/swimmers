use super::*;

const ATTENTION_GROUP_MAX_SESSIONS: usize = 6;
const ATTENTION_GROUP_SIZE_ENV: &str = "SWIMMERS_ATTENTION_GROUP_SIZE";
const ATTENTION_GROUP_LAYOUT_ENV: &str = "SWIMMERS_ATTENTION_GROUP_LAYOUT";
const ATTENTION_GROUP_INCLUDE_UNNUMBERED_ENV: &str = "SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED";

pub(super) fn attention_group_max_sessions() -> usize {
    std::env::var(ATTENTION_GROUP_SIZE_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(1, ATTENTION_GROUP_MAX_SESSIONS))
        .unwrap_or(ATTENTION_GROUP_MAX_SESSIONS)
}

pub(super) fn attention_group_layout() -> AttentionGroupLayout {
    std::env::var(ATTENTION_GROUP_LAYOUT_ENV)
        .ok()
        .as_deref()
        .map(AttentionGroupLayout::from_env_value)
        .unwrap_or_default()
}

pub(super) fn attention_group_include_unnumbered_sessions() -> bool {
    env_bool(ATTENTION_GROUP_INCLUDE_UNNUMBERED_ENV)
}

fn env_bool(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
