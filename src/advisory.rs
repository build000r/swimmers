use serde::Deserialize;

use crate::types::{AdvisoryMetadataSummary, SessionSummary};

const JSON_ENV: &str = "SWIMMERS_ADVISORY_METADATA";

const FIXED_ENVS: &[(&str, &str, &str)] = &[
    ("SWIMMERS_ADVISORY_C0_GROUP", "c0", "c0 group"),
    ("SWIMMERS_ADVISORY_NTM_WAVE", "ntm", "ntm wave"),
    ("SWIMMERS_ADVISORY_LOAD_STATE", "load_guard", "capacity"),
    ("SWIMMERS_ADVISORY_SBP_STATUS", "sbp", "skills"),
];

#[derive(Debug, Clone, Default, Deserialize)]
struct RawAdvisoryMetadata {
    #[serde(default)]
    source: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    cwd_prefix: Option<String>,
}

pub fn attach_advisories_to_sessions(sessions: &mut [SessionSummary]) {
    for session in sessions {
        let cwd = session
            .environment
            .canonical_cwd
            .as_deref()
            .or_else(|| (!session.cwd.is_empty()).then_some(session.cwd.as_str()));
        session.environment.advisory =
            advisory_for_target(session.environment.target_id.as_str(), cwd);
    }
}

pub fn advisory_for_target(target_id: &str, cwd: Option<&str>) -> Vec<AdvisoryMetadataSummary> {
    raw_advisories()
        .into_iter()
        .filter(|raw| advisory_matches(raw, target_id, cwd))
        .filter_map(advisory_summary)
        .collect()
}

fn advisory_matches(raw: &RawAdvisoryMetadata, target_id: &str, cwd: Option<&str>) -> bool {
    let target = raw
        .target_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("local");
    if target != target_id {
        return false;
    }
    let Some(prefix) = raw
        .cwd_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    cwd.is_some_and(|cwd| cwd.starts_with(prefix))
}

fn advisory_summary(raw: RawAdvisoryMetadata) -> Option<AdvisoryMetadataSummary> {
    let source = clean_required(raw.source)?;
    let label = clean_required(raw.label)?;
    let value = clean_required(raw.value)?;
    Some(AdvisoryMetadataSummary {
        source,
        label,
        value,
        status: "external".to_string(),
        stale: true,
    })
}

fn raw_advisories() -> Vec<RawAdvisoryMetadata> {
    let mut advisories = fixed_env_advisories();
    advisories.extend(json_env_advisories());
    advisories
}

fn fixed_env_advisories() -> Vec<RawAdvisoryMetadata> {
    FIXED_ENVS
        .iter()
        .filter_map(|(env_key, source, label)| {
            let value = std::env::var(env_key).ok()?;
            Some(RawAdvisoryMetadata {
                source: (*source).to_string(),
                label: (*label).to_string(),
                value,
                target_id: Some("local".to_string()),
                cwd_prefix: None,
            })
        })
        .collect()
}

fn json_env_advisories() -> Vec<RawAdvisoryMetadata> {
    let Ok(raw) = std::env::var(JSON_ENV) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<RawAdvisoryMetadata>>(&raw).unwrap_or_default()
}

fn clean_required(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<T>(vars: &[(&str, Option<&str>)], test: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let old = vars
            .iter()
            .map(|(key, _)| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for (key, value) in vars {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let result = test();
        for (key, value) in old {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        result
    }

    #[test]
    fn fixed_advisory_envs_are_local_external_and_stale() {
        with_env(
            &[
                ("SWIMMERS_ADVISORY_C0_GROUP", Some("wave-a")),
                ("SWIMMERS_ADVISORY_METADATA", None),
            ],
            || {
                let local = advisory_for_target("local", None);
                let remote = advisory_for_target("skillbox", None);

                assert_eq!(local.len(), 1);
                assert_eq!(local[0].source, "c0");
                assert_eq!(local[0].label, "c0 group");
                assert_eq!(local[0].value, "wave-a");
                assert_eq!(local[0].status, "external");
                assert!(local[0].stale);
                assert!(remote.is_empty());
            },
        );
    }

    #[test]
    fn json_advisory_env_filters_by_target_and_cwd_prefix() {
        with_env(
            &[(
                "SWIMMERS_ADVISORY_METADATA",
                Some(
                    r#"[
                        {"source":"sbp","label":"skills","value":"ok","status":"fresh","stale":false,"target_id":"skillbox","cwd_prefix":"/srv/skillbox"},
                        {"source":"ntm","label":"wave","value":"local","target_id":"local"},
                        {"source":"","label":"ignored","value":"bad","target_id":"skillbox"}
                    ]"#,
                ),
            )],
            || {
                let remote = advisory_for_target("skillbox", Some("/srv/skillbox/repos/swimmers"));
                let unmatched = advisory_for_target("skillbox", Some("/tmp/repos/swimmers"));

                assert_eq!(remote.len(), 1);
                assert_eq!(remote[0].source, "sbp");
                assert_eq!(remote[0].value, "ok");
                assert_eq!(remote[0].status, "external");
                assert!(remote[0].stale);
                assert!(unmatched.is_empty());
            },
        );
    }

    #[test]
    fn malformed_json_advisory_env_is_ignored() {
        with_env(&[("SWIMMERS_ADVISORY_METADATA", Some("not-json"))], || {
            assert!(advisory_for_target("local", None).is_empty());
        });
    }
}
