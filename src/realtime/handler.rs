//! WebSocket handler for the `/v1/realtime` endpoint.
//!
//! Multiplexes binary terminal I/O frames and JSON control messages over a
//! single WebSocket connection. Uses `tokio::select!` to concurrently read
//! from the client, forward supervisor lifecycle events, deliver per-session
//! terminal output, per-session control events, and thought updates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::AppState;
use crate::realtime::codec;
use crate::session::actor::{
    ActorHandle, ClientId, OutputFrame, ReplayCursor, SessionCommand, SubscribeOutcome,
};
use crate::session::supervisor::LifecycleEvent;
use crate::types::{
    ClientControlMessage, ControlErrorPayload, ControlEvent, DismissAttentionPayload,
    ReplayTruncatedPayload, ResizePayload, SessionCreatedPayload, SessionDeletedPayload,
    SessionOverloadedPayload, SessionSubscriptionPayload, SubscribeSessionPayload,
    UnsubscribeSessionPayload,
};

/// Global client ID counter for assigning unique IDs to output subscriptions.
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// Global connected-client counter for the connected_clients metric gauge.
static CONNECTED_CLIENTS_COUNT: AtomicUsize = AtomicUsize::new(0);

fn allocate_client_id() -> ClientId {
    NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Build the router for the `/v1/realtime` WebSocket endpoint.
///
/// This is designed to be nested: `Router::new().nest("/v1/realtime", ws_router())`.
pub fn ws_router() -> axum::Router<Arc<AppState>> {
    use axum::routing::get;
    axum::Router::new().route("/", get(ws_upgrade))
}

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-session subscription state held by this WebSocket connection.
struct SessionSub {
    client_id: ClientId,
    output_rx: mpsc::Receiver<OutputFrame>,
    /// Per-session control event receiver (session_state, session_title).
    event_rx: broadcast::Receiver<ControlEvent>,
}

enum PollEvent {
    Output {
        session_id: String,
        frame: OutputFrame,
    },
    Overloaded {
        session_id: String,
    },
    /// A per-session control event (session_state, session_title).
    SessionEvent(ControlEvent),
}

/// Main WebSocket connection loop.
///
/// Responsibilities:
/// 1. Read binary frames from the client and route terminal input to the
///    correct session actor.
/// 2. Read JSON text messages from the client and handle control commands
///    (subscribe_session, resize, dismiss_attention).
/// 3. Forward broadcast LifecycleEvents from the supervisor to the client
///    as JSON ControlEvents.
/// 4. Forward per-session terminal output to the client as binary frames.
/// 5. Forward per-session control events (session_state) to the client.
/// 6. Forward thought_update events from the thought loop to the client.
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Subscribe to supervisor-level lifecycle events (session created/deleted).
    let mut lifecycle_rx: broadcast::Receiver<LifecycleEvent> = state.supervisor.subscribe_events();

    // Subscribe to thought_update events from the thought loop.
    let mut thought_rx: broadcast::Receiver<ControlEvent> =
        state.supervisor.subscribe_thought_events();

    // Track per-session output subscriptions: session_id -> sub state.
    let mut output_subs: HashMap<String, SessionSub> = HashMap::new();

    tracing::info!("realtime client connected");
    let client_count = CONNECTED_CLIENTS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    crate::metrics::set_connected_clients(client_count);

    loop {
        // Build a future that resolves when any subscribed session has output
        // or a per-session control event.
        let output_fut = poll_output_subs(&mut output_subs);

        tokio::select! {
            // --- Client -> Server messages ---
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        crate::metrics::increment_frames_received();
                        handle_binary_frame(&data, &state, &mut ws_sink).await;
                    }
                    Some(Ok(Message::Text(text))) => {
                        handle_text_message(
                            &text,
                            &state,
                            &mut ws_sink,
                            &mut output_subs,
                        )
                        .await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!("realtime client disconnected");
                        break;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = ws_sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Ignore unsolicited pongs.
                    }
                    Some(Err(e)) => {
                        tracing::warn!("websocket receive error: {e}");
                        break;
                    }
                }
            }

            // --- Supervisor lifecycle events (session created/deleted) ---
            event = lifecycle_rx.recv() => {
                match event {
                    Ok(lifecycle_event) => {
                        let control_event = lifecycle_to_control_event(lifecycle_event);
                        let json = serde_json::to_string(&control_event)
                            .expect("ControlEvent must serialize");
                        if ws_sink.send(Message::Text(json.into())).await.is_err() {
                            tracing::warn!("failed to send lifecycle event to client");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("realtime client lagged by {n} lifecycle events");
                        // Continue -- client may miss some events but the
                        // connection is still usable.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("supervisor lifecycle broadcast closed");
                        break;
                    }
                }
            }

            // --- Thought update events from the thought loop ---
            event = thought_rx.recv() => {
                match event {
                    Ok(thought_event) => {
                        let json = serde_json::to_string(&thought_event)
                            .expect("ControlEvent must serialize");
                        if ws_sink.send(Message::Text(json.into())).await.is_err() {
                            tracing::warn!("failed to send thought_update event to client");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("realtime client lagged by {n} thought events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("thought broadcast closed");
                        // Don't break -- thought loop stopping is not fatal.
                    }
                }
            }

            // --- Per-session terminal output and control events ---
            Some(event) = output_fut => {
                match event {
                    PollEvent::Output { session_id, frame } => {
                        let binary = codec::encode_output_frame(&session_id, frame.seq, &frame.data);
                        crate::metrics::increment_frames_sent();
                        if ws_sink.send(Message::Binary(binary.into())).await.is_err() {
                            tracing::warn!("failed to send output frame to client");
                            break;
                        }
                    }
                    PollEvent::Overloaded { session_id } => {
                        crate::metrics::increment_overload(&session_id);
                        // Only emit overload when the session still exists.
                        if state.supervisor.get_session(&session_id).await.is_some() {
                            let event = ControlEvent {
                                event: "session_overloaded".to_string(),
                                session_id: session_id.clone(),
                                payload: serde_json::to_value(SessionOverloadedPayload {
                                    code: "SESSION_OVERLOADED".to_string(),
                                    queue_depth: state.config.outbound_queue_bound,
                                    queue_bytes: 0,
                                    retry_after_ms: 250,
                                })
                                .unwrap(),
                            };
                            if ws_sink
                                .send(Message::Text(
                                    serde_json::to_string(&event).unwrap().into(),
                                ))
                                .await
                                .is_err()
                            {
                                tracing::warn!("failed to send session_overloaded event");
                                break;
                            }
                        }
                    }
                    PollEvent::SessionEvent(control_event) => {
                        let json = serde_json::to_string(&control_event)
                            .expect("ControlEvent must serialize");
                        if ws_sink.send(Message::Text(json.into())).await.is_err() {
                            tracing::warn!("failed to send session event to client");
                            break;
                        }
                    }
                }
            }
        }
    }

    let client_count = CONNECTED_CLIENTS_COUNT.fetch_sub(1, Ordering::Relaxed) - 1;
    crate::metrics::set_connected_clients(client_count);

    // On disconnect, send Unsubscribe commands to each session actor so
    // it can clean up the subscriber entry.
    for (session_id, sub) in output_subs.drain() {
        if let Some(handle) = state.supervisor.get_session(&session_id).await {
            let _ = handle
                .send(SessionCommand::Unsubscribe {
                    client_id: sub.client_id,
                })
                .await;
        }
        drop(sub.output_rx);
    }

    tracing::info!("realtime client cleanup complete");
}

