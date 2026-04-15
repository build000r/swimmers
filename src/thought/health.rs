use std::cmp;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::watch;

const DEFAULT_SYNC_TIMEOUT_MULTIPLIER: u32 = 2;
const MIN_SYNC_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_FAILURE_BACKOFF: Duration = Duration::from_millis(500);
const MAX_FAILURE_BACKOFF: Duration = Duration::from_secs(60);
const UNHEALTHY_AFTER_TICKS: u32 = 3;
const MIN_UNHEALTHY_AFTER: Duration = Duration::from_secs(30);
const SELF_FENCE_AFTER_TICKS: u32 = 12;
const MIN_SELF_FENCE_AFTER: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeTiming {
    pub tick: Duration,
    pub sync_timeout: Duration,
    pub min_failure_backoff: Duration,
    pub max_failure_backoff: Duration,
    pub unhealthy_after: Duration,
    pub self_fence_after: Duration,
}

impl BridgeTiming {
    pub fn from_tick(tick: Duration) -> Self {
        Self {
            tick,
            sync_timeout: max_duration(
                saturating_mul_duration(tick, DEFAULT_SYNC_TIMEOUT_MULTIPLIER),
                MIN_SYNC_TIMEOUT,
            ),
            min_failure_backoff: MIN_FAILURE_BACKOFF,
            max_failure_backoff: MAX_FAILURE_BACKOFF,
            unhealthy_after: max_duration(
                saturating_mul_duration(tick, UNHEALTHY_AFTER_TICKS),
                MIN_UNHEALTHY_AFTER,
            ),
            self_fence_after: max_duration(
                saturating_mul_duration(tick, SELF_FENCE_AFTER_TICKS),
                MIN_SELF_FENCE_AFTER,
            ),
        }
    }

