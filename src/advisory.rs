use serde::Deserialize;

use chrono::{DateTime, Utc};

use crate::types::{AdvisoryMetadataSummary, SessionSummary};

const JSON_ENV: &str = "SWIMMERS_ADVISORY_METADATA";
const JSON_FILES_ENV: &str = "SWIMMERS_ADVISORY_METADATA_FILES";
const ACCEPTED_SOURCES: &[&str] = &["c0", "ntm", "sbp", "skillbox", "manual"];

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
    #[serde(default)]
    group_key: Option<String>,
    #[serde(default)]
    observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    stale_after_ms: Option<u64>,
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
    let source = clean_source(raw.source)?;
    let label = clean_required(raw.label)?;
    let value = clean_required(raw.value)?;
    let group_key = raw
        .group_key
        .and_then(clean_optional)
        .or_else(|| Some(default_advisory_group_key(&source, &label, &value)));
    let freshness_ms = raw.observed_at.and_then(advisory_freshness_ms);
    let stale = advisory_is_stale(freshness_ms, raw.stale_after_ms);
    Some(AdvisoryMetadataSummary {
        source,
        label,
        value,
        status: "external".to_string(),
        stale,
        group_key,
        observed_at: raw.observed_at,
        freshness_ms,
    })
}

fn raw_advisories() -> Vec<RawAdvisoryMetadata> {
    let mut advisories = fixed_env_advisories();
    advisories.extend(json_env_advisories());
    advisories.extend(json_file_advisories());
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
                group_key: None,
                observed_at: None,
                stale_after_ms: None,
            })
        })
        .collect()
}

fn json_env_advisories() -> Vec<RawAdvisoryMetadata> {
    let Ok(raw) = std::env::var(JSON_ENV) else {
        return Vec::new();
    };
    parse_raw_advisories(&raw)
}

fn json_file_advisories() -> Vec<RawAdvisoryMetadata> {
    let Ok(raw_paths) = std::env::var(JSON_FILES_ENV) else {
        return Vec::new();
    };
    std::env::split_paths(&raw_paths)
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|content| parse_raw_advisories(&content))
        .collect()
}

fn parse_raw_advisories(raw: &str) -> Vec<RawAdvisoryMetadata> {
    serde_json::from_str::<Vec<RawAdvisoryMetadata>>(raw)
        .or_else(|_| serde_json::from_str::<RawAdvisoryMetadata>(raw).map(|item| vec![item]))
        .unwrap_or_else(|_| {
            raw.lines()
                .filter_map(|line| serde_json::from_str::<RawAdvisoryMetadata>(line).ok())
                .collect()
        })
}

fn clean_source(value: String) -> Option<String> {
    let source = clean_required(value)?;
    let normalized = source.to_ascii_lowercase();
    ACCEPTED_SOURCES
        .contains(&normalized.as_str())
        .then_some(normalized)
}

fn clean_required(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn clean_optional(value: String) -> Option<String> {
    clean_required(value)
}

fn default_advisory_group_key(source: &str, label: &str, value: &str) -> String {
    [source, label, value]
        .into_iter()
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(":")
}

fn advisory_freshness_ms(observed_at: DateTime<Utc>) -> Option<u64> {
    let age = Utc::now().signed_duration_since(observed_at);
    if age < chrono::Duration::zero() {
        return Some(0);
    }
    age.to_std()
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn advisory_is_stale(freshness_ms: Option<u64>, stale_after_ms: Option<u64>) -> bool {
    match (freshness_ms, stale_after_ms) {
        (Some(freshness_ms), Some(stale_after_ms)) => freshness_ms > stale_after_ms,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<T>(vars: &[(&str, Option<&str>)], test: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let mut keys = vec![JSON_ENV, JSON_FILES_ENV];
        keys.extend(FIXED_ENVS.iter().map(|(key, _, _)| *key));
        for (key, _) in vars {
            if !keys.contains(key) {
                keys.push(*key);
            }
        }
        let old = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in &keys {
            std::env::remove_var(key);
        }
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
                assert_eq!(local[0].group_key.as_deref(), Some("c0:c0 group:wave-a"));
                assert!(local[0].freshness_ms.is_none());
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
                        {"source":"sbp","label":"skills","value":"ok","status":"fresh","stale":false,"target_id":"skillbox","cwd_prefix":"/srv/skillbox","group_key":"skills:ok"},
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
                assert_eq!(remote[0].group_key.as_deref(), Some("skills:ok"));
                assert!(unmatched.is_empty());
            },
        );
    }

    #[test]
    fn json_advisory_env_uses_observed_time_and_freshness_window() {
        let observed_at = Utc::now().to_rfc3339();
        let raw = format!(
            r#"[
                {{"source":"c0","label":"group","value":"swimmers","target_id":"local","observed_at":"{observed_at}","stale_after_ms":60000}}
            ]"#
        );
        with_env(
            &[("SWIMMERS_ADVISORY_METADATA", Some(raw.as_str()))],
            || {
                let local = advisory_for_target("local", Some("/Users/tester/repos/swimmers"));

                assert_eq!(local.len(), 1);
                assert_eq!(local[0].status, "external");
                assert!(!local[0].stale);
                assert!(local[0].observed_at.is_some());
                assert!(local[0].freshness_ms.is_some());
            },
        );
    }

    #[test]
    fn malformed_json_advisory_env_is_ignored() {
        with_env(&[("SWIMMERS_ADVISORY_METADATA", Some("not-json"))], || {
            assert!(advisory_for_target("local", None).is_empty());
        });
    }

    #[test]
    fn unsupported_advisory_sources_are_ignored() {
        with_env(
            &[(
                "SWIMMERS_ADVISORY_METADATA",
                Some(r#"[{"source":"scheduler","label":"claim","value":"trusted"}]"#),
            )],
            || {
                assert!(advisory_for_target("local", None).is_empty());
            },
        );
    }

    #[test]
    fn advisory_file_env_reads_jsonl_without_running_external_tools() {
        let path = std::env::temp_dir().join(format!(
            "swimmers-advisory-{}-{}.jsonl",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &path,
            "{\"source\":\"ntm\",\"label\":\"wave\",\"value\":\"frontier\",\"target_id\":\"skillbox\",\"cwd_prefix\":\"/srv/skillbox\"}\n",
        )
        .expect("write advisory fixture");
        let path_string = path.to_string_lossy().to_string();
        with_env(
            &[(
                "SWIMMERS_ADVISORY_METADATA_FILES",
                Some(path_string.as_str()),
            )],
            || {
                let remote = advisory_for_target("skillbox", Some("/srv/skillbox/repos/swimmers"));

                assert_eq!(remote.len(), 1);
                assert_eq!(remote[0].source, "ntm");
                assert_eq!(remote[0].label, "wave");
                assert_eq!(remote[0].value, "frontier");
                assert_eq!(remote[0].status, "external");
            },
        );
        let _ = std::fs::remove_file(path);
    }
}