// ---------------------------------------------------------------------------
// Lifecycle -> ControlEvent conversion
// ---------------------------------------------------------------------------

fn lifecycle_to_control_event(event: LifecycleEvent) -> ControlEvent {
    match event {
        LifecycleEvent::Created {
            session_id,
            summary,
            reason,
        } => ControlEvent {
            event: "session_created".to_string(),
            session_id,
            payload: serde_json::to_value(SessionCreatedPayload {
                reason,
                session: summary,
            })
            .unwrap(),
        },
        LifecycleEvent::Deleted {
            session_id,
            reason,
            delete_mode,
            tmux_session_alive,
        } => ControlEvent {
            event: "session_deleted".to_string(),
            session_id,
            payload: serde_json::to_value(SessionDeletedPayload {
                reason,
                delete_mode: delete_mode_to_wire(&delete_mode).to_string(),
                tmux_session_alive,
                at: Utc::now(),
            })
            .unwrap(),
        },
    }
}

fn delete_mode_to_wire(mode: &crate::config::SessionDeleteMode) -> &'static str {
    match mode {
        crate::config::SessionDeleteMode::DetachBridge => "detach_bridge",
        crate::config::SessionDeleteMode::KillTmux => "kill_tmux",
    }
}

// ---------------------------------------------------------------------------
// Binary frame handling
// ---------------------------------------------------------------------------

