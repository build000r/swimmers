use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::types::ErrorResponse;

pub struct ApiErrorDef {
    pub status: StatusCode,
    pub code: &'static str,
    pub default_message: &'static str,
}

pub const NATIVE_DESKTOP_UNAVAILABLE: ApiErrorDef = ApiErrorDef {
    status: StatusCode::BAD_REQUEST,
    code: "NATIVE_DESKTOP_UNAVAILABLE",
    default_message: "native desktop unavailable",
};

pub const SESSION_NOT_FOUND: ApiErrorDef = ApiErrorDef {
    status: StatusCode::NOT_FOUND,
    code: "SESSION_NOT_FOUND",
    default_message: "session not found",
};

pub const SESSION_EXITED: ApiErrorDef = ApiErrorDef {
    status: StatusCode::CONFLICT,
    code: "SESSION_EXITED",
    default_message: "session has already exited",
};

pub const NATIVE_OPEN_FAILED: ApiErrorDef = ApiErrorDef {
    status: StatusCode::INTERNAL_SERVER_ERROR,
    code: "NATIVE_DESKTOP_OPEN_FAILED",
    default_message: "native desktop open failed",
};

pub const INVALID_SKILL_TOOL: ApiErrorDef = ApiErrorDef {
    status: StatusCode::BAD_REQUEST,
    code: "INVALID_SKILL_TOOL",
    default_message: "tool must be one of: claude, codex",
};

pub const VALIDATION_FAILED: ApiErrorDef = ApiErrorDef {
    status: StatusCode::BAD_REQUEST,
    code: "VALIDATION_FAILED",
    default_message: "validation failed",
};

pub const PERSISTENCE_UNAVAILABLE: ApiErrorDef = ApiErrorDef {
    status: StatusCode::SERVICE_UNAVAILABLE,
    code: "PERSISTENCE_UNAVAILABLE",
    default_message: "persistence unavailable",
};

pub const INTERNAL_ERROR: ApiErrorDef = ApiErrorDef {
    status: StatusCode::INTERNAL_SERVER_ERROR,
    code: "INTERNAL_ERROR",
    default_message: "internal error",
};

pub const VERSION_CONFLICT: ApiErrorDef = ApiErrorDef {
    status: StatusCode::PRECONDITION_FAILED,
    code: "VERSION_CONFLICT",
    default_message: "version mismatch",
};

pub fn error_body(code: impl Into<String>, message: Option<String>) -> ErrorResponse {
    ErrorResponse::new(code, message)
}

pub fn error_body_msg(code: impl Into<String>, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse::with_message(code, message)
}

pub fn api_error(def: &ApiErrorDef) -> Response {
    error_response(def.status, def.code, def.default_message)
}

pub fn api_error_msg(def: &ApiErrorDef, message: impl Into<String>) -> Response {
    error_response(def.status, def.code, message)
}

pub fn error_response(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let body = serde_json::to_value(error_body_msg(code, message)).unwrap_or_else(|_| {
        serde_json::json!({
            "code": "internal_error",
            "message": "failed to serialize error response",
        })
    });
    (status, Json(body)).into_response()
}

pub fn success_json<T: Serialize>(status: StatusCode, body: &T) -> Response {
    match serde_json::to_value(body) {
        Ok(value) => (status, Json(value)).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SERIALIZATION_FAILED",
            "failed to serialize response",
        ),
    }
}

pub fn parse_if_match_version(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().trim_matches('"').parse::<u64>().ok())
}

/// A validated reservation for the next version of an optimistic-concurrency
/// counter. Holding this value asserts the `If-Match` precondition was checked
/// against `counter` while the caller's serializing lock was held.
///
/// `commit` publishes the new version; dropping without committing aborts the
/// reservation and leaves the counter untouched, so a fallible step (e.g. a
/// disk save) can sit between reservation and commit without consuming a
/// version slot on failure.
#[must_use = "version reservation must be committed or explicitly dropped"]
pub struct VersionReservation<'a> {
    counter: &'a AtomicU64,
    new_version: u64,
}

impl VersionReservation<'_> {
    pub fn commit(self) -> u64 {
        self.counter.store(self.new_version, Ordering::Release);
        self.new_version
    }
}

/// Validate an `If-Match` precondition and reserve the next version.
///
/// Must be called from inside the caller's write lock so concurrent writers
/// cannot race the precondition check and so the eventual commit order matches
/// the order in which state writes land. Returns `None` when the precondition
/// fails or the counter would overflow; the caller maps that to
/// [`VERSION_CONFLICT`].
pub fn reserve_version_locked(
    counter: &AtomicU64,
    requested_version: Option<u64>,
) -> Option<VersionReservation<'_>> {
    let current = counter.load(Ordering::Acquire);
    let new_version = match requested_version {
        Some(expected) if current != expected => return None,
        Some(expected) => expected.checked_add(1)?,
        None => current.checked_add(1)?,
    };
    Some(VersionReservation {
        counter,
        new_version,
    })
}
