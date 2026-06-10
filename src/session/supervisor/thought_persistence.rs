use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, Notify};
use tracing::warn;

use super::SessionSupervisor;
use crate::persistence::file_store::ThoughtSnapshot;
use crate::thought::loop_runner::{SessionInfo, SessionProvider};
use crate::thought::protocol::ThoughtDeliveryState;
use crate::types::{
    ActionCue, RestState, ThoughtPersistenceBackpressureSnapshot, ThoughtSource, ThoughtState,
};

pub(super) const THOUGHT_PERSIST_QUEUE_CAP: usize = 256;

impl SessionSupervisor {
    fn begin_pending_thought_persist(&self) {
        self.pending_thought_persists.fetch_add(1, Ordering::SeqCst);
        self.pending_thought_persists_notify.notify_waiters();
    }

    fn finish_pending_thought_persist(&self) {
        let previous = self.pending_thought_persists.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous > 0, "pending thought persist counter underflow");
        self.pending_thought_persists_notify.notify_waiters();
    }

    fn set_thought_persist_queue_capacity(&self, capacity: usize) {
        self.thought_persist_queue_capacity
            .store(capacity, Ordering::SeqCst);
    }

    fn thought_persist_queue_capacity(&self) -> usize {
        self.thought_persist_queue_capacity.load(Ordering::SeqCst)
    }

    fn record_thought_persist_queue_depth(&self, depth: usize) {
        let cap = self.thought_persist_queue_capacity();
        self.thought_persist_queue_depth
            .store(depth.min(cap), Ordering::SeqCst);
    }

    fn set_thought_persist_overflow_slots(&self, slots: usize) {
        // This only updates the overflow-slot gauge; it does not change the
        // `pending_thought_persists` counter that shutdown waiters re-read in
        // `wait_for_pending_thought_persists`, so waking them here would be a
        // spurious wakeup. The real counter changes notify waiters themselves.
        self.thought_persist_overflow_slots
            .store(slots, Ordering::SeqCst);
    }

    fn record_thought_persist_suppression(
        &self,
        counter: &AtomicU64,
        session_id: &str,
        label: &'static str,
    ) {
        counter.fetch_add(1, Ordering::SeqCst);
        crate::metrics::increment_thought_suppression(session_id, label, "persistence");
    }

    fn record_thought_persist_queue_full(&self, session_id: &str) {
        self.record_thought_persist_suppression(
            &self.thought_persist_queue_full_count,
            session_id,
            "persistence_queue_full",
        );
    }

    fn record_thought_persist_coalesced(&self, session_id: &str) {
        self.record_thought_persist_suppression(
            &self.thought_persist_coalesced_count,
            session_id,
            "persistence_coalesced",
        );
    }

    fn record_thought_persist_dropped(&self, session_id: &str) {
        self.record_thought_persist_suppression(
            &self.thought_persist_dropped_count,
            session_id,
            "persistence_dropped",
        );
    }

    pub fn thought_persistence_backpressure_snapshot(
        &self,
    ) -> ThoughtPersistenceBackpressureSnapshot {
        ThoughtPersistenceBackpressureSnapshot {
            queue_capacity: self.thought_persist_queue_capacity(),
            queue_depth: self.thought_persist_queue_depth.load(Ordering::SeqCst),
            pending_count: self.pending_thought_persists.load(Ordering::SeqCst),
            overflow_slots: self.thought_persist_overflow_slots.load(Ordering::SeqCst),
            queue_full_count: self.thought_persist_queue_full_count.load(Ordering::SeqCst),
            coalesced_count: self.thought_persist_coalesced_count.load(Ordering::SeqCst),
            dropped_count: self.thought_persist_dropped_count.load(Ordering::SeqCst),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_thought_persistence_backpressure_for_test(
        &self,
        queue_depth: usize,
        overflow_slots: usize,
        queue_full_count: u64,
        coalesced_count: u64,
        dropped_count: u64,
    ) {
        self.thought_persist_queue_depth
            .store(queue_depth, Ordering::SeqCst);
        self.thought_persist_overflow_slots
            .store(overflow_slots, Ordering::SeqCst);
        self.thought_persist_queue_full_count
            .store(queue_full_count, Ordering::SeqCst);
        self.thought_persist_coalesced_count
            .store(coalesced_count, Ordering::SeqCst);
        self.thought_persist_dropped_count
            .store(dropped_count, Ordering::SeqCst);
    }

    pub async fn wait_for_pending_thought_persists(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            // Register the waiter (via `enable`) BEFORE reading the counter so a
            // notify_waiters() that fires between the load and the await is not
            // lost. Without `enable`, `Notified` is only added to the wait list
            // on its first poll, opening a race window during shutdown where a
            // single in-flight persist's wakeup arrives before we begin awaiting
            // and we then sit on the timeout instead of completing immediately.
            let notified = self.pending_thought_persists_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let pending = self.pending_thought_persists.load(Ordering::SeqCst);
            if pending == 0 {
                return true;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                warn!(
                    pending,
                    timeout_ms = timeout.as_millis() as u64,
                    "timed out waiting for pending thought persists to drain"
                );
                return false;
            }

            if tokio::time::timeout(remaining, notified).await.is_err() {
                let still_pending = self.pending_thought_persists.load(Ordering::SeqCst);
                warn!(
                    pending = still_pending,
                    timeout_ms = timeout.as_millis() as u64,
                    "timed out waiting for pending thought persists to drain"
                );
                return still_pending == 0;
            }
        }
    }

    /// Persist a thought update for a specific session.
    #[allow(clippy::too_many_arguments)]
    pub async fn persist_thought(
        &self,
        session_id: &str,
        thought: Option<&str>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        rest_state: RestState,
        commit_candidate: bool,
        action_cues: Vec<ActionCue>,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_changed_at: Option<DateTime<Utc>>,
        objective_fingerprint: Option<String>,
    ) {
        {
            let mut thought_snapshots = self.thought_snapshots.write().await;
            let objective_changed_at = objective_changed_at.or_else(|| {
                thought_snapshots
                    .get(session_id)
                    .and_then(|existing| existing.objective_changed_at)
            });
            thought_snapshots.insert(
                session_id.to_string(),
                ThoughtSnapshot {
                    thought: thought.map(|value| value.to_string()),
                    thought_state,
                    thought_source,
                    rest_state,
                    commit_candidate,
                    action_cues: action_cues.clone(),
                    objective_changed_at,
                    objective_fingerprint: objective_fingerprint.clone(),
                    token_count,
                    context_limit,
                    updated_at,
                    delivery: delivery.clone(),
                },
            );
        }

        let store = {
            let guard = self.persistence.read().await;
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => return,
            }
        };

        store
            .save_thought(
                session_id,
                thought,
                token_count,
                context_limit,
                thought_state,
                thought_source,
                rest_state,
                commit_candidate,
                action_cues,
                updated_at,
                delivery,
                objective_changed_at,
                objective_fingerprint,
            )
            .await;
    }
}