async fn handle_binary_frame(
    data: &[u8],
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) {
    let dispatch_start = std::time::Instant::now();
    match codec::decode_input_frame(data) {
        Ok((session_id, input_bytes)) => {
            let handle = match state.supervisor.get_session(&session_id).await {
                Some(h) => h,
                None => {
                    tracing::warn!("[session {session_id}] input for unknown session");
                    send_control_error(
                        ws_sink,
                        &session_id,
                        "SESSION_NOT_FOUND",
                        &format!("session not found: {session_id}"),
                        None,
                    )
                    .await;
                    return;
                }
            };

            if let Err(e) = handle.send(SessionCommand::WriteInput(input_bytes)).await {
                tracing::warn!("[session {session_id}] input write failed: {e}");
                send_control_error(
                    ws_sink,
                    &session_id,
                    "SESSION_NOT_FOUND",
                    &e.to_string(),
                    None,
                )
                .await;
            } else {
                let elapsed = dispatch_start.elapsed();
                crate::metrics::record_dispatch_latency(&session_id, elapsed);
                crate::metrics::record_keystroke_echo(&session_id, elapsed);
            }
        }
        Err(e) => {
            tracing::warn!("failed to decode binary frame: {e}");
            send_control_error(
                ws_sink,
                "",
                "VALIDATION_FAILED",
                &format!("invalid binary frame: {e}"),
                None,
            )
            .await;
        }
    }
}

// ---------------------------------------------------------------------------
// JSON control message handling
// ---------------------------------------------------------------------------

async fn handle_text_message(
    text: &str,
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    output_subs: &mut HashMap<String, SessionSub>,
) {
    let msg: ClientControlMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("invalid JSON control message: {e}");
            send_control_error(ws_sink, "", "VALIDATION_FAILED", &e.to_string(), None).await;
            return;
        }
    };

    let request_id = msg.request_id.clone();

    match msg.msg_type.as_str() {
        "subscribe_session" => {
            handle_subscribe(state, ws_sink, output_subs, &msg.payload, &request_id).await;
        }
        "unsubscribe_session" => {
            handle_unsubscribe(state, ws_sink, output_subs, &msg.payload, &request_id).await;
        }
        "resize" => {
            handle_resize(state, ws_sink, &msg.payload, &request_id).await;
        }
        "dismiss_attention" => {
            handle_dismiss(state, ws_sink, &msg.payload, &request_id).await;
        }
        unknown => {
            tracing::warn!("unknown control message type: {unknown}");
            send_control_error(
                ws_sink,
                "",
                "VALIDATION_FAILED",
                &format!("unknown message type: {unknown}"),
                request_id.as_deref(),
            )
            .await;
        }
    }
}

