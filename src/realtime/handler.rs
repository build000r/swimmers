//! WebSocket handler for the `/v1/realtime` endpoint.
//!
//! Multiplexes binary terminal I/O frames and JSON control messages over a
//! single WebSocket connection. Uses `tokio::select!` to concurrently read
//! from the client, forward supervisor lifecycle events, deliver per-session
//! terminal output, per-session control events, and thought updates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

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
const RESIZE_AUTH_WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
struct ResizeAuthority {
    client_id: ClientId,
    at: Instant,
}

#[derive(Debug, Clone, Copy)]
enum ResizeAuthorization {
    Authorized,
    Missing,
    Expired {
        owner_client_id: ClientId,
        age_ms: u128,
    },
    NotOwner {
        owner_client_id: ClientId,
        age_ms: u128,
    },
}

static RESIZE_AUTHORITIES: OnceLock<Mutex<HashMap<String, ResizeAuthority>>> = OnceLock::new();

fn allocate_client_id() -> ClientId {
    NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed)
}

fn resize_authorities() -> &'static Mutex<HashMap<String, ResizeAuthority>> {
    RESIZE_AUTHORITIES.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn note_resize_input_authority(session_id: &str, client_id: ClientId) {
    let mut authorities = resize_authorities().lock().await;
    authorities.insert(
        session_id.to_string(),
        ResizeAuthority {
            client_id,
            at: Instant::now(),
        },
    );
}

async fn evaluate_resize_authority(session_id: &str, client_id: ClientId) -> ResizeAuthorization {
    let mut authorities = resize_authorities().lock().await;
    let Some(authority) = authorities.get(session_id).copied() else {
        return ResizeAuthorization::Missing;
    };

    let age = authority.at.elapsed();
    if age > RESIZE_AUTH_WINDOW {
        authorities.remove(session_id);
        return ResizeAuthorization::Expired {
            owner_client_id: authority.client_id,
            age_ms: age.as_millis(),
        };
    }

    if authority.client_id == client_id {
        ResizeAuthorization::Authorized
    } else {
        ResizeAuthorization::NotOwner {
            owner_client_id: authority.client_id,
            age_ms: age.as_millis(),
        }
    }
}

async fn clear_resize_authority_for_client(client_id: ClientId) {
    let mut authorities = resize_authorities().lock().await;
    authorities.retain(|_, authority| authority.client_id != client_id);
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
    let mut lifecycle_rx: broadcast::Receiver<LifecycleEvent> = state.supervisor.subscribe_events();
    let mut thought_rx: broadcast::Receiver<ControlEvent> =
        state.supervisor.subscribe_thought_events();
    let mut output_subs: HashMap<String, SessionSub> = HashMap::new();

    tracing::info!("realtime client connected");
    let client_count = CONNECTED_CLIENTS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    crate::metrics::set_connected_clients(client_count);
    let connection_client_id = allocate_client_id();

    while process_socket_iteration(
        &mut ws_stream,
        &mut ws_sink,
        &mut lifecycle_rx,
        &mut thought_rx,
        &mut output_subs,
        connection_client_id,
        &state,
    )
    .await
    {}

    let client_count = CONNECTED_CLIENTS_COUNT.fetch_sub(1, Ordering::Relaxed) - 1;
    crate::metrics::set_connected_clients(client_count);
    cleanup_output_subs(&state, &mut output_subs).await;
    clear_resize_authority_for_client(connection_client_id).await;
    tracing::info!("realtime client cleanup complete");
}

async fn process_socket_iteration(
    ws_stream: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    lifecycle_rx: &mut broadcast::Receiver<LifecycleEvent>,
    thought_rx: &mut broadcast::Receiver<ControlEvent>,
    output_subs: &mut HashMap<String, SessionSub>,
    connection_client_id: ClientId,
    state: &Arc<AppState>,
) -> bool {
    let output_fut = poll_output_subs(output_subs);
    tokio::select! {
        msg = ws_stream.next() => {
            handle_socket_message(msg, connection_client_id, state, ws_sink, output_subs).await
        }
        event = lifecycle_rx.recv() => forward_lifecycle_event(event, ws_sink).await,
        event = thought_rx.recv() => forward_thought_event(event, ws_sink).await,
        Some(event) = output_fut => forward_polled_event(event, state, ws_sink).await,
    }
}