/// Wrapper that implements the synchronous `SessionProvider` trait by using
/// a dedicated thread to call async supervisor methods without panicking
/// from within the tokio runtime.
pub struct SupervisorProvider {
    supervisor: Arc<SessionSupervisor>,
    handle: tokio::runtime::Handle,
    persist_tx: mpsc::Sender<PersistThoughtRequest>,
    persist_overflow: Arc<ThoughtPersistOverflow>,
}

struct PersistThoughtRequest {
    session_id: String,
    thought: Option<String>,
    token_count: u64,
    context_limit: u64,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    rest_state: RestState,
    commit_candidate: bool,
    action_cues: Vec<ActionCue>,
    updated_at: DateTime<Utc>,
    delivery: ThoughtDeliveryState,
    objective_changed_at: Option<DateTime<Utc>>,
    objective_fingerprint: Option<String>,
}

struct ThoughtPersistOverflow {
    slots: StdMutex<HashMap<String, PersistThoughtRequest>>,
    notify: Notify,
}

impl Default for ThoughtPersistOverflow {
    fn default() -> Self {
        Self {
            slots: StdMutex::new(HashMap::new()),
            notify: Notify::new(),
        }
    }
}

impl ThoughtPersistOverflow {
    fn insert_latest(&self, req: PersistThoughtRequest) -> (bool, usize) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let replaced = slots.insert(req.session_id.clone(), req).is_some();
        (replaced, slots.len())
    }

    fn pop_next(&self) -> (Option<PersistThoughtRequest>, usize) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let next_key = slots.keys().next().cloned();
        let req = next_key.and_then(|key| slots.remove(&key));
        (req, slots.len())
    }
}

impl SupervisorProvider {
    pub fn new(supervisor: Arc<SessionSupervisor>) -> Self {
        Self::with_persist_queue_capacity(supervisor, THOUGHT_PERSIST_QUEUE_CAP)
    }