async fn handle_subscribe(
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    output_subs: &mut HashMap<String, SessionSub>,
    payload: &serde_json::Value,
    request_id: &Option<String>,
) {
    let sub_payload: SubscribeSessionPayload = match serde_json::from_value(payload.clone()) {
        Ok(s) => s,
        Err(e) => {
            send_control_error(
                ws_sink,
                "",
                "VALIDATION_FAILED",
                &e.to_string(),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    let session_id = sub_payload.session_id.clone();

    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            send_control_error(
                ws_sink,
                &session_id,
                "SESSION_NOT_FOUND",
                &format!("session not found: {session_id}"),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    // If already subscribed to this session, unsubscribe first.
    if let Some(old_sub) = output_subs.remove(&session_id) {
        crate::metrics::increment_subscription_lifecycle(&session_id, "unsubscribe");
        let _ = handle
            .send(SessionCommand::Unsubscribe {
                client_id: old_sub.client_id,
            })
            .await;
    }

    // Create a bounded channel for this subscription's output.
    let client_id = allocate_client_id();
    let (client_tx, client_rx) = mpsc::channel::<OutputFrame>(state.config.outbound_queue_bound);

    // Subscribe to per-session control events (session_state, session_title).
    let event_rx = handle.subscribe_events();

    // Send the Subscribe command to the session actor. The actor handles
    // replay internally and sends replayed frames through the channel.
    let (ack_tx, ack_rx) = oneshot::channel();
    if let Err(e) = handle
        .send(SessionCommand::Subscribe {
            client_id,
            client_tx,
            resume_from_seq: sub_payload.resume_from_seq,
            ack: ack_tx,
        })
        .await
    {
        tracing::warn!("[session {session_id}] subscribe command failed: {e}");
        send_control_error(
            ws_sink,
            &session_id,
            "SESSION_NOT_FOUND",
            &e.to_string(),
            request_id.as_deref(),
        )
        .await;
        return;
    }

    // Store the subscription.
    output_subs.insert(
        session_id.clone(),
        SessionSub {
            client_id,
            output_rx: client_rx,
            event_rx,
        },
    );

    let mut replay_truncated_payload: Option<ReplayTruncatedPayload> = None;
    match tokio::time::timeout(Duration::from_secs(2), ack_rx).await {
        Ok(Ok(SubscribeOutcome::Ok)) => {}
        Ok(Ok(SubscribeOutcome::ReplayTruncated {
            requested_resume_from_seq,
            replay_window_start_seq,
            latest_seq,
        })) => {
            crate::metrics::increment_replay_truncation(&session_id);
            replay_truncated_payload = Some(ReplayTruncatedPayload {
                code: "REPLAY_TRUNCATED".to_string(),
                requested_resume_from_seq,
                replay_window_start_seq,
                latest_seq,
            });
        }
        Ok(Err(_)) => {
            tracing::warn!("[session {session_id}] subscribe ack channel dropped");
        }
        Err(_) => {
            tracing::warn!("[session {session_id}] subscribe ack timed out");
        }
    }

    let cursor = fetch_replay_cursor(&handle).await.unwrap_or(ReplayCursor {
        latest_seq: 0,
        replay_window_start_seq: 0,
    });

    emit_session_subscription(
        ws_sink,
        &session_id,
        "subscribed",
        sub_payload.resume_from_seq,
        cursor,
    )
    .await;
    crate::metrics::increment_subscription_lifecycle(&session_id, "subscribe");

    if let Some(payload) = replay_truncated_payload {
        let event = ControlEvent {
            event: "replay_truncated".to_string(),
            session_id: session_id.clone(),
            payload: serde_json::to_value(payload).unwrap(),
        };
        send_control_event(ws_sink, &event).await;
    }

    tracing::info!(
        "[session {session_id}] client {client_id} subscribed (resume_from_seq={:?})",
        sub_payload.resume_from_seq
    );
}

async fn handle_unsubscribe(
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    output_subs: &mut HashMap<String, SessionSub>,
    payload: &serde_json::Value,
    request_id: &Option<String>,
) {
    let unsub_payload: UnsubscribeSessionPayload = match serde_json::from_value(payload.clone()) {
        Ok(s) => s,
        Err(e) => {
            send_control_error(
                ws_sink,
                "",
                "VALIDATION_FAILED",
                &e.to_string(),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    let session_id = unsub_payload.session_id;
    let removed = output_subs.remove(&session_id);

    if let Some(sub) = removed {
        if let Some(handle) = state.supervisor.get_session(&session_id).await {
            let _ = handle
                .send(SessionCommand::Unsubscribe {
                    client_id: sub.client_id,
                })
                .await;
        }
        crate::metrics::increment_subscription_lifecycle(&session_id, "unsubscribe");
    } else {
        crate::metrics::increment_subscription_lifecycle(&session_id, "idempotent_unsubscribe");
    }

    let cursor = match state.supervisor.get_session(&session_id).await {
        Some(handle) => fetch_replay_cursor(&handle).await.unwrap_or(ReplayCursor {
            latest_seq: 0,
            replay_window_start_seq: 0,
        }),
        None => ReplayCursor {
            latest_seq: 0,
            replay_window_start_seq: 0,
        },
    };

    emit_session_subscription(ws_sink, &session_id, "unsubscribed", None, cursor).await;
}

async fn handle_resize(
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    payload: &serde_json::Value,
    request_id: &Option<String>,
) {
    let resize: ResizePayload = match serde_json::from_value(payload.clone()) {
        Ok(r) => r,
        Err(e) => {
            send_control_error(
                ws_sink,
                "",
                "VALIDATION_FAILED",
                &e.to_string(),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    let handle = match state.supervisor.get_session(&resize.session_id).await {
        Some(h) => h,
        None => {
            send_control_error(
                ws_sink,
                &resize.session_id,
                "SESSION_NOT_FOUND",
                &format!("session not found: {}", resize.session_id),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    if let Err(e) = handle
        .send(SessionCommand::Resize {
            cols: resize.cols,
            rows: resize.rows,
        })
        .await
    {
        tracing::warn!("[session {}] resize failed: {e}", resize.session_id);
        send_control_error(
            ws_sink,
            &resize.session_id,
            "SESSION_NOT_FOUND",
            &e.to_string(),
            request_id.as_deref(),
        )
        .await;
    }
}

async fn handle_dismiss(
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    payload: &serde_json::Value,
    request_id: &Option<String>,
) {
    let dismiss: DismissAttentionPayload = match serde_json::from_value(payload.clone()) {
        Ok(d) => d,
        Err(e) => {
            send_control_error(
                ws_sink,
                "",
                "VALIDATION_FAILED",
                &e.to_string(),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    let handle = match state.supervisor.get_session(&dismiss.session_id).await {
        Some(h) => h,
        None => {
            send_control_error(
                ws_sink,
                &dismiss.session_id,
                "SESSION_NOT_FOUND",
                &format!("session not found: {}", dismiss.session_id),
                request_id.as_deref(),
            )
            .await;
            return;
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::warn!(
            "[session {}] dismiss_attention failed: {e}",
            dismiss.session_id
        );
        send_control_error(
            ws_sink,
            &dismiss.session_id,
            "SESSION_NOT_FOUND",
            &e.to_string(),
            request_id.as_deref(),
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Poll all per-session output receivers and event receivers.
/// Returns the first ready frame, overload notification, or session event.
///
/// Returns `None` (via `pending`) if there are no active subscriptions, which
/// causes the `tokio::select!` branch to be disabled.
async fn poll_output_subs(subs: &mut HashMap<String, SessionSub>) -> Option<PollEvent> {
    if subs.is_empty() {
        // Return a future that never resolves so the select branch is inactive.
        return std::future::pending().await;
    }

    loop {
        // Non-blocking sweep across all subscriptions prevents a quiet session
        // from starving peers that are actively producing output.
        let keys: Vec<String> = subs.keys().cloned().collect();
        for key in &keys {
            if let Some(sub) = subs.get_mut(key) {
                // Check for terminal output first.
                match sub.output_rx.try_recv() {
                    Ok(frame) => {
                        return Some(PollEvent::Output {
                            session_id: key.clone(),
                            frame,
                        });
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        tracing::info!("[session {key}] output channel closed, unsubscribing");
                        subs.remove(key);
                        return Some(PollEvent::Overloaded {
                            session_id: key.clone(),
                        });
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                }

                // Check for per-session control events.
                match sub.event_rx.try_recv() {
                    Ok(event) => {
                        return Some(PollEvent::SessionEvent(event));
                    }
                    Err(broadcast::error::TryRecvError::Lagged(n)) => {
                        tracing::warn!("[session {key}] lagged by {n} session events");
                    }
                    Err(broadcast::error::TryRecvError::Closed) => {
                        // Actor shut down -- output channel will close soon.
                    }
                    Err(broadcast::error::TryRecvError::Empty) => {}
                }
            }
        }

        if subs.is_empty() {
            return std::future::pending().await;
        }

        // Nothing ready yet; yield briefly before the next fair sweep.
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn fetch_replay_cursor(handle: &ActorHandle) -> Option<ReplayCursor> {
    let (cursor_tx, cursor_rx) = oneshot::channel();
    if handle
        .send(SessionCommand::GetReplayCursor(cursor_tx))
        .await
        .is_err()
    {
        return None;
    }

    match tokio::time::timeout(Duration::from_secs(1), cursor_rx).await {
        Ok(Ok(cursor)) => Some(cursor),
        Ok(Err(_)) => None,
        Err(_) => None,
    }
}

async fn emit_session_subscription(
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    session_id: &str,
    state: &str,
    resume_from_seq: Option<u64>,
    cursor: ReplayCursor,
) {
    let event = ControlEvent {
        event: "session_subscription".to_string(),
        session_id: session_id.to_string(),
        payload: serde_json::to_value(SessionSubscriptionPayload {
            state: state.to_string(),
            resume_from_seq,
            latest_seq: cursor.latest_seq,
            replay_window_start_seq: cursor.replay_window_start_seq,
            at: Utc::now(),
        })
        .unwrap(),
    };
    send_control_event(ws_sink, &event).await;
}

async fn send_control_event(
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    event: &ControlEvent,
) {
    let json = serde_json::to_string(event).unwrap();
    let _ = ws_sink.send(Message::Text(json.into())).await;
}

async fn send_control_error(
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    session_id: &str,
    code: &str,
    message: &str,
    request_id: Option<&str>,
) {
    let event = ControlEvent {
        event: "control_error".to_string(),
        session_id: session_id.to_string(),
        payload: serde_json::to_value(ControlErrorPayload {
            code: code.to_string(),
            message: message.to_string(),
            request_id: request_id.map(|s| s.to_string()),
        })
        .unwrap(),
    };
    send_control_event(ws_sink, &event).await;
}
