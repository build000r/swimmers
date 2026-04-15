//! Compatibility shim for the retired in-process thought engine.
//!
//! `swimmers` now uses the external `clawgs emit --stdio` daemon as the
//! thought engine boundary. This module preserves the session snapshot contract
//! shared by bridge/client code and keeps a temporary compatibility runner for
//! `SWIMMERS_THOUGHT_BACKEND=inproc`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, RwLock};
use tracing::warn;

use crate::thought::bridge_runner::BridgeRunner;
use crate::thought::emitter_client::EmitterClient;
use crate::thought::health::BridgeHealthState;
use crate::thought::protocol::{SyncRequestSequence, ThoughtDeliveryState};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{ControlEvent, RestState, SessionState, ThoughtSource, ThoughtState};

/// Snapshot of a single session's data, provided by the supervisor each tick.
#[derive(Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub state: SessionState,
    pub exited: bool,
    /// The detected coding tool name (e.g. "Claude Code", "Codex"), if any.
    pub tool: Option<String>,
    /// Working directory of the session.
    pub cwd: String,
    /// Last visible terminal text from the replay buffer.
    pub replay_text: String,
    /// Current persisted thought text from summary snapshot.
    pub thought: Option<String>,
    /// Current persisted thought lifecycle state.
    pub thought_state: ThoughtState,
    /// Current persisted thought source.
    pub thought_source: ThoughtSource,
    /// Current daemon-authored rest state.
    pub rest_state: RestState,
    /// Passive commit reminder state emitted by clawgs.
    pub commit_candidate: bool,
    /// Last seen objective fingerprint used to avoid noisy rewrites.
    pub objective_fingerprint: Option<String>,
    /// Time of last persisted thought update.
    pub thought_updated_at: Option<DateTime<Utc>>,
    /// Token count from the session summary.
    pub token_count: u64,
    /// Context limit from the session summary.
    pub context_limit: u64,
    /// Last observed terminal activity timestamp.
    pub last_activity_at: DateTime<Utc>,
}

/// Trait abstracting the supervisor so thought runners are testable in
/// isolation.
pub trait SessionProvider: Send + Sync {
    /// Return info for every tracked session.
    ///
    /// Must be async: the supervisor implementation awaits RwLocks and
    /// session-actor mailboxes. An earlier sync version wrapped an
    /// `std::thread::scope(|s| s.spawn(|| handle.block_on(...)).join())`
    /// which blocked the calling Tokio worker and, when the I/O driver
    /// happened to ride on that worker, stalled the entire reactor. Making
    /// this method async lets callers `.await` it without migrating threads.
    fn session_snapshots(&self) -> impl std::future::Future<Output = Vec<SessionInfo>> + Send;

    /// Persist the latest thought snapshot for a session.
    fn persist_thought(
        &self,
        _session_id: &str,
        _thought: Option<&str>,
        _token_count: u64,
        _context_limit: u64,
        _thought_state: ThoughtState,
        _thought_source: ThoughtSource,
        _rest_state: RestState,
        _commit_candidate: bool,
        _updated_at: DateTime<Utc>,
        _delivery: ThoughtDeliveryState,
        _objective_changed_at: Option<DateTime<Utc>>,
        _objective_fingerprint: Option<String>,
    ) {
    }

    /// Return the last accepted stream/sequence watermark for each session.
    fn thought_delivery_states(&self) -> std::collections::HashMap<String, ThoughtDeliveryState> {
        std::collections::HashMap::new()
    }
}

/// Temporary compatibility runner that delegates to the daemon bridge.
pub struct ThoughtLoopRunner {
    event_tx: broadcast::Sender<ControlEvent>,
    runtime_config: Arc<RwLock<ThoughtConfig>>,
    request_sequence: Arc<SyncRequestSequence>,
    bridge_health: Arc<BridgeHealthState>,
}

impl ThoughtLoopRunner {
    pub fn with_runtime_config(
        tick_ms: u64,
        event_tx: broadcast::Sender<ControlEvent>,
        runtime_config: Arc<RwLock<ThoughtConfig>>,
        request_sequence: Arc<SyncRequestSequence>,
    ) -> Self {
        Self {
            event_tx,
            runtime_config,
            request_sequence,
            bridge_health: Arc::new(BridgeHealthState::new_with_tick(Duration::from_millis(
                tick_ms,
            ))),
        }
    }

    pub fn bridge_health(&self) -> Arc<BridgeHealthState> {
        self.bridge_health.clone()
    }

    /// Start the compatibility shim as a detached task.
    pub fn spawn<P: SessionProvider + 'static>(
        self,
        provider: Arc<P>,
    ) -> tokio::task::JoinHandle<()> {
        warn!("legacy inproc thought backend selected; delegating to clawgs daemon bridge");

        BridgeRunner::with_existing_health(self.event_tx, self.runtime_config, self.bridge_health)
            .spawn(
                provider,
                EmitterClient::with_request_sequence(self.request_sequence),
            )
    }
}
