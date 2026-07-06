//! Authentication and authorization middleware.
//!
//! Two modes:
//! - `LocalTrust`: all requests pass through with full scopes (no overhead).
//! - `TailnetTrust`: same request behavior as local trust, but startup only
//!   allows it on Tailscale bind addresses.
//! - `Token`: Bearer token required in `Authorization` header. A valid token
//!   grants operator scopes (`sessions:read`, `sessions:write`, `stream:write`).

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, uri::Authority, HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use subtle::ConstantTimeEq;

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

/// All operator scopes — granted to operator tokens and trust modes.
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
    #[allow(clippy::result_large_err)]
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

fn untrusted_request_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            code: "UNTRUSTED_REQUEST_ORIGIN".to_string(),
            message: Some("Host or Origin is not allowed for trusted auth mode".to_string()),
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

#[allow(clippy::result_large_err)]
fn token_mode_auth_info(config: &Config, request: &Request) -> Result<AuthInfo, Response> {
    let Some(provided) = extract_bearer_token(request) else {
        return Err(not_authenticated_response());
    };

    if config
        .auth_token
        .as_deref()
        .is_some_and(|expected| bearer_tokens_eq(provided, expected))
    {
        return Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec()));
    }

    if config
        .observer_token
        .as_deref()
        .is_some_and(|expected| bearer_tokens_eq(provided, expected))
    {
        return Ok(AuthInfo::new(OBSERVER_SCOPES.to_vec()));
    }

    Err(not_authenticated_response())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrustedAuthority {
    host: String,
    port: Option<u16>,
}

#[allow(clippy::result_large_err)]
pub(crate) fn validate_trusted_request_headers(
    config: &Config,
    headers: &HeaderMap,
) -> Result<(), Response> {
    if matches!(config.auth_mode, AuthMode::Token) {
        return Ok(());
    }

    let Some(host) = trusted_authority_from_header(headers, header::HOST) else {
        return Err(untrusted_request_response());
    };
    if !trusted_authority_allowed(config, &host) {
        return Err(untrusted_request_response());
    }

    if let Some(origin) = headers.get(header::ORIGIN) {
        let Some(origin) = trusted_authority_from_origin(origin.to_str().ok()) else {
            return Err(untrusted_request_response());
        };
        if origin != host {
            return Err(untrusted_request_response());
        }
    }

    Ok(())
}

fn trusted_authority_from_header(
    headers: &HeaderMap,
    name: header::HeaderName,
) -> Option<TrustedAuthority> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(trusted_authority_from_authority_str)
}

fn trusted_authority_from_origin(value: Option<&str>) -> Option<TrustedAuthority> {
    let uri = value?.trim().parse::<Uri>().ok()?;
    let scheme = uri.scheme_str()?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    trusted_authority_from_authority(uri.authority()?)
}

fn trusted_authority_from_authority_str(value: &str) -> Option<TrustedAuthority> {
    let authority = value.trim().parse::<Authority>().ok()?;
    trusted_authority_from_authority(&authority)
}

fn trusted_authority_from_authority(authority: &Authority) -> Option<TrustedAuthority> {
    let host = normalize_authority_host(authority.host())?;
    Some(TrustedAuthority {
        host,
        port: authority.port_u16(),
    })
}

fn normalize_authority_host(host: &str) -> Option<String> {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn trusted_authority_allowed(config: &Config, authority: &TrustedAuthority) -> bool {
    match config.auth_mode {
        AuthMode::LocalTrust => trusted_host_is_loopback(&authority.host),
        AuthMode::TailnetTrust => {
            trusted_host_matches_config_bind(config, &authority.host)
                && trusted_host_is_tailnet(&authority.host)
        }
        AuthMode::Token => true,
    }
}

fn trusted_host_matches_config_bind(config: &Config, host: &str) -> bool {
    normalize_authority_host(crate::cli::bind_host(&config.bind)).is_some_and(|bind| bind == host)
}

fn trusted_host_is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn trusted_host_is_tailnet(host: &str) -> bool {
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let octets = ip.octets();
            octets[0] == 100 && (64..=127).contains(&octets[1])
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            let segments = ip.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
        Err(_) => false,
    }
}

