use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tracing::debug;

use super::{LifecycleEvent, SessionSupervisor, PROCESS_EXIT_SUMMARY_TIMEOUT};
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::types::SessionState;

const PROCESS_EXIT_REAP_INTERVAL: Duration = Duration::from_millis(250);
const PROCESS_EXIT_DELETE_GRACE: Duration = Duration::ZERO;

impl SessionSupervisor {
    /// Spawn a background task that reaps exited sessions once actors report
    /// them as exited.
    pub fn spawn_process_exit_reaper(self: &Arc<Self>) {
        let supervisor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PROCESS_EXIT_REAP_INTERVAL);
            loop {
                interval.tick().await;
                supervisor.reap_exited_sessions().await;
            }
        });
    }

    async fn collect_exited_session_ids(&self, timeout: Duration) -> HashSet<String> {
        let handles: Vec<ActorHandle> = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect()
        };

        let mut exited_ids = HashSet::new();
        for handle in handles {
            let (tx, rx) = oneshot::channel();
            if handle
                .cmd_tx
                .send(SessionCommand::GetSummary(tx))
                .await
                .is_err()
            {
                continue;
            }
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(summary)) if summary.state == SessionState::Exited => {
                    exited_ids.insert(summary.session_id);
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => {
                    debug!(
                        session_id = %handle.session_id,
                        "reaper summary channel dropped"
                    );
                }
                Err(_) => {
                    debug!(
                        session_id = %handle.session_id,
                        "reaper summary request timed out"
                    );
                }
            }
        }

        exited_ids
    }

    async fn reap_exited_sessions(&self) {
        let exited_ids = self
            .collect_exited_session_ids(PROCESS_EXIT_SUMMARY_TIMEOUT)
            .await;
        let ready = self.ready_exited_session_ids(&exited_ids).await;
        let removed = self.remove_ready_exited_handles(&ready).await;
        if removed.is_empty() {
            return;
        }

        self.clear_removed_process_exit_tracking(&removed).await;
        self.emit_process_exit_deletions(removed).await;
        self.persist_registry().await;
    }

    async fn ready_exited_session_ids(&self, exited_ids: &HashSet<String>) -> Vec<String> {
        let now = Instant::now();
        let mut seen = self.process_exit_seen_at.write().await;
        ready_process_exit_ids(&mut seen, exited_ids, now, PROCESS_EXIT_DELETE_GRACE)
    }

    async fn remove_ready_exited_handles(&self, ready: &[String]) -> Vec<ActorHandle> {
        if ready.is_empty() {
            return Vec::new();
        }

        let mut sessions = self.sessions.write().await;
        let mut removed = Vec::with_capacity(ready.len());
        for session_id in ready {
            if let Some(handle) = sessions.remove(session_id) {
                removed.push(handle);
            }
        }
        crate::metrics::set_active_sessions(sessions.len());
        removed
    }

    async fn clear_removed_process_exit_tracking(&self, removed: &[ActorHandle]) {
        let mut seen = self.process_exit_seen_at.write().await;
        for handle in removed {
            seen.remove(&handle.session_id);
        }
    }

    async fn emit_process_exit_deletions(&self, removed: Vec<ActorHandle>) {
        for handle in removed {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
            let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
                session_id: handle.session_id,
                reason: "process_exit".to_string(),
                delete_mode: crate::config::SessionDeleteMode::DetachBridge,
                tmux_session_alive: false,
            });
        }
    }
}

fn ready_process_exit_ids(
    seen: &mut HashMap<String, Instant>,
    exited_ids: &HashSet<String>,
    now: Instant,
    grace: Duration,
) -> Vec<String> {
    seen.retain(|session_id, _| exited_ids.contains(session_id));
    for session_id in exited_ids {
        seen.entry(session_id.clone()).or_insert(now);
    }

    seen.iter()
        .filter(|(_, first_seen)| now.duration_since(**first_seen) >= grace)
        .map(|(session_id, _)| session_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::types::{fallback_rest_state, SessionSummary, ThoughtState};
    use chrono::Utc;
    use std::iter::FromIterator;
    use tokio::sync::mpsc;

    fn test_summary(session_id: &str, state: SessionState) -> SessionSummary {
        let mut summary = SessionSummary::live(
            session_id,
            format!("tmux-{session_id}"),
            state,
            Some("cargo test".to_string()),
            Default::default(),
            "/tmp/project",
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        );
        summary.rest_state = fallback_rest_state(state, ThoughtState::Holding);
        summary
    }

    async fn spawn_summary_handle(summary: SessionSummary) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(
            summary.session_id.clone(),
            summary.tmux_name.clone(),
            cmd_tx,
        );
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    #[test]
    fn ready_process_exit_ids_reaps_immediately_when_grace_is_zero() {
        let mut seen = HashMap::new();
        let exited = HashSet::from_iter(["sess_1".to_string()]);
        let start = Instant::now();

        let ready = ready_process_exit_ids(&mut seen, &exited, start, Duration::ZERO);
        assert_eq!(ready, vec!["sess_1".to_string()]);
    }

    #[test]
    fn ready_process_exit_ids_drops_recovered_sessions() {
        let mut seen = HashMap::new();
        let exited = HashSet::from_iter(["sess_1".to_string()]);
        let start = Instant::now();

        let _ = ready_process_exit_ids(&mut seen, &exited, start, Duration::from_secs(2));
        assert!(seen.contains_key("sess_1"));

        let none = HashSet::new();
        let ready = ready_process_exit_ids(
            &mut seen,
            &none,
            start + Duration::from_secs(10),
            Duration::from_secs(2),
        );
        assert!(ready.is_empty());
        assert!(!seen.contains_key("sess_1"));
    }

    #[tokio::test]
    async fn collect_exited_session_ids_reports_only_exited_sessions() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-idle", SessionState::Idle)).await,
            )
            .await;
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
            )
            .await;

        let exited = supervisor
            .collect_exited_session_ids(Duration::from_millis(50))
            .await;
        assert_eq!(exited, HashSet::from_iter(["sess-exited".to_string()]));
    }

    #[tokio::test]
    async fn reap_exited_sessions_removes_ready_actor_handles() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let mut events = supervisor.subscribe_events();
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
            )
            .await;

        supervisor.reap_exited_sessions().await;
        assert!(supervisor.get_session("sess-exited").await.is_none());
        assert!(!supervisor
            .process_exit_seen_at
            .read()
            .await
            .contains_key("sess-exited"));

        let event = tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .expect("deleted event timeout")
            .expect("deleted event");
        match event {
            LifecycleEvent::Deleted {
                session_id,
                reason,
                delete_mode,
                tmux_session_alive,
            } => {
                assert_eq!(session_id, "sess-exited");
                assert_eq!(reason, "process_exit");
                assert!(matches!(
                    delete_mode,
                    crate::config::SessionDeleteMode::DetachBridge
                ));
                assert!(!tmux_session_alive);
            }
            other => panic!("unexpected lifecycle event: {other:?}"),
        }
    }
}
