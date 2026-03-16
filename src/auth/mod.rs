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

/// All operator scopes — granted to operator tokens and local-trust mode.
pub const OPERATOR_SCOPES: &[AuthScope] = &[
    AuthScope::SessionsRead,
    AuthScope::SessionsWrite,
    AuthScope::StreamWrite,
];

/// Observer scopes — read-only access (no session mutation, no terminal input).
pub const OBSERVER_SCOPES: &[AuthScope] = &[AuthScope::SessionsRead];

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

fn token_mode_auth_info(config: &Config, request: &Request) -> Result<AuthInfo, Response> {
    let Some(provided) = extract_bearer_token(request) else {
        return Err(not_authenticated_response());
    };

    if config
        .auth_token
        .as_deref()
        .is_some_and(|expected| provided == expected)
    {
        return Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec()));
    }

    if config
        .observer_token
        .as_deref()
        .is_some_and(|expected| provided == expected)
    {
        return Ok(AuthInfo::new(OBSERVER_SCOPES.to_vec()));
    }

    Err(not_authenticated_response())
}

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
    let auth_info = match config.auth_mode {
        AuthMode::LocalTrust => AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        AuthMode::Token => match token_mode_auth_info(&config, &request) {
            Ok(info) => info,
            Err(response) => return response,
        },
    };

    request.extensions_mut().insert(auth_info);
    next.run(request).await
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
        use axum::http::HeaderValue;

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
    fn observer_has_read_only_scope() {
        let info = AuthInfo::new(OBSERVER_SCOPES.to_vec());
        assert!(info.has_scope(AuthScope::SessionsRead));
        assert!(!info.has_scope(AuthScope::SessionsWrite));
        assert!(!info.has_scope(AuthScope::StreamWrite));
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

    #[test]
    fn token_mode_auth_info_rejects_missing_and_invalid_tokens() {
        let request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("secret".to_string()),
            observer_token: Some("observer".to_string()),
            ..Config::default()
        };
        assert!(token_mode_auth_info(&config, &request).is_err());

        let mut invalid_request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        invalid_request.headers_mut().insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer nope"),
        );
        assert!(token_mode_auth_info(&config, &invalid_request).is_err());
    }

    #[test]
    fn token_mode_auth_info_returns_expected_scopes() {
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("secret".to_string()),
            observer_token: Some("observer".to_string()),
            ..Config::default()
        };

        let mut operator_request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        operator_request.headers_mut().insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let operator = token_mode_auth_info(&config, &operator_request).expect("operator auth");
        assert!(operator.has_scope(AuthScope::SessionsWrite));

        let mut observer_request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        observer_request.headers_mut().insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer observer"),
        );
        let observer = token_mode_auth_info(&config, &observer_request).expect("observer auth");
        assert!(observer.has_scope(AuthScope::SessionsRead));
        assert!(!observer.has_scope(AuthScope::SessionsWrite));
    }
}