async fn handle_socket_message(
    msg: Option<Result<Message, axum::Error>>,
    client_id: ClientId,
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    output_subs: &mut HashMap<String, SessionSub>,
) -> bool {
    match msg {
        Some(Ok(message)) => {
            handle_received_socket_message(message, client_id, state, ws_sink, output_subs).await
        }
        Some(Err(e)) => {
            tracing::warn!("websocket receive error: {e}");
            false
        }
        None => {
            tracing::info!("realtime client disconnected");
            false
        }
    }
}

async fn handle_received_socket_message(
    message: Message,
    client_id: ClientId,
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    output_subs: &mut HashMap<String, SessionSub>,
) -> bool {
    match message {
        Message::Binary(data) => {
            crate::metrics::increment_frames_received();
            handle_binary_frame(&data, client_id, state, ws_sink).await;
        }
        Message::Text(text) => {
            handle_text_message(&text, client_id, state, ws_sink, output_subs).await;
        }
        Message::Close(_) => {
            tracing::info!("realtime client disconnected");
            return false;
        }
        Message::Ping(payload) => {
            let _ = ws_sink.send(Message::Pong(payload)).await;
        }
        Message::Pong(_) => {}
    }
    true
}

async fn forward_lifecycle_event(
    event: Result<LifecycleEvent, broadcast::error::RecvError>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) -> bool {
    match event {
        Ok(lifecycle_event) => {
            send_json_message(
                ws_sink,
                &lifecycle_to_control_event(lifecycle_event),
                "failed to send lifecycle event to client",
            )
            .await
        }
        Err(broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!("realtime client lagged by {n} lifecycle events");
            true
        }
        Err(broadcast::error::RecvError::Closed) => {
            tracing::info!("supervisor lifecycle broadcast closed");
            false
        }
    }
}

async fn forward_thought_event(
    event: Result<ControlEvent, broadcast::error::RecvError>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) -> bool {
    match event {
        Ok(thought_event) => {
            send_json_message(
                ws_sink,
                &thought_event,
                "failed to send thought_update event to client",
            )
            .await
        }
        Err(broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!("realtime client lagged by {n} thought events");
            true
        }
        Err(broadcast::error::RecvError::Closed) => {
            tracing::info!("thought broadcast closed");
            true
        }
    }
}

async fn forward_polled_event(
    event: PollEvent,
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) -> bool {
    match event {
        PollEvent::Output { session_id, frame } => {
            let binary = codec::encode_output_frame(&session_id, frame.seq, &frame.data);
            crate::metrics::increment_frames_sent();
            if ws_sink.send(Message::Binary(binary.into())).await.is_err() {
                tracing::warn!("failed to send output frame to client");
                return false;
            }
        }
        PollEvent::Overloaded { session_id } => {
            if !forward_overloaded_event(&session_id, state, ws_sink).await {
                return false;
            }
        }
        PollEvent::SessionEvent(control_event) => {
            if !send_json_message(
                ws_sink,
                &control_event,
                "failed to send session event to client",
            )
            .await
            {
                return false;
            }
        }
    }
    true
}

async fn forward_overloaded_event(
    session_id: &str,
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) -> bool {
    crate::metrics::increment_overload(session_id);
    if state.supervisor.get_session(session_id).await.is_none() {
        return true;
    }

    let event = ControlEvent {
        event: "session_overloaded".to_string(),
        session_id: session_id.to_string(),
        payload: serde_json::to_value(SessionOverloadedPayload {
            code: "SESSION_OVERLOADED".to_string(),
            queue_depth: state.config.outbound_queue_bound,
            queue_bytes: 0,
            retry_after_ms: 250,
        })
        .unwrap(),
    };
    send_json_message(ws_sink, &event, "failed to send session_overloaded event").await
}

async fn send_json_message(
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    event: &ControlEvent,
    warning: &str,
) -> bool {
    let json = serde_json::to_string(event).expect("ControlEvent must serialize");
    if ws_sink.send(Message::Text(json.into())).await.is_err() {
        tracing::warn!("{warning}");
        return false;
    }
    true
}

