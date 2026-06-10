use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::Message;
use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::{
    ActorHandle, OutputFrame, ReplayCursor, SessionCommand, SubscribeOutcome,
};
use crate::session::supervisor::LifecycleEvent;
use crate::types::{opcodes, ControlEvent, SessionSummary};

use super::ws_auth::WsOutputMode;
use super::{WsReceiver, WsSender, REPLY_TIMEOUT};

static NEXT_WS_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct SessionWsStart {
    pub(super) client_id: u64,
    output_rx: mpsc::Receiver<OutputFrame>,
    session_events: broadcast::Receiver<ControlEvent>,
    thought_events: broadcast::Receiver<ControlEvent>,
    lifecycle_events: broadcast::Receiver<LifecycleEvent>,
    subscribe_outcome: SubscribeOutcome,
    ready_payload: serde_json::Value,
}

pub(super) async fn prepare_session_ws_start(
    state: &Arc<AppState>,
    handle: &ActorHandle,
    session_id: &str,
    auth: &AuthInfo,
    resume_from_seq: Option<u64>,
    output_mode: WsOutputMode,
) -> anyhow::Result<SessionWsStart> {
    let client_id = NEXT_WS_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let replay_cursor = request_replay_cursor(handle).await?;
    let requested_resume_from_seq =
        resume_from_seq.unwrap_or_else(|| replay_cursor.replay_window_start_seq.saturating_sub(1));
    let (output_rx, subscribe_outcome) =
        subscribe_to_output(state, handle, client_id, Some(requested_resume_from_seq)).await?;
    let session_events = handle.subscribe_events();
    let thought_events = state.supervisor.subscribe_thought_events();
    let lifecycle_events = state.supervisor.subscribe_events();
    let summary = fetch_live_summary(state, session_id).await?;
    let can_write = auth.has_scope(AuthScope::StreamWrite);

    let ready_payload = build_ready_payload(
        session_id,
        can_write,
        replay_cursor,
        requested_resume_from_seq,
        output_mode,
        &summary,
    );

    Ok(SessionWsStart {
        client_id,
        output_rx,
        session_events,
        thought_events,
        lifecycle_events,
        subscribe_outcome,
        ready_payload,
    })
}

pub(super) async fn send_session_ws_ready(
    sender: &mut WsSender,
    session: &SessionWsStart,
) -> anyhow::Result<bool> {
    sender
        .send(Message::Text(session.ready_payload.to_string().into()))
        .await?;

    if let Some((notice, should_close)) = subscribe_outcome_notice(&session.subscribe_outcome) {
        sender
            .send(Message::Text(notice.to_string().into()))
            .await?;
        return Ok(!should_close);
    }

    Ok(true)
}

pub(super) async fn run_session_ws_event_loop(
    handle: &ActorHandle,
    sender: &mut WsSender,
    receiver: &mut WsReceiver,
    auth: &AuthInfo,
    output_mode: WsOutputMode,
    session_id: &str,
    session: &mut SessionWsStart,
) -> anyhow::Result<()> {
    while continue_session_ws_event_loop(
        handle,
        sender,
        receiver,
        auth,
        output_mode,
        session_id,
        session,
    )
    .await?
    {}

    Ok(())
}

async fn continue_session_ws_event_loop(
    handle: &ActorHandle,
    sender: &mut WsSender,
    receiver: &mut WsReceiver,
    auth: &AuthInfo,
    output_mode: WsOutputMode,
    session_id: &str,
    session: &mut SessionWsStart,
) -> anyhow::Result<bool> {
    let Some(event) = next_session_ws_event(receiver, session).await else {
        return Ok(false);
    };
    handle_session_ws_event(handle, sender, auth, output_mode, session_id, event).await
}

async fn next_session_ws_event(
    receiver: &mut WsReceiver,
    session: &mut SessionWsStart,
) -> Option<SessionWsEvent> {
    let output_rx = &mut session.output_rx;
    let session_events = &mut session.session_events;
    let thought_events = &mut session.thought_events;
    let lifecycle_events = &mut session.lifecycle_events;

    tokio::select! {
        maybe_message = receiver.next() => maybe_message.map(SessionWsEvent::Incoming),
        maybe_frame = output_rx.recv() => maybe_frame.map(SessionWsEvent::Frame),
        event = session_events.recv() => Some(SessionWsEvent::SessionControl(event)),
        event = thought_events.recv() => Some(SessionWsEvent::ThoughtControl(event)),
        event = lifecycle_events.recv() => Some(SessionWsEvent::Lifecycle(Box::new(event))),
    }
}

