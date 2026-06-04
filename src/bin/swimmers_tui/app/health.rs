use super::*;

const API_FAILURE_BANNER_THRESHOLD: u8 = 3;
pub(super) const API_STALE_BANNER_TEXT: &str = "API disconnected - showing stale data";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ApiRefreshHealth {
    consecutive_errors: u8,
}

impl ApiRefreshHealth {
    pub(super) fn record_success(&mut self) {
        self.consecutive_errors = 0;
    }

    pub(super) fn record_failure(&mut self) {
        self.consecutive_errors = self.consecutive_errors.saturating_add(1);
    }

    pub(super) fn banner_text(&self) -> Option<&'static str> {
        (self.consecutive_errors >= API_FAILURE_BANNER_THRESHOLD).then_some(API_STALE_BANNER_TEXT)
    }
}

fn concise_health_detail(value: Option<&String>) -> Option<String> {
    value
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| truncate_label(text, 64))
}

pub(super) fn backend_health_warning_text(health: &BackendHealthResponse) -> Option<String> {
    let persistence = &health.persistence;
    if !persistence.available {
        return Some("persistence unavailable".to_string());
    }
    if !persistence.ok {
        let operation = persistence
            .last_failed_operation
            .as_deref()
            .unwrap_or("write");
        let detail = concise_health_detail(persistence.last_error.as_ref())
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        return Some(format!("persistence degraded: {operation}{detail}"));
    }

    let thought = &health.thought_bridge;
    match thought.status.as_str() {
        "healthy" | "" => None,
        "degraded" => {
            let detail = concise_health_detail(thought.last_backend_error.as_ref())
                .or_else(|| concise_health_detail(thought.last_error.as_ref()))
                .map(|error| format!(": {error}"))
                .unwrap_or_default();
            Some(format!("thought bridge degraded{detail}"))
        }
        "unhealthy" => {
            let detail = concise_health_detail(
                thought
                    .shutdown_reason
                    .as_ref()
                    .or(thought.last_error.as_ref()),
            )
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
            Some(format!("thought bridge unhealthy{detail}"))
        }
        other => Some(format!("thought bridge {other}")),
    }
}

pub(super) fn dependency_degradation_line(deps: &BackendDependencyLedger) -> Option<String> {
    let checks: &[(&str, &BackendDependencySnapshot)] = &[
        ("tmux capture", &deps.tmux_capture),
        ("native scripts", &deps.native_scripts),
        ("remote targets", &deps.remote_targets),
    ];
    let mut parts = Vec::new();
    for &(label, snap) in checks {
        let tag = match snap.status.as_str() {
            "unavailable" => "unavailable",
            "degraded" => "degraded",
            _ => continue,
        };
        let detail = concise_health_detail(snap.last_error.as_ref())
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        parts.push(format!("{label} {tag}{detail}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("  "))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DaemonDefaultsStatus {
    #[default]
    Unknown,
    Available,
    Unavailable,
}

impl DaemonDefaultsStatus {
    pub(super) fn from_defaults(defaults: Option<&DaemonDefaults>) -> Self {
        if defaults.is_some() {
            Self::Available
        } else {
            Self::Unavailable
        }
    }

    pub(crate) fn is_unavailable(self) -> bool {
        self == Self::Unavailable
    }
}
