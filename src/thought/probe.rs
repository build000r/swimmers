use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::thought::emitter_client::EmitterClient;
use crate::thought::loop_runner::SessionInfo;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{RestState, SessionState, ThoughtSource, ThoughtState};

pub const THOUGHT_CONFIG_PROBE_REPLAY_TEXT: &str = concat!(
    "running cargo test --all\n",
    "test auth::login_rejects_missing_token ... FAILED\n",
    "assertion failed: status should stay unauthorized after missing token\n",
    "reviewing auth middleware header parsing and session fallback handling\n",
);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtConfigProbeResult {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backend_error: Option<String>,
    pub llm_calls: u64,
}

pub fn thought_config_probe_session(now: DateTime<Utc>) -> SessionInfo {
    SessionInfo {
        session_id: "thought-config-test".to_string(),
        state: SessionState::Busy,
        exited: false,
        tool: None,
        cwd: "/tmp/project".to_string(),
        replay_text: THOUGHT_CONFIG_PROBE_REPLAY_TEXT.to_string(),
        thought: None,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        rest_state: RestState::Active,
        commit_candidate: false,
        objective_fingerprint: None,
        thought_updated_at: None,
        token_count: 1_000,
        context_limit: 192_000,
        last_activity_at: now,
    }
}

pub async fn run_thought_config_probe(config: &ThoughtConfig) -> ThoughtConfigProbeResult {
    let mut client = EmitterClient::new();
    let sample = thought_config_probe_session(Utc::now());
    match client.next_sync_response(config, &[sample]).await {
        Ok(response) => {
            let ok = response.last_backend_error.is_none() && response.llm_calls > 0;
            let message = if let Some(err) = response.last_backend_error.clone() {
                format!("probe failed: {err}")
            } else if response.llm_calls > 0 {
                "probe succeeded".to_string()
            } else {
                "probe inconclusive: no llm call was made".to_string()
            };
            ThoughtConfigProbeResult {
                ok,
                message,
                last_backend_error: response.last_backend_error,
                llm_calls: response.llm_calls,
            }
        }
        Err(err) => ThoughtConfigProbeResult {
            ok: false,
            message: format!("probe failed: {err}"),
            last_backend_error: Some(err.to_string()),
            llm_calls: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::THOUGHT_CONFIG_PROBE_REPLAY_TEXT;

    #[test]
    fn probe_replay_text_exceeds_meaningful_terminal_threshold() {
        let non_whitespace = THOUGHT_CONFIG_PROBE_REPLAY_TEXT
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .count();
        assert!(
            non_whitespace >= 100,
            "probe replay text should exceed clawgs terminal delta threshold"
        );
    }
}