/// Build the `ready` handshake payload sent immediately after a client
/// authenticates. Pure; no I/O. `readOnly` mirrors the absence of write scope
/// and `protocol.output` reflects the negotiated output mode.
pub(super) fn build_ready_payload(
    session_id: &str,
    can_write: bool,
    replay_cursor: ReplayCursor,
    requested_resume_from_seq: u64,
    output_mode: WsOutputMode,
    summary: &Option<SessionSummary>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "ready",
        "sessionId": session_id,
        "readOnly": !can_write,
        "replay": {
            "latestSeq": replay_cursor.latest_seq,
            "windowStartSeq": replay_cursor.replay_window_start_seq,
            "resumeFromSeq": requested_resume_from_seq,
        },
        "protocol": {
            "output": output_mode.protocol_output(),
        },
        "summary": summary,
    })
}

/// Pure mapping from a subscribe outcome to the notice payload to send (if any)
/// and whether the connection should close after sending it. No I/O. `Ok` sends
/// nothing; `Rejected` emits an overloaded notice and closes; `ReplayTruncated`
/// emits a notice and keeps streaming.
pub(super) fn subscribe_outcome_notice(
    outcome: &SubscribeOutcome,
) -> Option<(serde_json::Value, bool)> {
    match outcome {
        SubscribeOutcome::Ok => None,
        SubscribeOutcome::Rejected { reason } => Some((
            serde_json::json!({
                "type": "overloaded",
                "code": "SESSION_OVERLOADED",
                "message": reason,
                "retryAfterMs": 4000,
            }),
            true,
        )),
        SubscribeOutcome::ReplayTruncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        } => Some((
            serde_json::json!({
                "type": "replay_truncated",
                "requestedResumeFromSeq": requested_resume_from_seq,
                "windowStartSeq": replay_window_start_seq,
                "latestSeq": latest_seq,
            }),
            false,
        )),
    }
}

enum SessionWsEvent {
    Incoming(Result<Message, axum::Error>),
    Frame(OutputFrame),
    SessionControl(Result<ControlEvent, broadcast::error::RecvError>),
    ThoughtControl(Result<ControlEvent, broadcast::error::RecvError>),
    Lifecycle(Box<Result<LifecycleEvent, broadcast::error::RecvError>>),
}

async fn handle_session_ws_event(
    handle: &ActorHandle,
    sender: &mut WsSender,
    auth: &AuthInfo,
    output_mode: WsOutputMode,
    session_id: &str,
    event: SessionWsEvent,
) -> anyhow::Result<bool> {
    match event {
        SessionWsEvent::Incoming(Ok(message)) => {
            super::handle_client_message(handle, sender, auth, message).await
        }
        SessionWsEvent::Incoming(Err(err)) => Err(err.into()),
        SessionWsEvent::Frame(frame) => {
            send_output_frame(sender, frame, output_mode).await?;
            Ok(true)
        }
        SessionWsEvent::SessionControl(event) => {
            send_control_event_if_relevant(sender, session_id, "session_events", event).await
        }
        SessionWsEvent::ThoughtControl(event) => {
            send_control_event_if_relevant(sender, session_id, "thought_events", event).await
        }
        SessionWsEvent::Lifecycle(event) => {
            send_lifecycle_event_if_relevant(sender, session_id, *event).await
        }
    }
}

async fn send_control_event_if_relevant(
    sender: &mut WsSender,
    session_id: &str,
    stream: &'static str,
    event: Result<ControlEvent, broadcast::error::RecvError>,
) -> anyhow::Result<bool> {
    send_ws_json_if_some(
        sender,
        control_event_delivery_payload(session_id, stream, &event),
    )
    .await?;
    Ok(true)
}

async fn send_lifecycle_event_if_relevant(
    sender: &mut WsSender,
    session_id: &str,
    event: Result<LifecycleEvent, broadcast::error::RecvError>,
) -> anyhow::Result<bool> {
    send_ws_json_if_some(sender, lifecycle_event_delivery_payload(session_id, &event)).await?;
    Ok(true)
}

pub(super) fn control_event_delivery_payload(
    session_id: &str,
    stream: &str,
    event: &Result<ControlEvent, broadcast::error::RecvError>,
) -> Option<serde_json::Value> {
    match event {
        Ok(event) => matching_control_event_payload(session_id, event),
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            Some(event_stream_lagged_payload(stream, *skipped))
        }
        Err(broadcast::error::RecvError::Closed) => None,
    }
}

fn matching_control_event_payload(
    session_id: &str,
    event: &ControlEvent,
) -> Option<serde_json::Value> {
    Some(event)
        .filter(|event| event.session_id == session_id)
        .map(control_event_ws_payload)
}

pub(super) fn lifecycle_event_delivery_payload(
    session_id: &str,
    event: &Result<LifecycleEvent, broadcast::error::RecvError>,
) -> Option<serde_json::Value> {
    match event {
        Ok(event) => matching_lifecycle_event_payload(session_id, event),
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            Some(event_stream_lagged_payload("lifecycle_events", *skipped))
        }
        Err(broadcast::error::RecvError::Closed) => None,
    }
}