    fn with_persist_queue_capacity(
        supervisor: Arc<SessionSupervisor>,
        queue_capacity: usize,
    ) -> Self {
        let handle = tokio::runtime::Handle::current();
        supervisor.set_thought_persist_queue_capacity(queue_capacity);
        let (persist_tx, mut persist_rx) = mpsc::channel::<PersistThoughtRequest>(queue_capacity);
        let persist_supervisor = supervisor.clone();
        let persist_overflow = Arc::new(ThoughtPersistOverflow::default());
        let worker_overflow = persist_overflow.clone();
        handle.spawn(async move {
            loop {
                tokio::select! {
                    req = persist_rx.recv() => {
                        let Some(req) = req else {
                            drain_coalesced_persist_requests(&persist_supervisor, &worker_overflow).await;
                            break;
                        };
                        persist_thought_request(&persist_supervisor, req).await;
                        persist_supervisor.record_thought_persist_queue_depth(persist_rx.len());
                        drain_coalesced_persist_requests(&persist_supervisor, &worker_overflow).await;
                    }
                    _ = worker_overflow.notify.notified() => {
                        if persist_rx.is_empty() {
                            drain_coalesced_persist_requests(&persist_supervisor, &worker_overflow).await;
                        }
                    }
                }
            }
        });

        Self {
            supervisor,
            handle,
            persist_tx,
            persist_overflow,
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_persist_queue_capacity(
        supervisor: Arc<SessionSupervisor>,
        queue_capacity: usize,
    ) -> Self {
        Self::with_persist_queue_capacity(supervisor, queue_capacity)
    }

    fn record_queue_depth(&self) {
        let depth = self
            .persist_tx
            .max_capacity()
            .saturating_sub(self.persist_tx.capacity());
        self.supervisor.record_thought_persist_queue_depth(depth);
    }
}

async fn persist_thought_request(supervisor: &Arc<SessionSupervisor>, req: PersistThoughtRequest) {
    supervisor
        .persist_thought(
            &req.session_id,
            req.thought.as_deref(),
            req.token_count,
            req.context_limit,
            req.thought_state,
            req.thought_source,
            req.rest_state,
            req.commit_candidate,
            req.action_cues,
            req.updated_at,
            req.delivery,
            req.objective_changed_at,
            req.objective_fingerprint,
        )
        .await;
    supervisor.finish_pending_thought_persist();
}

async fn drain_coalesced_persist_requests(
    supervisor: &Arc<SessionSupervisor>,
    overflow: &Arc<ThoughtPersistOverflow>,
) {
    loop {
        let (req, slots) = overflow.pop_next();
        supervisor.set_thought_persist_overflow_slots(slots);
        let Some(req) = req else {
            return;
        };
        persist_thought_request(supervisor, req).await;
    }
}

impl SessionProvider for SupervisorProvider {
    async fn session_snapshots(&self) -> Vec<SessionInfo> {
        self.supervisor.collect_session_snapshots().await
    }

    fn persist_thought(
        &self,
        session_id: &str,
        thought: Option<&str>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        rest_state: RestState,
        commit_candidate: bool,
        action_cues: Vec<ActionCue>,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_changed_at: Option<DateTime<Utc>>,
        objective_fingerprint: Option<String>,
    ) -> bool {
        self.supervisor.begin_pending_thought_persist();
        let req = PersistThoughtRequest {
            session_id: session_id.to_string(),
            thought: thought.map(|value| value.to_string()),
            token_count,
            context_limit,
            thought_state,
            thought_source,
            rest_state,
            commit_candidate,
            action_cues,
            updated_at,
            delivery,
            objective_changed_at,
            objective_fingerprint,
        };

        match self.persist_tx.try_send(req) {
            Ok(()) => {
                self.record_queue_depth();
                true
            }
            Err(mpsc::error::TrySendError::Full(req)) => {
                self.supervisor
                    .record_thought_persist_queue_full(session_id);
                let (replaced, slots) = self.persist_overflow.insert_latest(req);
                self.supervisor.set_thought_persist_overflow_slots(slots);
                if replaced {
                    self.supervisor.record_thought_persist_coalesced(session_id);
                    self.supervisor.finish_pending_thought_persist();
                }
                self.persist_overflow.notify.notify_one();
                self.record_queue_depth();
                warn!(
                    session_id = %session_id,
                    overflow_slots = slots,
                    coalesced = replaced,
                    "persist_thought queue full; coalescing latest thought snapshot"
                );
                false
            }
            Err(mpsc::error::TrySendError::Closed(_req)) => {
                self.supervisor.finish_pending_thought_persist();
                self.supervisor.record_thought_persist_dropped(session_id);
                self.record_queue_depth();
                warn!(
                    session_id = %session_id,
                    "persist_thought queue closed; dropping thought snapshot"
                );
                false
            }
        }
    }

    fn thought_delivery_states(&self) -> HashMap<String, ThoughtDeliveryState> {
        let supervisor = self.supervisor.clone();
        let handle = self.handle.clone();
        std::thread::scope(|s| {
            let join = s
                .spawn(|| {
                    handle.block_on(async {
                        supervisor
                            .thought_snapshots
                            .read()
                            .await
                            .iter()
                            .map(|(session_id, snapshot)| {
                                (session_id.clone(), snapshot.delivery.clone())
                            })
                            .collect::<HashMap<_, _>>()
                    })
                })
                .join();
            // A panic inside the scoped thread used to crash the entire
            // thought-bridge runner via `.expect(..)`. Degrade gracefully:
            // log and return an empty map so the next tick can recover.
            join.unwrap_or_else(|_| {
                tracing::error!(
                    "thought_delivery_states snapshot thread panicked; returning empty map"
                );
                HashMap::new()
            })
        })
    }
}
