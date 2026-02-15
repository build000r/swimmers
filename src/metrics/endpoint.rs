//! Axum handler for the `GET /metrics` Prometheus scrape endpoint.
//!
//! The handler renders the current metrics snapshot in the Prometheus text
//! exposition format using the [`PrometheusHandle`] returned by
//! [`super::init_metrics`].

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;

/// Shared state for the metrics endpoint. This is kept separate from
/// `AppState` so the metrics module has no dependency on other throngterm
/// modules.
#[derive(Clone)]
pub struct MetricsState {
    pub handle: Arc<PrometheusHandle>,
}

/// Axum handler that renders the Prometheus text exposition format.
///
/// Returns `Content-Type: text/plain; version=0.0.4; charset=utf-8` as
/// required by the Prometheus scrape protocol.
async fn metrics_handler(State(state): State<MetricsState>) -> impl IntoResponse {
    let body = state.handle.render();
    (
        [(
            CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

/// Build a standalone router for `GET /metrics`.
///
/// This returns a `Router<()>` (no shared app state required) because it
/// carries its own `MetricsState` via a nested `.with_state()`.
///
/// # Usage in main.rs
///
/// ```rust,ignore
/// let prom_handle = metrics::init_metrics();
/// let metrics_router = metrics::endpoint::metrics_router(prom_handle);
///
/// let app = Router::new()
///     .merge(api::api_router())
///     .merge(metrics_router)
///     // ...
///     .with_state(state);
/// ```
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    let metrics_state = MetricsState {
        handle: Arc::new(handle),
    };
    Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(metrics_state)
}