fn matching_lifecycle_event_payload(
    session_id: &str,
    event: &LifecycleEvent,
) -> Option<serde_json::Value> {
    Some(event)
        .filter(|event| lifecycle_event_session_id(event) == session_id)
        .map(lifecycle_event_ws_payload)
}

async fn send_ws_json_if_some(
    sender: &mut WsSender,
    payload: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    match payload {
        Some(payload) => send_ws_json(sender, payload).await,
        None => Ok(()),
    }
}

async fn send_ws_json(sender: &mut WsSender, payload: serde_json::Value) -> anyhow::Result<()> {
    sender
        .send(Message::Text(payload.to_string().into()))
        .await?;
    Ok(())
}

pub(super) fn control_event_ws_payload(event: &ControlEvent) -> serde_json::Value {
    let contract = event.payload_contract();
    serde_json::json!({
        "type": "control_event",
        "event": contract.event_name(),
        "sessionId": event.session_id,
        "payload": contract.payload_value(),
    })
}

pub(super) fn lifecycle_event_session_id(event: &LifecycleEvent) -> &str {
    match event {
        LifecycleEvent::Created { session_id, .. } | LifecycleEvent::Deleted { session_id, .. } => {
            session_id
        }
    }
}

pub(super) fn lifecycle_event_ws_payload(event: &LifecycleEvent) -> serde_json::Value {
    match event {
        LifecycleEvent::Created {
            session_id,
            summary,
            reason,
            repo_theme,
        } => serde_json::json!({
            "type": "lifecycle_event",
            "event": "session_created",
            "sessionId": session_id,
            "reason": reason,
            "summary": summary,
            "repoTheme": repo_theme,
        }),
        LifecycleEvent::Deleted {
            session_id,
            reason,
            delete_mode,
            tmux_session_alive,
        } => serde_json::json!({
            "type": "lifecycle_event",
            "event": "session_deleted",
            "sessionId": session_id,
            "reason": reason,
            "deleteMode": delete_mode,
            "tmuxSessionAlive": tmux_session_alive,
        }),
    }
}

pub(super) fn event_stream_lagged_payload(stream: &str, skipped: u64) -> serde_json::Value {
    serde_json::json!({
        "type": "event_stream_lagged",
        "stream": stream,
        "skipped": skipped,
    })
}

async fn send_output_frame(
    sender: &mut WsSender,
    frame: OutputFrame,
    output_mode: WsOutputMode,
) -> anyhow::Result<()> {
    match output_mode {
        WsOutputMode::Raw => {
            sender.send(Message::Binary(frame.data.into())).await?;
        }
        WsOutputMode::Framed => {
            sender
                .send(Message::Binary(encode_terminal_output_frame(frame).into()))
                .await?;
        }
    }
    Ok(())
}

pub(super) fn encode_terminal_output_frame(frame: OutputFrame) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + std::mem::size_of::<u64>() + frame.data.len());
    bytes.push(opcodes::TERMINAL_OUTPUT);
    bytes.extend_from_slice(&frame.seq.to_be_bytes());
    bytes.extend_from_slice(&frame.data);
    bytes
}

async fn subscribe_to_output(
    state: &Arc<AppState>,
    handle: &ActorHandle,
    client_id: u64,
    resume_from_seq: Option<u64>,
) -> anyhow::Result<(mpsc::Receiver<OutputFrame>, SubscribeOutcome)> {
    let (client_tx, client_rx) = mpsc::channel(state.config.outbound_queue_bound.max(64));
    let (ack_tx, ack_rx) = oneshot::channel();
    handle
        .send(SessionCommand::Subscribe {
            client_id,
            client_tx,
            resume_from_seq,
            ack: ack_tx,
        })
        .await
        .map_err(|err| anyhow::anyhow!("failed to subscribe to session output: {err}"))?;

    let outcome = tokio::time::timeout(REPLY_TIMEOUT, ack_rx)
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for subscribe ack"))?
        .map_err(|_| anyhow::anyhow!("session actor dropped subscribe ack"))?;

    Ok((client_rx, outcome))
}

async fn request_replay_cursor(handle: &ActorHandle) -> anyhow::Result<ReplayCursor> {
    let (tx, rx) = oneshot::channel();
    handle
        .send(SessionCommand::GetReplayCursor(tx))
        .await
        .map_err(|err| anyhow::anyhow!("failed to request replay cursor: {err}"))?;

    tokio::time::timeout(REPLY_TIMEOUT, rx)
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for replay cursor"))?
        .map_err(|_| anyhow::anyhow!("session actor dropped replay cursor"))
}
