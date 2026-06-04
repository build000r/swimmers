use std::collections::HashMap;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::{ClientId, OutputFrame, ReplayPlan, SubscribeOutcome, SubscribeRejection};

pub(super) async fn replay_existing_frames(
    session_id: String,
    client_id: ClientId,
    client_tx: &mpsc::Sender<OutputFrame>,
    replay_plan: ReplayPlan,
) -> SubscribeOutcome {
    match replay_plan {
        ReplayPlan::None => SubscribeOutcome::Ok,
        ReplayPlan::Frames(frames) => {
            replay_buffered_frames(&session_id, client_id, client_tx, frames).await
        }
        ReplayPlan::Truncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        } => replay_truncated_outcome(
            &session_id,
            client_id,
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        ),
    }
}

pub(super) fn subscriber_cap_rejection(active_subscribers: usize) -> SubscribeRejection {
    SubscribeRejection {
        reason: format!("session already has {active_subscribers} active browser subscribers"),
    }
}

pub(super) fn subscribe_outcome_for_rejection(rejection: SubscribeRejection) -> SubscribeOutcome {
    SubscribeOutcome::Rejected {
        reason: rejection.reason,
    }
}

pub(super) fn retain_open_subscribers(
    subscribers: &mut HashMap<ClientId, mpsc::Sender<OutputFrame>>,
) {
    subscribers.retain(|_, tx| !tx.is_closed());
}

pub(super) fn apply_subscriber_cap(
    active_subscribers: usize,
    max_subscribers: usize,
) -> Result<(), SubscribeRejection> {
    (active_subscribers < max_subscribers)
        .then_some(())
        .ok_or_else(|| subscriber_cap_rejection(active_subscribers))
}

pub(super) fn attach_open_subscriber(
    subscribers: &mut HashMap<ClientId, mpsc::Sender<OutputFrame>>,
    client_id: ClientId,
    client_tx: mpsc::Sender<OutputFrame>,
) -> bool {
    let open = !client_tx.is_closed();
    if open {
        subscribers.insert(client_id, client_tx);
    }
    open
}

async fn replay_buffered_frames(
    session_id: &str,
    client_id: ClientId,
    client_tx: &mpsc::Sender<OutputFrame>,
    frames: Vec<(u64, Vec<u8>)>,
) -> SubscribeOutcome {
    if send_replay_frames(client_tx, frames).await.is_none() {
        warn!(
            session_id = %session_id,
            client_id,
            "subscriber dropped during replay"
        );
    }
    SubscribeOutcome::Ok
}

async fn send_replay_frames(
    client_tx: &mpsc::Sender<OutputFrame>,
    frames: Vec<(u64, Vec<u8>)>,
) -> Option<()> {
    for (seq, data) in frames {
        client_tx.send(OutputFrame { seq, data }).await.ok()?;
    }
    Some(())
}

fn replay_truncated_outcome(
    session_id: &str,
    client_id: ClientId,
    requested_resume_from_seq: u64,
    replay_window_start_seq: u64,
    latest_seq: u64,
) -> SubscribeOutcome {
    warn!(
        session_id = %session_id,
        client_id,
        requested_resume_from_seq,
        window_start = replay_window_start_seq,
        "replay truncated, client needs full refresh"
    );
    SubscribeOutcome::ReplayTruncated {
        requested_resume_from_seq,
        replay_window_start_seq,
        latest_seq,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BroadcastRemovalReason {
    Overloaded,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BroadcastSendOutcome {
    Delivered,
    Remove(BroadcastRemovalReason),
}

impl BroadcastSendOutcome {
    fn removal_reason(self) -> Option<BroadcastRemovalReason> {
        match self {
            Self::Delivered => None,
            Self::Remove(reason) => Some(reason),
        }
    }
}

fn classify_broadcast_send_error(
    err: mpsc::error::TrySendError<OutputFrame>,
) -> BroadcastSendOutcome {
    match err {
        mpsc::error::TrySendError::Full(_) => {
            BroadcastSendOutcome::Remove(BroadcastRemovalReason::Overloaded)
        }
        mpsc::error::TrySendError::Closed(_) => {
            BroadcastSendOutcome::Remove(BroadcastRemovalReason::Closed)
        }
    }
}

fn try_send_broadcast_frame(
    tx: &mpsc::Sender<OutputFrame>,
    frame: &OutputFrame,
) -> BroadcastSendOutcome {
    tx.try_send(frame.clone())
        .map(|()| BroadcastSendOutcome::Delivered)
        .unwrap_or_else(classify_broadcast_send_error)
}

pub(super) fn broadcast_removal_for_subscriber(
    session_id: &str,
    client_id: ClientId,
    tx: &mpsc::Sender<OutputFrame>,
    frame: &OutputFrame,
) -> Option<ClientId> {
    let reason = try_send_broadcast_frame(tx, frame).removal_reason()?;
    note_broadcast_removal(session_id, client_id, reason);
    Some(client_id)
}

fn note_broadcast_removal(session_id: &str, client_id: ClientId, reason: BroadcastRemovalReason) {
    match reason {
        BroadcastRemovalReason::Overloaded => note_overloaded_subscriber(session_id, client_id),
        BroadcastRemovalReason::Closed => note_closed_subscriber(session_id, client_id),
    }
}

fn note_overloaded_subscriber(session_id: &str, client_id: ClientId) {
    warn!(
        session_id = %session_id,
        client_id,
        "subscriber channel full (SESSION_OVERLOADED), dropping client"
    );
    crate::metrics::increment_overload(session_id);
}

fn note_closed_subscriber(session_id: &str, client_id: ClientId) {
    debug!(session_id = %session_id, client_id, "subscriber channel closed");
}

pub(super) fn remove_broadcast_subscribers(
    subscribers: &mut HashMap<ClientId, mpsc::Sender<OutputFrame>>,
    client_ids: Vec<ClientId>,
) {
    for id in client_ids {
        subscribers.remove(&id);
    }
}
