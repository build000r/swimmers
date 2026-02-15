//! Authentication and authorization middleware.
//!
//! Two modes:
//! - `LocalTrust`: all requests pass through with full scopes (no overhead).
//! - `Token`: Bearer token required in `Authorization` header. A valid token
//!   grants operator scopes (`sessions:read`, `sessions:write`, `stream:write`).

use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::config::{AuthMode, Config};
use crate::types::ErrorResponse;

// ---------------------------------------------------------------------------
// Scopes
// ---------------------------------------------------------------------------

/// Authorization scopes that can be granted to a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthScope {
    SessionsRead,
    SessionsWrite,
    StreamWrite,
}

/// All operator scopes (a valid token currently receives all of these).
pub const OPERATOR_SCOPES: &[AuthScope] = &[
    AuthScope::SessionsRead,
    AuthScope::SessionsWrite,
    AuthScope::StreamWrite,
];

// ---------------------------------------------------------------------------
// AuthInfo — inserted as a request extension
// ---------------------------------------------------------------------------

/// Resolved authentication information attached to every request that passes
/// through the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthInfo {
    scopes: Vec<AuthScope>,
}

impl AuthInfo {
    /// Create an `AuthInfo` with the given scopes.
    pub fn new(scopes: Vec<AuthScope>) -> Self {
        Self { scopes }
    }

    /// Returns `true` if this auth info carries the given scope.
    pub fn has_scope(&self, scope: AuthScope) -> bool {
        self.scopes.contains(&scope)
    }

    /// Convenience: require a scope or return a 403 error response.
    pub fn require_scope(&self, scope: AuthScope) -> Result<(), Response> {
        if self.has_scope(scope) {
            Ok(())
        } else {
            Err(forbidden_response())
        }
    }
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn not_authenticated_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            code: "NOT_AUTHENTICATED".to_string(),
            message: Some("Missing or invalid authentication token".to_string()),
        }),
    )
        .into_response()
}

fn forbidden_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            code: "NOT_AUTHORIZED".to_string(),
            message: Some("Insufficient scope for this action".to_string()),
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Axum `from_fn` middleware that enforces authentication based on the
/// application's [`Config::auth_mode`].
///
/// In `LocalTrust` mode this is a transparent pass-through: an `AuthInfo` with
/// all operator scopes is inserted and the request continues immediately.
///
/// In `Token` mode the `Authorization: Bearer <token>` header is validated
/// against [`Config::auth_token`]. A missing or invalid token results in a 401
/// JSON response.
pub async fn auth_middleware(config: Arc<Config>, mut request: Request, next: Next) -> Response {
    match config.auth_mode {
        AuthMode::LocalTrust => {
            // Grant all scopes — no authentication overhead.
            request
                .extensions_mut()
                .insert(AuthInfo::new(OPERATOR_SCOPES.to_vec()));
            next.run(request).await
        }
        AuthMode::Token => {
            let token = extract_bearer_token(&request);

            match (&config.auth_token, token) {
                (Some(expected), Some(provided)) if provided == expected.as_str() => {
                    // Valid operator token — grant all scopes.
                    request
                        .extensions_mut()
                        .insert(AuthInfo::new(OPERATOR_SCOPES.to_vec()));
                    next.run(request).await
                }
                _ => {
                    // Missing, invalid, or no auth_token configured in token mode.
                    not_authenticated_response()
                }
            }
        }
    }
}

/// Extract a bearer token from the `Authorization` header.
fn extract_bearer_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_info_has_scope() {
        let info = AuthInfo::new(vec![AuthScope::SessionsRead]);
        assert!(info.has_scope(AuthScope::SessionsRead));
        assert!(!info.has_scope(AuthScope::SessionsWrite));
        assert!(!info.has_scope(AuthScope::StreamWrite));
    }

    #[test]
    fn operator_has_all_scopes() {
        let info = AuthInfo::new(OPERATOR_SCOPES.to_vec());
        assert!(info.has_scope(AuthScope::SessionsRead));
        assert!(info.has_scope(AuthScope::SessionsWrite));
        assert!(info.has_scope(AuthScope::StreamWrite));
    }

    #[test]
    fn extract_bearer_works() {
        use axum::http::{HeaderMap, HeaderValue};

        // Build a minimal request with an Authorization header.
        let mut request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_static("Bearer my-secret-token"),
        );

        assert_eq!(extract_bearer_token(&request), Some("my-secret-token"));
    }

    #[test]
    fn extract_bearer_missing() {
        let request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();

        assert_eq!(extract_bearer_token(&request), None);
    }

    #[test]
    fn extract_bearer_wrong_scheme() {
        use axum::http::HeaderValue;

        let mut request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        );

        assert_eq!(extract_bearer_token(&request), None);
    }
}