#[allow(clippy::result_large_err)]
fn auth_info_for_request(config: &Config, request: &Request) -> Result<AuthInfo, Response> {
    match config.auth_mode {
        AuthMode::LocalTrust | AuthMode::TailnetTrust => {
            validate_trusted_request_headers(config, request.headers())?;
            Ok(AuthInfo::new(OPERATOR_SCOPES.to_vec()))
        }
        AuthMode::Token => token_mode_auth_info(config, request),
    }
}

fn bearer_tokens_eq(provided: &str, expected: &str) -> bool {
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Axum `from_fn` middleware that enforces authentication based on the
/// application's [`Config::auth_mode`].
///
/// In `LocalTrust` and `TailnetTrust` modes the request Host and any browser
/// Origin must still match the configured trust boundary before operator scopes
/// are inserted. Startup validation controls where those trust modes may bind.
///
/// In `Token` mode the `Authorization: Bearer <token>` header is validated
/// against [`Config::auth_token`]. A missing or invalid token results in a 401
/// JSON response.
pub async fn auth_middleware(config: Arc<Config>, mut request: Request, next: Next) -> Response {
    let auth_info = match auth_info_for_request(&config, &request) {
        Ok(info) => info,
        Err(response) => return response,
    };

    request.extensions_mut().insert(auth_info);
    next.run(request).await
}

/// Extract a bearer token from the `Authorization` header.
///
/// Returns `None` for missing, non-UTF-8, non-bearer-scheme, or empty
/// tokens. The empty-token guard makes a misconfigured `AUTH_TOKEN=""` (or a
/// header literally `Bearer `) impossible to authenticate, defense-in-depth
/// for the constant-time compare in `bearer_tokens_eq`.
fn extract_bearer_token(request: &Request) -> Option<&str> {
    let header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)?;
    let (scheme_end, separator) = header
        .char_indices()
        .find(|(_, ch)| ch.is_ascii_whitespace())?;
    let scheme = &header[..scheme_end];
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let token = header[scheme_end + separator.len_utf8()..].trim_start_matches(char::is_whitespace);
    if token.is_empty() {
        return None;
    }
    Some(token)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_host_origin(host: &str, origin: Option<&str>) -> Request {
        let mut request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        request.headers_mut().insert(
            header::HOST,
            axum::http::HeaderValue::from_str(host).expect("valid host header"),
        );
        if let Some(origin) = origin {
            request.headers_mut().insert(
                header::ORIGIN,
                axum::http::HeaderValue::from_str(origin).expect("valid origin header"),
            );
        }
        request
    }

    fn token_request_with_host_origin(host: &str, origin: Option<&str>, token: &str) -> Request {
        let mut request = request_with_host_origin(host, origin);
        request.headers_mut().insert(
            header::AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {token}"))
                .expect("valid authorization header"),
        );
        request
    }

    fn trust_mode_config(auth_mode: AuthMode, bind: &str) -> Config {
        Config {
            auth_mode,
            bind: bind.to_string(),
            ..Config::default()
        }
    }

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
    fn extract_bearer_scheme_is_case_insensitive() {
        use axum::http::HeaderValue;

        for value in ["bearer lower-token", "bEaReR mixed-token"] {
            let mut request = Request::builder()
                .uri("/test")
                .body(axum::body::Body::empty())
                .unwrap();
            request
                .headers_mut()
                .insert("authorization", HeaderValue::from_static(value));

            assert_eq!(
                extract_bearer_token(&request),
                value.split_once(' ').map(|(_, token)| token)
            );
        }
    }

    #[test]
    fn extract_bearer_accepts_extra_separator_whitespace() {
        use axum::http::HeaderValue;

        let mut request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_static("Bearer   spaced-token"),
        );

        assert_eq!(extract_bearer_token(&request), Some("spaced-token"));
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
    fn bearer_tokens_eq_distinguishes_equal_and_unequal_values() {
        assert!(bearer_tokens_eq("secret-token", "secret-token"));
        assert!(!bearer_tokens_eq("secret-token", "secret-tokxn"));
        assert!(!bearer_tokens_eq("secret-token", "secret-token-extra"));
    }

    #[test]
    fn extract_bearer_rejects_empty_token() {
        use axum::http::HeaderValue;

        let mut request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", HeaderValue::from_static("Bearer "));

        // A `Bearer ` header with no token must never authenticate, even
        // if a misconfigured AUTH_TOKEN="" exists.
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

    #[test]
    fn auth_info_for_request_trust_modes_grant_operator_scopes() {
        for (auth_mode, bind, host, origin) in [
            (
                AuthMode::LocalTrust,
                "127.0.0.1",
                "127.0.0.1:3210",
                "http://127.0.0.1:3210",
            ),
            (
                AuthMode::TailnetTrust,
                "100.64.1.2",
                "100.64.1.2:3210",
                "http://100.64.1.2:3210",
            ),
        ] {
            let request = request_with_host_origin(host, Some(origin));
            let config = trust_mode_config(auth_mode, bind);
            let info = auth_info_for_request(&config, &request).expect("trust mode auth");

            assert!(info.has_scope(AuthScope::SessionsRead));
            assert!(info.has_scope(AuthScope::SessionsWrite));
            assert!(info.has_scope(AuthScope::StreamWrite));
        }
    }

    #[test]
    fn trusted_request_rejects_hostile_host_before_granting_scopes() {
        for config in [
            trust_mode_config(AuthMode::LocalTrust, "127.0.0.1"),
            trust_mode_config(AuthMode::TailnetTrust, "100.64.1.2"),
        ] {
            let request = request_with_host_origin("attacker.test:3210", None);
            let response =
                auth_info_for_request(&config, &request).expect_err("hostile host rejected");

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[test]
    fn trusted_request_rejects_hostile_origin_before_granting_scopes() {
        for (config, host) in [
            (
                trust_mode_config(AuthMode::LocalTrust, "127.0.0.1"),
                "127.0.0.1:3210",
            ),
            (
                trust_mode_config(AuthMode::TailnetTrust, "100.64.1.2"),
                "100.64.1.2:3210",
            ),
        ] {
            let request = request_with_host_origin(host, Some("https://attacker.test"));
            let response =
                auth_info_for_request(&config, &request).expect_err("hostile origin rejected");

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[test]
    fn trusted_request_allows_loopback_and_configured_tailnet_hosts() {
        for (config, host, origin) in [
            (
                trust_mode_config(AuthMode::LocalTrust, "127.0.0.1"),
                "localhost:3210",
                "http://localhost:3210",
            ),
            (
                trust_mode_config(AuthMode::LocalTrust, "127.0.0.1"),
                "127.9.8.7:3210",
                "http://127.9.8.7:3210",
            ),
            (
                trust_mode_config(AuthMode::LocalTrust, "::1"),
                "[::1]:3210",
                "http://[::1]:3210",
            ),
            (
                trust_mode_config(AuthMode::TailnetTrust, "100.64.1.2"),
                "100.64.1.2:3210",
                "http://100.64.1.2:3210",
            ),
        ] {
            let request = request_with_host_origin(host, Some(origin));
            let info = auth_info_for_request(&config, &request).expect("trusted request allowed");

            assert!(info.has_scope(AuthScope::SessionsWrite));
        }
    }

    #[test]
    fn trusted_request_token_mode_ignores_host_origin_guard() {
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("secret".to_string()),
            observer_token: Some("observer".to_string()),
            ..Config::default()
        };
        let request = token_request_with_host_origin(
            "attacker.test:3210",
            Some("https://attacker.test"),
            "secret",
        );

        let info =
            auth_info_for_request(&config, &request).expect("token mode still authenticates");

        assert!(info.has_scope(AuthScope::SessionsWrite));
    }

    #[test]
    fn auth_info_for_request_token_mode_delegates_to_token_validation() {
        let mut request = Request::builder()
            .uri("/test")
            .body(axum::body::Body::empty())
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer observer"),
        );
        let config = Config {
            auth_mode: AuthMode::Token,
            auth_token: Some("secret".to_string()),
            observer_token: Some("observer".to_string()),
            ..Config::default()
        };

        let info = auth_info_for_request(&config, &request).expect("observer auth");

        assert!(info.has_scope(AuthScope::SessionsRead));
        assert!(!info.has_scope(AuthScope::SessionsWrite));
        assert!(!info.has_scope(AuthScope::StreamWrite));
    }
}