async fn cleanup_output_subs(state: &Arc<AppState>, output_subs: &mut HashMap<String, SessionSub>) {
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
            sprite_pack,
            repo_theme,
        } => ControlEvent {
            event: "session_created".to_string(),
            session_id,
            payload: serde_json::to_value(SessionCreatedPayload {
                reason,
                session: summary,
                sprite_pack,
                repo_theme,
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
    client_id: ClientId,
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
                note_resize_input_authority(&session_id, client_id).await;
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
    client_id: ClientId,
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
            handle_resize(client_id, state, ws_sink, &msg.payload, &request_id).await;
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
    client_id: ClientId,
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

    match evaluate_resize_authority(&resize.session_id, client_id).await {
        ResizeAuthorization::Authorized => {}
        ResizeAuthorization::Missing => {
            tracing::warn!(
                session_id = %resize.session_id,
                requester_client_id = client_id,
                reason = "no_recent_authority",
                "blocked resize from non-authoritative client"
            );
            send_control_error(
                ws_sink,
                &resize.session_id,
                "RESIZE_AUTH_REQUIRED",
                "resize requires recent keyboard input from this client",
                request_id.as_deref(),
            )
            .await;
            return;
        }
        ResizeAuthorization::Expired {
            owner_client_id,
            age_ms,
        } => {
            tracing::warn!(
                session_id = %resize.session_id,
                requester_client_id = client_id,
                owner_client_id,
                lease_age_ms = age_ms,
                reason = "expired_authority",
                "blocked resize from non-authoritative client"
            );
            send_control_error(
                ws_sink,
                &resize.session_id,
                "RESIZE_AUTH_REQUIRED",
                "resize requires recent keyboard input from this client",
                request_id.as_deref(),
            )
            .await;
            return;
        }
        ResizeAuthorization::NotOwner {
            owner_client_id,
            age_ms,
        } => {
            tracing::warn!(
                session_id = %resize.session_id,
                requester_client_id = client_id,
                owner_client_id,
                lease_age_ms = age_ms,
                reason = "non_authoritative_resize",
                "blocked resize from non-authoritative client"
            );
            send_control_error(
                ws_sink,
                &resize.session_id,
                "RESIZE_AUTH_REQUIRED",
                "resize requires recent keyboard input from this client",
                request_id.as_deref(),
            )
            .await;
            return;
        }
    }

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
        return std::future::pending().await;
    }

    loop {
        if let Some(event) = poll_output_subs_once(subs) {
            return Some(event);
        }
        if subs.is_empty() {
            return std::future::pending().await;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn poll_output_subs_once(subs: &mut HashMap<String, SessionSub>) -> Option<PollEvent> {
    let keys: Vec<String> = subs.keys().cloned().collect();
    for key in keys {
        if let Some(event) = poll_single_sub(subs, &key) {
            return Some(event);
        }
    }
    None
}

fn poll_single_sub(subs: &mut HashMap<String, SessionSub>, session_id: &str) -> Option<PollEvent> {
    if let Some(event) = poll_sub_output(subs, session_id) {
        return Some(event);
    }

    let sub = subs.get_mut(session_id)?;
    poll_sub_event(session_id, sub)
}

fn poll_sub_output(subs: &mut HashMap<String, SessionSub>, session_id: &str) -> Option<PollEvent> {
    let sub = subs.get_mut(session_id)?;
    match sub.output_rx.try_recv() {
        Ok(frame) => Some(PollEvent::Output {
            session_id: session_id.to_string(),
            frame,
        }),
        Err(mpsc::error::TryRecvError::Disconnected) => {
            tracing::info!("[session {session_id}] output channel closed, unsubscribing");
            subs.remove(session_id);
            Some(PollEvent::Overloaded {
                session_id: session_id.to_string(),
            })
        }
        Err(mpsc::error::TryRecvError::Empty) => None,
    }
}

fn poll_sub_event(session_id: &str, sub: &mut SessionSub) -> Option<PollEvent> {
    match sub.event_rx.try_recv() {
        Ok(event) => Some(PollEvent::SessionEvent(event)),
        Err(broadcast::error::TryRecvError::Lagged(n)) => {
            tracing::warn!("[session {session_id}] lagged by {n} session events");
            None
        }
        Err(broadcast::error::TryRecvError::Closed) => None,
        Err(broadcast::error::TryRecvError::Empty) => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn test_session_id(prefix: &str) -> String {
        let n = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-session-{n}")
    }

    #[tokio::test]
    async fn resize_authority_missing_without_recent_input() {
        let session_id = test_session_id("missing");

        let auth = evaluate_resize_authority(&session_id, 77).await;
        assert!(matches!(auth, ResizeAuthorization::Missing));
    }

    #[tokio::test]
    async fn resize_authority_latest_input_wins() {
        let session_id = test_session_id("latest");

        note_resize_input_authority(&session_id, 11).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        note_resize_input_authority(&session_id, 22).await;

        let latest_owner = evaluate_resize_authority(&session_id, 22).await;
        assert!(matches!(latest_owner, ResizeAuthorization::Authorized));

        let previous_owner = evaluate_resize_authority(&session_id, 11).await;
        assert!(matches!(
            previous_owner,
            ResizeAuthorization::NotOwner {
                owner_client_id: 22,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn resize_authority_is_scoped_per_session() {
        let source_session = test_session_id("scope-a");
        let other_session = test_session_id("scope-b");

        note_resize_input_authority(&source_session, 91).await;

        let auth = evaluate_resize_authority(&other_session, 91).await;
        assert!(matches!(auth, ResizeAuthorization::Missing));
    }

    #[tokio::test]
    async fn resize_authority_accepts_just_inside_window() {
        let session_id = test_session_id("inside-window");
        {
            let mut authorities = resize_authorities().lock().await;
            authorities.insert(
                session_id.clone(),
                ResizeAuthority {
                    client_id: 33,
                    at: Instant::now() - (RESIZE_AUTH_WINDOW - Duration::from_millis(5)),
                },
            );
        }

        let auth = evaluate_resize_authority(&session_id, 33).await;
        assert!(matches!(auth, ResizeAuthorization::Authorized));
    }

    #[tokio::test]
    async fn resize_authority_expires_after_window_and_is_removed() {
        let session_id = test_session_id("expired-window");
        {
            let mut authorities = resize_authorities().lock().await;
            authorities.insert(
                session_id.clone(),
                ResizeAuthority {
                    client_id: 44,
                    at: Instant::now() - RESIZE_AUTH_WINDOW - Duration::from_millis(100),
                },
            );
        }

        let expired = evaluate_resize_authority(&session_id, 44).await;
        assert!(matches!(
            expired,
            ResizeAuthorization::Expired {
                owner_client_id: 44,
                ..
            }
        ));

        let removed = evaluate_resize_authority(&session_id, 44).await;
        assert!(matches!(removed, ResizeAuthorization::Missing));
    }

    #[tokio::test]
    async fn clear_resize_authority_for_client_removes_all_owned_sessions() {
        let client_to_clear = 501;
        let other_client = 777;
        let session_one = test_session_id("clear-one");
        let session_two = test_session_id("clear-two");
        let session_other = test_session_id("clear-other");

        note_resize_input_authority(&session_one, client_to_clear).await;
        note_resize_input_authority(&session_two, client_to_clear).await;
        note_resize_input_authority(&session_other, other_client).await;

        clear_resize_authority_for_client(client_to_clear).await;

        let one = evaluate_resize_authority(&session_one, client_to_clear).await;
        assert!(matches!(one, ResizeAuthorization::Missing));

        let two = evaluate_resize_authority(&session_two, client_to_clear).await;
        assert!(matches!(two, ResizeAuthorization::Missing));

        let other = evaluate_resize_authority(&session_other, other_client).await;
        assert!(matches!(other, ResizeAuthorization::Authorized));
    }
}

#[cfg(test)]
mod control_error_tests {
    use super::*;
    use crate::api::AppState;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::runtime_config::ThoughtConfig;
    use futures::Sink;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::task::{Context, Poll};
    use tokio::sync::RwLock;

    #[derive(Default)]
    struct CaptureSink {
        messages: Vec<Message>,
    }

    impl Sink<Message> for CaptureSink {
        type Error = axum::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.messages.push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            daemon_defaults: None,
            file_store: None,
            published_selection: Arc::new(RwLock::new(
                crate::api::PublishedSelectionState::default(),
            )),
        })
    }

    fn last_control_error(sink: &CaptureSink) -> Option<(String, String, Option<String>)> {
        let text = sink
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::Text(value) => Some(value.to_string()),
                _ => None,
            })?;

        let event: ControlEvent = serde_json::from_str(&text).ok()?;
        if event.event != "control_error" {
            return None;
        }
        let payload: ControlErrorPayload = serde_json::from_value(event.payload).ok()?;
        Some((event.session_id, payload.code, payload.request_id))
    }

    fn text_events(sink: &CaptureSink) -> Vec<ControlEvent> {
        sink.messages
            .iter()
            .filter_map(|message| match message {
                Message::Text(value) => serde_json::from_str::<ControlEvent>(value).ok(),
                _ => None,
            })
            .collect()
    }

    async fn spawn_subscription_handle(
        session_id: &str,
        cursor: ReplayCursor,
        truncated: bool,
        unsubscribed: Arc<StdMutex<Vec<ClientId>>>,
    ) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(session_id, format!("tmux-{session_id}"), cmd_tx);
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::Subscribe { ack, .. } => {
                        let outcome = if truncated {
                            SubscribeOutcome::ReplayTruncated {
                                requested_resume_from_seq: 7,
                                replay_window_start_seq: 3,
                                latest_seq: cursor.latest_seq,
                            }
                        } else {
                            SubscribeOutcome::Ok
                        };
                        let _ = ack.send(outcome);
                    }
                    SessionCommand::GetReplayCursor(reply) => {
                        let _ = reply.send(cursor);
                    }
                    SessionCommand::Unsubscribe { client_id } => {
                        unsubscribed.lock().unwrap().push(client_id);
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    #[tokio::test]
    async fn resize_validation_error_precedes_auth_check() {
        let state = test_state();
        let mut sink = CaptureSink::default();
        let payload = serde_json::json!({
            "session_id": "sess-validation",
            "cols": "bad",
            "rows": 30
        });
        let request_id = Some("req-77".to_string());

        handle_resize(10, &state, &mut sink, &payload, &request_id).await;

        let error = last_control_error(&sink).expect("expected control_error event");
        assert_eq!(error.1, "VALIDATION_FAILED");
        assert_eq!(error.2.as_deref(), Some("req-77"));
    }

    #[tokio::test]
    async fn resize_unknown_session_precedes_auth_required() {
        let state = test_state();
        let mut sink = CaptureSink::default();
        let payload = serde_json::json!({
            "session_id": "missing-1",
            "cols": 80,
            "rows": 24
        });
        let request_id = Some("req-missing".to_string());

        handle_resize(10, &state, &mut sink, &payload, &request_id).await;

        let error = last_control_error(&sink).expect("expected control_error event");
        assert_eq!(error.0, "missing-1");
        assert_eq!(error.1, "SESSION_NOT_FOUND");
        assert_eq!(error.2.as_deref(), Some("req-missing"));
    }

    #[tokio::test]
    async fn handle_subscribe_emits_subscription_and_replay_truncated_events() {
        let state = test_state();
        let unsubscribed = Arc::new(StdMutex::new(Vec::new()));
        let handle = spawn_subscription_handle(
            "sess-sub",
            ReplayCursor {
                latest_seq: 11,
                replay_window_start_seq: 4,
            },
            true,
            unsubscribed,
        )
        .await;
        state.supervisor.insert_test_handle(handle).await;

        let mut sink = CaptureSink::default();
        let mut output_subs = HashMap::new();
        let payload = serde_json::json!({
            "session_id": "sess-sub",
            "resume_from_seq": 7
        });
        let request_id = Some("req-sub".to_string());

        handle_subscribe(&state, &mut sink, &mut output_subs, &payload, &request_id).await;

        assert!(output_subs.contains_key("sess-sub"));
        let events = text_events(&sink);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "session_subscription");
        assert_eq!(events[0].session_id, "sess-sub");
        assert_eq!(events[1].event, "replay_truncated");
        assert_eq!(events[1].session_id, "sess-sub");
    }

    #[tokio::test]
    async fn handle_unsubscribe_removes_subscription_and_emits_event() {
        let state = test_state();
        let unsubscribed = Arc::new(StdMutex::new(Vec::new()));
        let handle = spawn_subscription_handle(
            "sess-unsub",
            ReplayCursor {
                latest_seq: 14,
                replay_window_start_seq: 6,
            },
            false,
            unsubscribed.clone(),
        )
        .await;
        state.supervisor.insert_test_handle(handle).await;

        let (_output_tx, output_rx) = mpsc::channel(1);
        let (_event_tx, event_rx) = broadcast::channel(1);
        let mut output_subs = HashMap::from([(
            "sess-unsub".to_string(),
            SessionSub {
                client_id: 42,
                output_rx,
                event_rx,
            },
        )]);
        let mut sink = CaptureSink::default();
        let payload = serde_json::json!({
            "session_id": "sess-unsub"
        });

        handle_unsubscribe(&state, &mut sink, &mut output_subs, &payload, &None).await;

        assert!(!output_subs.contains_key("sess-unsub"));
        assert_eq!(unsubscribed.lock().unwrap().as_slice(), &[42]);

        let events = text_events(&sink);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "session_subscription");
        assert_eq!(events[0].session_id, "sess-unsub");

        let payload: SessionSubscriptionPayload =
            serde_json::from_value(events[0].payload.clone()).expect("session subscription");
        assert_eq!(payload.state, "unsubscribed");
        assert_eq!(payload.resume_from_seq, None);
        assert_eq!(payload.latest_seq, 14);
        assert_eq!(payload.replay_window_start_seq, 6);
    }

    #[tokio::test]
    async fn forward_polled_event_handles_all_event_kinds() {
        let state = test_state();
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-overloaded", "tmux", cmd_tx))
            .await;

        let mut sink = CaptureSink::default();
        assert!(
            forward_polled_event(
                PollEvent::Output {
                    session_id: "sess-out".to_string(),
                    frame: OutputFrame {
                        seq: 9,
                        data: b"hello".to_vec(),
                    },
                },
                &state,
                &mut sink,
            )
            .await
        );
        assert!(matches!(sink.messages.first(), Some(Message::Binary(_))));

        assert!(
            forward_polled_event(
                PollEvent::Overloaded {
                    session_id: "sess-overloaded".to_string(),
                },
                &state,
                &mut sink,
            )
            .await
        );
        assert!(
            forward_polled_event(
                PollEvent::SessionEvent(ControlEvent {
                    event: "session_state".to_string(),
                    session_id: "sess-out".to_string(),
                    payload: serde_json::json!({"state":"idle"}),
                }),
                &state,
                &mut sink,
            )
            .await
        );

        let events = text_events(&sink);
        assert!(events
            .iter()
            .any(|event| event.event == "session_overloaded"));
        assert!(events.iter().any(|event| event.event == "session_state"));
    }

    #[tokio::test]
    async fn poll_output_subs_returns_output_event_and_overload() {
        let (output_tx, output_rx) = mpsc::channel(1);
        let (_event_tx, event_rx) = broadcast::channel(1);
        output_tx
            .send(OutputFrame {
                seq: 3,
                data: b"frame".to_vec(),
            })
            .await
            .expect("send output frame");
        let mut subs = HashMap::from([(
            "sess-ready".to_string(),
            SessionSub {
                client_id: 1,
                output_rx,
                event_rx,
            },
        )]);

        let ready = poll_output_subs(&mut subs).await.expect("ready event");
        assert!(matches!(
            ready,
            PollEvent::Output {
                session_id,
                frame: OutputFrame { seq: 3, .. }
            } if session_id == "sess-ready"
        ));

        let (stale_tx, stale_rx) = mpsc::channel(1);
        let (_event_tx, event_rx) = broadcast::channel(1);
        drop(stale_tx);
        let mut subs = HashMap::from([(
            "sess-overloaded".to_string(),
            SessionSub {
                client_id: 2,
                output_rx: stale_rx,
                event_rx,
            },
        )]);

        let overloaded = poll_output_subs(&mut subs).await.expect("overload event");
        assert!(matches!(
            overloaded,
            PollEvent::Overloaded { session_id } if session_id == "sess-overloaded"
        ));
    }
}