    pub fn retry_delay(self, consecutive_failures: u32) -> Duration {
        let exponent = consecutive_failures.saturating_sub(1).min(16);
        let scaled = saturating_mul_duration(self.min_failure_backoff, 1u32 << exponent);
        max_duration(self.tick, min_duration(scaled, self.max_failure_backoff))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeStatus {
    Starting,
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BridgeHealthSnapshot {
    pub status: BridgeStatus,
    pub tick_ms: u64,
    pub sync_timeout_ms: u64,
    pub consecutive_failures: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backend_error: Option<String>,
    pub next_retry_delay_ms: u64,
    pub shutdown_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shutdown_reason: Option<String>,
}

impl BridgeHealthSnapshot {
    pub fn is_ready(&self) -> bool {
        matches!(self.status, BridgeStatus::Healthy | BridgeStatus::Degraded)
    }
}

struct BridgeHealthInner {
    status: BridgeStatus,
    started_at: Instant,
    consecutive_failures: u32,
    last_success_instant: Option<Instant>,
    last_success_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    last_backend_error: Option<String>,
    next_retry_delay: Duration,
    unhealthy_since: Option<Instant>,
    shutdown_reason: Option<String>,
}

pub struct BridgeHealthState {
    timing: BridgeTiming,
    inner: Mutex<BridgeHealthInner>,
    shutdown_tx: watch::Sender<Option<String>>,
}

impl BridgeHealthState {
    pub fn new_with_tick(tick: Duration) -> Self {
        Self::with_timing(BridgeTiming::from_tick(tick))
    }

    pub fn with_timing(timing: BridgeTiming) -> Self {
        let (shutdown_tx, _) = watch::channel(None);
        Self {
            timing,
            inner: Mutex::new(BridgeHealthInner {
                status: BridgeStatus::Starting,
                started_at: Instant::now(),
                consecutive_failures: 0,
                last_success_instant: None,
                last_success_at: None,
                last_failure_at: None,
                last_error: None,
                last_backend_error: None,
                next_retry_delay: Duration::ZERO,
                unhealthy_since: None,
                shutdown_reason: None,
            }),
            shutdown_tx,
        }
    }

    pub fn timing(&self) -> BridgeTiming {
        self.timing
    }

    pub fn snapshot(&self) -> BridgeHealthSnapshot {
        let inner = self.inner.lock().expect("bridge health mutex");
        BridgeHealthSnapshot {
            status: inner.status,
            tick_ms: duration_millis(self.timing.tick),
            sync_timeout_ms: duration_millis(self.timing.sync_timeout),
            consecutive_failures: inner.consecutive_failures,
            last_success_at: inner.last_success_at,
            last_failure_at: inner.last_failure_at,
            last_error: inner.last_error.clone(),
            last_backend_error: inner.last_backend_error.clone(),
            next_retry_delay_ms: duration_millis(inner.next_retry_delay),
            shutdown_requested: inner.shutdown_reason.is_some(),
            shutdown_reason: inner.shutdown_reason.clone(),
        }
    }

    pub fn next_retry_delay_for_failure(&self) -> Duration {
        let inner = self.inner.lock().expect("bridge health mutex");
        self.timing
            .retry_delay(inner.consecutive_failures.saturating_add(1))
    }

    pub fn record_success(&self, last_backend_error: Option<String>) {
        let mut inner = self.inner.lock().expect("bridge health mutex");
        let now = Instant::now();
        inner.status = if last_backend_error.is_some() {
            BridgeStatus::Degraded
        } else {
            BridgeStatus::Healthy
        };
        inner.consecutive_failures = 0;
        inner.last_success_instant = Some(now);
        inner.last_success_at = Some(Utc::now());
        inner.last_error = None;
        inner.last_backend_error = last_backend_error;
        inner.next_retry_delay = Duration::ZERO;
        inner.unhealthy_since = None;
    }

    pub fn record_failure(&self, error: impl Into<String>, retry_delay: Duration) {
        let error = error.into();
        let now = Instant::now();
        let now_wall = Utc::now();
        let mut shutdown_reason = None;

        {
            let mut inner = self.inner.lock().expect("bridge health mutex");
            inner.consecutive_failures = inner.consecutive_failures.saturating_add(1);
            inner.last_failure_at = Some(now_wall);
            inner.last_error = Some(error.clone());
            inner.last_backend_error = None;
            inner.next_retry_delay = retry_delay;

            let baseline = inner.last_success_instant.unwrap_or(inner.started_at);
            let stale_for = now.saturating_duration_since(baseline);
            let unhealthy =
                inner.consecutive_failures >= 3 || stale_for >= self.timing.unhealthy_after;

            if unhealthy {
                let unhealthy_since = *inner.unhealthy_since.get_or_insert(now);
                inner.status = BridgeStatus::Unhealthy;
                if inner.shutdown_reason.is_none()
                    && now.saturating_duration_since(unhealthy_since)
                        >= self.timing.self_fence_after
                {
                    let reason = format!(
                        "thought bridge unhealthy for {}ms after {} consecutive failures: {}",
                        duration_millis(self.timing.self_fence_after),
                        inner.consecutive_failures,
                        error
                    );
                    inner.shutdown_reason = Some(reason.clone());
                    shutdown_reason = Some(reason);
                }
            } else {
                inner.status = BridgeStatus::Degraded;
                inner.unhealthy_since = None;
            }
        }

        if let Some(reason) = shutdown_reason {
            self.shutdown_tx.send_replace(Some(reason));
        }
    }

    pub fn shutdown_reason(&self) -> Option<String> {
        let inner = self.inner.lock().expect("bridge health mutex");
        inner.shutdown_reason.clone()
    }

    pub async fn wait_for_shutdown_request(&self) -> String {
        let mut rx = self.shutdown_tx.subscribe();
        loop {
            if let Some(reason) = rx.borrow().clone() {
                return reason;
            }
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

fn saturating_mul_duration(duration: Duration, factor: u32) -> Duration {
    duration.checked_mul(factor).unwrap_or(Duration::MAX)
}

fn min_duration(left: Duration, right: Duration) -> Duration {
    cmp::min(left, right)
}

fn max_duration(left: Duration, right: Duration) -> Duration {
    cmp::max(left, right)
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn retry_delay_honors_tick_floor_and_backoff_cap() {
        let timing = BridgeTiming {
            tick: Duration::from_millis(25),
            sync_timeout: Duration::from_millis(50),
            min_failure_backoff: Duration::from_millis(10),
            max_failure_backoff: Duration::from_millis(80),
            unhealthy_after: Duration::from_millis(75),
            self_fence_after: Duration::from_millis(150),
        };

        assert_eq!(timing.retry_delay(1), Duration::from_millis(25));
        assert_eq!(timing.retry_delay(2), Duration::from_millis(25));
        assert_eq!(timing.retry_delay(4), Duration::from_millis(80));
        assert_eq!(timing.retry_delay(8), Duration::from_millis(80));
    }

    #[tokio::test]
    async fn state_escalates_to_unhealthy_and_requests_shutdown() {
        let timing = BridgeTiming {
            tick: Duration::from_millis(5),
            sync_timeout: Duration::from_millis(20),
            min_failure_backoff: Duration::from_millis(5),
            max_failure_backoff: Duration::from_millis(10),
            unhealthy_after: Duration::from_millis(10),
            self_fence_after: Duration::from_millis(15),
        };
        let health = Arc::new(BridgeHealthState::with_timing(timing));

        health.record_failure("spawn failed", Duration::from_millis(5));
        assert_eq!(health.snapshot().status, BridgeStatus::Degraded);

        tokio::time::sleep(Duration::from_millis(12)).await;
        health.record_failure("timeout", Duration::from_millis(10));
        assert_eq!(health.snapshot().status, BridgeStatus::Unhealthy);
        assert!(!health.snapshot().shutdown_requested);

        tokio::time::sleep(Duration::from_millis(20)).await;
        health.record_failure("still timing out", Duration::from_millis(10));
        let snapshot = health.snapshot();
        assert_eq!(snapshot.status, BridgeStatus::Unhealthy);
        assert!(snapshot.shutdown_requested);

        let reason = health.wait_for_shutdown_request().await;
        assert!(reason.contains("thought bridge unhealthy"));
    }

    #[test]
    fn success_clears_failures_and_marks_backend_error_as_degraded() {
        let health = BridgeHealthState::new_with_tick(Duration::from_secs(1));
        health.record_failure("boom", Duration::from_secs(1));
        assert_eq!(health.snapshot().status, BridgeStatus::Degraded);

        health.record_success(Some("model unavailable".to_string()));
        let snapshot = health.snapshot();
        assert_eq!(snapshot.status, BridgeStatus::Degraded);
        assert_eq!(snapshot.consecutive_failures, 0);
        assert_eq!(
            snapshot.last_backend_error.as_deref(),
            Some("model unavailable")
        );
        assert!(snapshot.last_error.is_none());
    }
}
