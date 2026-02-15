//! WebSocket handler for the `/v1/realtime` endpoint.
//!
//! Multiplexes binary terminal I/O frames and JSON control messages over a
//! single WebSocket connection. Uses `tokio::select!` to concurrently read
//! from the client, forward supervisor lifecycle events, and deliver
//! per-session terminal output.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc};

use crate::api::AppState;
use crate::realtime::codec;
use crate::session::actor::{ClientId, OutputFrame, SessionCommand};
use crate::session::supervisor::LifecycleEvent;
use crate::types::{
    ClientControlMessage, ControlErrorPayload, ControlEvent, DismissAttentionPayload,
    ResizePayload, SessionCreatedPayload, SessionDeletedPayload, SubscribeSessionPayload,
};

/// Global client ID counter for assigning unique IDs to output subscriptions.
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

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
    session_id: String,
    output_rx: mpsc::Receiver<OutputFrame>,
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
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Subscribe to supervisor-level lifecycle events (session created/deleted).
    let mut lifecycle_rx: broadcast::Receiver<LifecycleEvent> =
        state.supervisor.subscribe_events();

    // Track per-session output subscriptions: session_id -> sub state.
    let mut output_subs: HashMap<String, SessionSub> = HashMap::new();

    tracing::info!("realtime client connected");

    loop {
        // Build a future that resolves when any subscribed session has output.
        let output_fut = poll_output_subs(&mut output_subs);

        tokio::select! {
            // --- Client -> Server messages ---
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
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

            // --- Per-session terminal output ---
            Some((session_id, frame)) = output_fut => {
                let binary = codec::encode_output_frame(&session_id, frame.seq, &frame.data);
                if ws_sink.send(Message::Binary(binary.into())).await.is_err() {
                    tracing::warn!("failed to send output frame to client");
                    break;
                }
            }
        }
    }

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
            payload: serde_json::to_value(SessionCreatedPayload { reason, session: summary })
                .unwrap(),
        },
        LifecycleEvent::Deleted {
            session_id,
            reason,
        } => ControlEvent {
            event: "session_deleted".to_string(),
            session_id,
            payload: serde_json::to_value(SessionDeletedPayload {
                reason,
                delete_mode: "detach_bridge".to_string(),
                tmux_session_alive: true,
                at: Utc::now(),
            })
            .unwrap(),
        },
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
        let _ = handle
            .send(SessionCommand::Unsubscribe {
                client_id: old_sub.client_id,
            })
            .await;
    }

    // Create a bounded channel for this subscription's output.
    let client_id = allocate_client_id();
    let (client_tx, client_rx) = mpsc::channel::<OutputFrame>(
        state.config.outbound_queue_bound,
    );

    // Send the Subscribe command to the session actor. The actor handles
    // replay internally and sends replayed frames through the channel.
    if let Err(e) = handle
        .send(SessionCommand::Subscribe {
            client_id,
            client_tx,
            resume_from_seq: sub_payload.resume_from_seq,
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
            session_id: session_id.clone(),
            output_rx: client_rx,
        },
    );

    tracing::info!(
        "[session {session_id}] client {client_id} subscribed (resume_from_seq={:?})",
        sub_payload.resume_from_seq
    );
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

/// Poll all per-session output receivers and return the first ready frame.
///
/// Returns `None` (via `pending`) if there are no active subscriptions, which
/// causes the `tokio::select!` branch to be disabled.
async fn poll_output_subs(
    subs: &mut HashMap<String, SessionSub>,
) -> Option<(String, OutputFrame)> {
    if subs.is_empty() {
        // Return a future that never resolves so the select branch is inactive.
        return std::future::pending().await;
    }

    // First pass: try_recv for non-blocking check across all subscriptions.
    let keys: Vec<String> = subs.keys().cloned().collect();
    for key in &keys {
        if let Some(sub) = subs.get_mut(key) {
            match sub.output_rx.try_recv() {
                Ok(frame) => return Some((key.clone(), frame)),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    tracing::info!("[session {key}] output channel closed, unsubscribing");
                    subs.remove(key);
                    return None; // Will re-enter select loop.
                }
                Err(mpsc::error::TryRecvError::Empty) => continue,
            }
        }
    }

    // No data immediately available. Await the first receiver.
    // For true fairness we would use FuturesUnordered, but this is adequate
    // for the expected session count (typically < 32).
    if let Some((key, sub)) = subs.iter_mut().next() {
        let key = key.clone();
        match sub.output_rx.recv().await {
            Some(frame) => Some((key, frame)),
            None => {
                tracing::info!("[session {key}] output channel closed during recv");
                subs.remove(&key);
                None
            }
        }
    } else {
        std::future::pending().await
    }
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
    let json = serde_json::to_string(&event).unwrap();
    let _ = ws_sink.send(Message::Text(json.into())).await;
}
