use axum::extract::ws::Message;
use serde::Deserialize;
use tokio::sync::oneshot;

use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::SessionCommand;
use crate::types::{clamp_terminal_resize, MAX_SESSION_INPUT_BYTES};

pub(super) const MAX_WS_INPUT_BYTES: usize = MAX_SESSION_INPUT_BYTES;
pub(super) const MAX_WS_TEXT_FRAME_BYTES: usize = MAX_WS_INPUT_BYTES + 4096;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserClientMessage {
    Auth {
        token: String,
    },
    InputText {
        data: String,
        #[serde(default, alias = "clientMessageId")]
        client_message_id: Option<String>,
    },
    SubmitLine {
        data: String,
        #[serde(default, alias = "clientMessageId")]
        client_message_id: Option<String>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Ping,
}

/// Pure routing decision derived from an incoming WebSocket message. No I/O.
#[derive(Debug)]
pub(super) enum WsClientDecision {
    Close,
    Ignore,
    SendPong(Vec<u8>),
    ReplyPong,
    SendError {
        code: &'static str,
        message: String,
        client_message_id: Option<String>,
    },
    Forward {
        cmd: SessionCommand,
        client_message_id: Option<String>,
    },
}

pub(super) fn decode_client_message(auth: &AuthInfo, message: &Message) -> WsClientDecision {
    match message {
        Message::Close(_) => WsClientDecision::Close,
        Message::Pong(_) => WsClientDecision::Ignore,
        Message::Ping(bytes) => WsClientDecision::SendPong(bytes.to_vec()),
        Message::Binary(bytes) => decode_binary_client_message(auth, bytes),
        Message::Text(text) => decode_text_client_message(auth, text.as_str()),
    }
}

fn decode_binary_client_message(auth: &AuthInfo, bytes: &[u8]) -> WsClientDecision {
    if !auth.has_scope(AuthScope::StreamWrite) {
        return read_only_terminal_error("observer connections cannot send terminal input", None);
    }
    decode_binary_input(bytes)
}

pub(super) fn decode_binary_input(bytes: &[u8]) -> WsClientDecision {
    if bytes.is_empty() {
        WsClientDecision::Ignore
    } else if bytes.len() > MAX_WS_INPUT_BYTES {
        oversized_input_error(None)
    } else {
        WsClientDecision::Forward {
            cmd: SessionCommand::WriteInput(bytes.to_vec()),
            client_message_id: None,
        }
    }
}

fn read_only_terminal_error(
    message: &'static str,
    client_message_id: Option<String>,
) -> WsClientDecision {
    WsClientDecision::SendError {
        code: "READ_ONLY",
        message: message.to_string(),
        client_message_id,
    }
}

fn oversized_input_error(client_message_id: Option<String>) -> WsClientDecision {
    WsClientDecision::SendError {
        code: "INPUT_TOO_LARGE",
        message: format!("terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit"),
        client_message_id,
    }
}

fn invalid_client_message_error(err: serde_json::Error) -> WsClientDecision {
    WsClientDecision::SendError {
        code: "INVALID_MESSAGE",
        message: format!("invalid control message: {err}"),
        client_message_id: None,
    }
}

fn parse_browser_client_message(text: &str) -> Result<BrowserClientMessage, WsClientDecision> {
    serde_json::from_str(text).map_err(invalid_client_message_error)
}

pub(super) fn decode_text_client_message(auth: &AuthInfo, text: &str) -> WsClientDecision {
    if text.len() > MAX_WS_TEXT_FRAME_BYTES {
        return oversized_text_frame_error();
    }

    parse_browser_client_message(text)
        .map(|parsed| decode_browser_client_message(auth, parsed))
        .unwrap_or_else(|decision| decision)
}

fn oversized_text_frame_error() -> WsClientDecision {
    WsClientDecision::SendError {
        code: "INPUT_TOO_LARGE",
        message: format!("terminal control frame exceeds {MAX_WS_TEXT_FRAME_BYTES} byte limit"),
        client_message_id: None,
    }
}

fn decode_browser_client_message(
    auth: &AuthInfo,
    parsed: BrowserClientMessage,
) -> WsClientDecision {
    match parsed {
        BrowserClientMessage::Ping => WsClientDecision::ReplyPong,
        BrowserClientMessage::Auth { token: _token } => WsClientDecision::Ignore,
        BrowserClientMessage::InputText {
            data,
            client_message_id,
        } => decode_input_text_message(auth, data, client_message_id),
        BrowserClientMessage::SubmitLine {
            data,
            client_message_id,
        } => decode_submit_line_message(auth, data, client_message_id),
        BrowserClientMessage::Resize { cols, rows } => decode_resize_message(auth, cols, rows),
    }
}

fn decode_input_text_message(
    auth: &AuthInfo,
    data: String,
    client_message_id: Option<String>,
) -> WsClientDecision {
    if !auth.has_scope(AuthScope::StreamWrite) {
        return read_only_terminal_error(
            "observer connections cannot send terminal input",
            client_message_id,
        );
    }
    decode_input_text(data, client_message_id)
}

pub(super) fn decode_input_text(
    data: String,
    client_message_id: Option<String>,
) -> WsClientDecision {
    decode_terminal_text_input(data, client_message_id, str::is_empty, |data| {
        SessionCommand::WriteInputAck {
            data: data.into_bytes(),
            ack: oneshot::channel().0,
        }
    })
}

fn decode_terminal_text_input(
    data: String,
    client_message_id: Option<String>,
    is_empty_input: impl FnOnce(&str) -> bool,
    build_command: impl FnOnce(String) -> SessionCommand,
) -> WsClientDecision {
    match (is_empty_input(&data), data.len() > MAX_WS_INPUT_BYTES) {
        (true, _) => WsClientDecision::Ignore,
        (_, true) => oversized_input_error(client_message_id),
        _ => WsClientDecision::Forward {
            cmd: build_command(data),
            client_message_id,
        },
    }
}

fn decode_submit_line_message(
    auth: &AuthInfo,
    data: String,
    client_message_id: Option<String>,
) -> WsClientDecision {
    if !auth.has_scope(AuthScope::StreamWrite) {
        return read_only_terminal_error(
            "observer connections cannot submit terminal input",
            client_message_id,
        );
    }
    decode_submit_line(data, client_message_id)
}

pub(super) fn decode_submit_line(
    data: String,
    client_message_id: Option<String>,
) -> WsClientDecision {
    decode_terminal_text_input(data, client_message_id, submit_line_is_empty, |data| {
        SessionCommand::SubmitLineAck {
            text: data,
            ack: oneshot::channel().0,
        }
    })
}

fn submit_line_is_empty(data: &str) -> bool {
    data.trim().is_empty()
}

fn decode_resize_message(auth: &AuthInfo, cols: u16, rows: u16) -> WsClientDecision {
    if !auth.has_scope(AuthScope::StreamWrite) {
        return read_only_terminal_error(
            "observer connections cannot resize terminal sessions",
            None,
        );
    }
    let (cols, rows) = clamp_terminal_resize(cols, rows);
    WsClientDecision::Forward {
        cmd: SessionCommand::Resize { cols, rows },
        client_message_id: None,
    }
}
