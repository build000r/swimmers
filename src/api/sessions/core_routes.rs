use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::api::envelope::error_body;
use crate::api::service::{
    cleanup_exact_local_session, create_local_session, create_local_sessions_batch,
    list_sessions_for_client, validate_sessions_batch_dirs,
};
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::config::SessionDeleteMode;
use crate::fleet_lens::{build_fleet_lens_presets, build_fleet_lens_summary};
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};
use crate::session::supervisor::{CassSessionAdmission, TmuxAdoptError};
use crate::types::{
    AdoptSessionRequest, AdoptSessionResponse, AuthorizedProviderResumeLaunchReceipt,
    AuthorizedProviderResumeReceipt, CassAdmissionError, CassAdmissionIntent,
    CassAdmissionPreflightRequest, CassAdmissionPreflightResponse,
    CassAdmissionReservationEnvelope, CassAwareCreateSessionsBatchRequest,
    CassBatchAdmissionAttachment, CreateSessionRequest, CreateSessionResponse,
    CreateSessionsBatchRequest, CreateSessionsBatchResponse, EnvironmentListResponse,
    LaunchTargetSummary, ProviderResumeCaptureSource, ProviderResumeProvider, SessionInputRequest,
    SessionInputResponse, SessionListResponse, SessionState, TerminalSnapshot,
    CASS_ADMISSION_RESERVATION_SCHEMA, MAX_SESSION_INPUT_BYTES,
};

const SNAPSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const INPUT_DELIVERY_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

// ---------------------------------------------------------------------------
// GET /v1/sessions
// ---------------------------------------------------------------------------

pub(super) async fn list_sessions(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let sessions = list_sessions_for_client(&state, true).await;
    // The version counter is not tracked by the supervisor itself; we use 0
    // as a placeholder. A proper monotonic version can be added to the
    // supervisor later if clients need ETag-style cache validation.
    let environment_metadata = environment_list_response();
    Ok(Json(SessionListResponse {
        fleet_lens: build_fleet_lens_summary(&sessions),
        fleet_presets: environment_metadata.fleet_presets,
        sessions,
        version: 0,
        repo_themes: Default::default(),
        environments: environment_metadata.environments,
    }))
}

pub(super) async fn list_environments(
    Extension(auth): Extension<AuthInfo>,
) -> Result<Json<EnvironmentListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    Ok(Json(environment_list_response()))
}

fn environment_list_response() -> EnvironmentListResponse {
    EnvironmentListResponse {
        environments: remote_sessions::environment_summaries(true),
        fleet_presets: build_fleet_lens_presets(
            crate::session::overlay::default_overlay()
                .map(|overlay| overlay.all_fleet_presets())
                .unwrap_or_default(),
        ),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions
// ---------------------------------------------------------------------------

pub(super) async fn create_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    let cass_mode = match crate::types::resolve_cass_admission_mode(None) {
        Ok(mode) => mode,
        Err(error) => return cass_admission_error_response(CassAdmissionRouteError::from(error)),
    };
    create_session_with_cass_mode(&auth, &state, body, cass_mode).await
}

pub(super) async fn create_session_with_cass_mode(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    body: CreateSessionRequest,
    cass_mode: crate::types::CassAdmissionMode,
) -> Response {
    if cass_mode.is_enforce() {
        return cass_admission_error_response(CassAdmissionRouteError::Admission(
            CassAdmissionError::new(
                "reservation_mismatch",
                "enforce mode requires batch admission preflight",
            ),
        ));
    }
    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return create_remote_session_response(body).await;
    }

    create_local_session_response(auth, state, body).await
}

async fn create_remote_session_response(body: CreateSessionRequest) -> axum::response::Response {
    match remote_sessions::create_remote_session(body).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn create_local_session_response(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    body: CreateSessionRequest,
) -> axum::response::Response {
    let explicit_local_override = explicit_local_launch_override(body.launch_target.as_deref());
    match create_local_session(
        state,
        body.name,
        body.cwd,
        body.spawn_tool,
        body.tmux_target,
        body.initial_request,
    )
    .await
    {
        Ok(mut response) => {
            if explicit_local_override {
                mark_create_session_local_override(&mut response);
            }
            provider_create_session_response(auth, state, response).await
        }
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

async fn provider_create_session_response(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    response: CreateSessionResponse,
) -> Response {
    let (provider_receipt, session_generation) = match response.session.as_ref() {
        Some(session) => {
            let provider_receipt = state
                .supervisor
                .provider_launch_receipt(&session.session_id)
                .await;
            let session_generation = state
                .supervisor
                .exact_cleanup_generation(&session.session_id)
                .await;
            (provider_receipt, session_generation)
        }
        None => (None, None),
    };
    provider_create_session_response_with_receipt(
        auth,
        response,
        provider_receipt,
        session_generation.as_deref(),
    )
}

fn provider_create_session_response_with_receipt(
    auth: &AuthInfo,
    response: CreateSessionResponse,
    stored_receipt: Option<AuthorizedProviderResumeLaunchReceipt>,
    session_generation: Option<&str>,
) -> Response {
    let Some(launch_receipt) = response.launch_receipt.clone() else {
        return (StatusCode::CREATED, Json(response)).into_response();
    };
    let provider_resume = stored_receipt
        .as_ref()
        .map(|receipt| receipt.provider_resume().clone())
        .unwrap_or_else(|| {
            AuthorizedProviderResumeReceipt::unknown(
                ProviderResumeProvider::Unknown,
                ProviderResumeCaptureSource::Unknown,
            )
        });
    let receipt = AuthorizedProviderResumeLaunchReceipt::new(launch_receipt, provider_resume);
    let mut value = match serde_json::to_value(&response) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "failed to serialize create-session response");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some("failed to serialize create-session response".to_string()),
            );
        }
    };
    let Some(object) = value.as_object_mut() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some("invalid create-session response shape".to_string()),
        );
    };

    if auth.has_scope(AuthScope::StreamWrite) {
        let Some(session_generation) = session_generation else {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some("exact cleanup generation unavailable".to_string()),
            );
        };
        let mut authorized_receipt =
            serde_json::to_value(receipt).expect("provider receipt serialization cannot fail");
        authorized_receipt
            .as_object_mut()
            .expect("provider receipt must serialize as an object")
            .insert(
                "session_generation".to_string(),
                serde_json::Value::String(session_generation.to_string()),
            );
        object.insert("launch_receipt".to_string(), authorized_receipt);
    } else {
        object.remove("launch_receipt");
        object.insert(
            "provider_resume".to_string(),
            serde_json::to_value(receipt.public_projection())
                .expect("public provider projection serialization cannot fail"),
        );
    }
    (StatusCode::CREATED, Json(value)).into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/adopt
// ---------------------------------------------------------------------------

pub(super) async fn adopt_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<AdoptSessionRequest>,
) -> axum::response::Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match state
        .supervisor
        .adopt_tmux_session(body.tmux_name, body.tmux_target, body.session_id)
        .await
    {
        Ok(adopted) => (
            StatusCode::CREATED,
            Json(AdoptSessionResponse {
                session: adopted.session,
                repo_theme: adopted.repo_theme,
                reused_session_id: adopted.reused_session_id,
            }),
        )
            .into_response(),
        Err(error) => adopt_session_error_response(error),
    }
}

fn adopt_session_error_response(error: TmuxAdoptError) -> axum::response::Response {
    let (status, code) = match &error {
        TmuxAdoptError::EmptyTmuxName => (StatusCode::BAD_REQUEST, "TMUX_NAME_REQUIRED"),
        TmuxAdoptError::DiscoveryUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "TMUX_DISCOVERY_UNAVAILABLE",
        ),
        TmuxAdoptError::TargetNotFound { .. } => (StatusCode::NOT_FOUND, "TMUX_SESSION_NOT_FOUND"),
        TmuxAdoptError::AmbiguousTarget { .. } => (StatusCode::CONFLICT, "TMUX_SESSION_AMBIGUOUS"),
        TmuxAdoptError::AlreadyTracked { .. } => {
            (StatusCode::CONFLICT, "TMUX_SESSION_ALREADY_TRACKED")
        }
        TmuxAdoptError::StaleSessionNotFound { .. } => {
            (StatusCode::NOT_FOUND, "STALE_SESSION_NOT_FOUND")
        }
        TmuxAdoptError::StaleSessionConflict { .. } => {
            (StatusCode::CONFLICT, "STALE_SESSION_CONFLICT")
        }
        TmuxAdoptError::InvalidTarget { .. } => (StatusCode::BAD_REQUEST, "INVALID_TMUX_TARGET"),
        TmuxAdoptError::SpawnFailed { .. } => {
            tracing::error!("adopt_session failed: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, "TMUX_ADOPT_FAILED")
        }
    };

    error_response(status, code, Some(error.to_string()))
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/batch/admission
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CassReservationCancellationGuard {
    reservation_ids: Vec<String>,
}

impl CassReservationCancellationGuard {
    fn track(&mut self, reservation_id: String) {
        self.reservation_ids.push(reservation_id);
    }

    fn disarm(&mut self) {
        self.reservation_ids.clear();
    }
}

impl Drop for CassReservationCancellationGuard {
    fn drop(&mut self) {
        if self.reservation_ids.is_empty() {
            return;
        }
        let reservation_ids = std::mem::take(&mut self.reservation_ids);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                reservation_count = reservation_ids.len(),
                "Cass reservations dropped unresolved outside a Tokio runtime"
            );
            return;
        };
        runtime.spawn(async move {
            for reservation_id in reservation_ids {
                let result = crate::session::supervisor::run_configured_cass_admission_command(
                    crate::types::CassAdmissionCommandRequest::reconcile(
                        &reservation_id,
                        "transport_loss",
                    ),
                )
                .await;
                match result {
                    Ok(response)
                        if response.ok
                            && response.reservation_id.as_deref()
                                == Some(reservation_id.as_str())
                            && response.state.as_deref() == Some("unresolved") => {}
                    Ok(response) => tracing::warn!(
                        cass_error_code = response
                            .error
                            .as_ref()
                            .map(|error| error.code.as_str())
                            .unwrap_or("partial_result"),
                        "Cass cancellation reconciliation returned an unbound result"
                    ),
                    Err(error) => tracing::warn!(
                        cass_error_code = %error.code,
                        "failed to reconcile cancellation-dropped Cass reservation"
                    ),
                }
            }
        });
    }
}

pub(super) async fn admit_sessions_batch(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CassAdmissionPreflightRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    let _ = state;
    match admit_sessions_batch_inner(body).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => cass_admission_error_response(error),
    }
}

async fn admit_sessions_batch_inner(
    body: CassAdmissionPreflightRequest,
) -> Result<CassAdmissionPreflightResponse, CassAdmissionRouteError> {
    validate_sessions_batch_dirs(&body.dirs)
        .map_err(|error| CassAdmissionRouteError::validation(error.message()))?;
    if body.intents.is_empty() {
        return Err(CassAdmissionRouteError::validation(
            "admission preflight requires dirs and intents",
        ));
    }
    if body.dirs.len() != body.intents.len() {
        return Err(CassAdmissionRouteError::validation(
            "admission intents must match dirs one-for-one",
        ));
    }
    let batch_id = body
        .intents
        .first()
        .map(|intent| intent.batch_id.clone())
        .expect("non-empty admission intents checked above");
    if body
        .intents
        .iter()
        .any(|intent| intent.batch_id != batch_id)
    {
        return Err(CassAdmissionRouteError::validation(
            "admission intents must share one canonical batch_id",
        ));
    }
    let mode = crate::types::resolve_cass_admission_mode(body.cass_admission_mode)
        .map_err(CassAdmissionRouteError::from)?;
    let mut body = body;
    body.cass_admission_mode = Some(mode);
    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return remote_sessions::admit_remote_sessions_batch(body)
            .await
            .map_err(CassAdmissionRouteError::Remote);
    }
    if !mode.is_enforce() {
        return Ok(CassAdmissionPreflightResponse {
            target_id: "local".to_string(),
            batch_id,
            reservations: Vec::new(),
        });
    }
    if mode.is_enforce() && !crate::session::supervisor::cass_admission_command_is_configured() {
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "admission_command_unavailable",
            "target admission command unavailable in enforce mode",
        )));
    }
    let mut reservations = Vec::new();
    let mut cancellation_guard = CassReservationCancellationGuard::default();
    for (index, intent) in body.intents.into_iter().enumerate() {
        if intent.batch_index != index as u64 {
            let settlement =
                release_preflight_reservations(&cancellation_guard.reservation_ids).await;
            if settlement.is_ok() {
                cancellation_guard.disarm();
            }
            settlement?;
            return Err(CassAdmissionRouteError::validation(format!(
                "intent batch_index {0} must match dirs index {index}",
                intent.batch_index
            )));
        }
        match reserve_local_cass_intent(intent, index as u64).await {
            Ok(envelope) => {
                cancellation_guard.track(envelope.reservation_id.clone());
                reservations.push(envelope);
            }
            Err(error) => {
                let settlement = if is_explicit_pre_provider_failure(&error) {
                    release_preflight_reservations(&cancellation_guard.reservation_ids).await
                } else {
                    reconcile_preflight_reservations(
                        &cancellation_guard.reservation_ids,
                        "partial_result",
                    )
                    .await
                };
                if settlement.is_ok() {
                    cancellation_guard.disarm();
                }
                settlement?;
                return Err(error);
            }
        }
    }
    cancellation_guard.disarm();
    Ok(CassAdmissionPreflightResponse {
        target_id: "local".to_string(),
        batch_id,
        reservations,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
async fn reserve_local_cass_intent(
    intent: CassAdmissionIntent,
    index: u64,
) -> Result<CassAdmissionReservationEnvelope, CassAdmissionRouteError> {
    let response = crate::session::supervisor::run_configured_cass_admission_command(
        crate::types::CassAdmissionCommandRequest::reserve(intent.clone()),
    )
    .await
    .map_err(CassAdmissionRouteError::Admission)?;
    if !response.ok {
        let mut cancellation_guard = CassReservationCancellationGuard::default();
        if let Some(reservation_id) = response.reservation_id.clone() {
            cancellation_guard.track(reservation_id);
            let settlement = reconcile_preflight_reservations(
                &cancellation_guard.reservation_ids,
                "partial_result",
            )
            .await;
            if settlement.is_ok() {
                cancellation_guard.disarm();
            }
            settlement?;
        }
        let error = response
            .error
            .unwrap_or(crate::types::CassAdmissionCommandError {
                code: "reservation_failed".to_string(),
                message: "admission reserve refused".to_string(),
            });
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            error.code,
            error.message,
        )));
    }
    let reservation_id = response.reservation_id.ok_or_else(|| {
        CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "partial_result",
            "admission reserve returned no reservation_id",
        ))
    })?;
    let mut cancellation_guard = CassReservationCancellationGuard::default();
    cancellation_guard.track(reservation_id.clone());
    if response.state.as_deref() != Some("reserved")
        || response.batch_id.as_deref() != Some(intent.batch_id.as_str())
        || response.batch_index != Some(intent.batch_index)
    {
        let settlement =
            reconcile_preflight_reservations(&cancellation_guard.reservation_ids, "partial_result")
                .await;
        if settlement.is_ok() {
            cancellation_guard.disarm();
        }
        settlement?;
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "partial_result",
            "admission reserve returned an unbound result",
        )));
    }
    let envelope = CassAdmissionReservationEnvelope {
        schema_version: CASS_ADMISSION_RESERVATION_SCHEMA.to_string(),
        reservation_id,
        batch_id: intent.batch_id,
        batch_index: intent.batch_index,
        index,
        target_id: "local".to_string(),
    };
    if let Err(error) = CassAdmissionReservationEnvelope::from_value(
        &serde_json::to_value(&envelope).map_err(|_| {
            CassAdmissionRouteError::Admission(CassAdmissionError::new(
                "partial_result",
                "admission reserve returned an unreadable reservation envelope",
            ))
        })?,
    ) {
        let settlement =
            reconcile_preflight_reservations(&cancellation_guard.reservation_ids, "partial_result")
                .await;
        if settlement.is_ok() {
            cancellation_guard.disarm();
        }
        settlement?;
        return Err(CassAdmissionRouteError::Admission(error));
    }
    cancellation_guard.disarm();
    Ok(envelope)
}

#[cfg_attr(not(test), allow(dead_code))]
async fn release_preflight_reservations(
    reservation_ids: &[String],
) -> Result<(), CassAdmissionRouteError> {
    let mut first_error = None;
    for reservation_id in reservation_ids {
        let settled = async {
            let response = crate::session::supervisor::run_configured_cass_admission_command(
                crate::types::CassAdmissionCommandRequest::release(reservation_id),
            )
            .await
            .map_err(CassAdmissionRouteError::Admission)?;
            validate_preflight_settlement_response(&response, reservation_id, "released")
        }
        .await;
        if first_error.is_none() {
            first_error = settled.err();
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg_attr(not(test), allow(dead_code))]
async fn reconcile_preflight_reservations(
    reservation_ids: &[String],
    cause: &str,
) -> Result<(), CassAdmissionRouteError> {
    let mut first_error = None;
    for reservation_id in reservation_ids {
        let settled = async {
            let response = crate::session::supervisor::run_configured_cass_admission_command(
                crate::types::CassAdmissionCommandRequest::reconcile(reservation_id, cause),
            )
            .await
            .map_err(CassAdmissionRouteError::Admission)?;
            validate_preflight_settlement_response(&response, reservation_id, "unresolved")
        }
        .await;
        if first_error.is_none() {
            first_error = settled.err();
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn validate_preflight_settlement_response(
    response: &crate::types::CassAdmissionCommandResponse,
    reservation_id: &str,
    expected_state: &str,
) -> Result<(), CassAdmissionRouteError> {
    if !response.ok {
        let error = response
            .error
            .clone()
            .unwrap_or(crate::types::CassAdmissionCommandError {
                code: "partial_result".to_string(),
                message: "Cass settlement command was refused".to_string(),
            });
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            error.code,
            error.message,
        )));
    }
    if response.reservation_id.as_deref() != Some(reservation_id)
        || response.state.as_deref() != Some(expected_state)
    {
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "partial_result",
            "Cass settlement command returned an unbound result",
        )));
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn is_explicit_pre_provider_failure(error: &CassAdmissionRouteError) -> bool {
    match error {
        CassAdmissionRouteError::Admission(error) => {
            matches!(
                error.code.as_str(),
                "reservation_failed" | "reservation_mismatch" | "unknown_provider"
            )
        }
        CassAdmissionRouteError::Validation(_) => true,
        CassAdmissionRouteError::Remote(_) => false,
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/batch
// ---------------------------------------------------------------------------

pub(super) async fn create_sessions_batch(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionsBatchRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return create_remote_sessions_batch_response(body).await;
    }

    create_local_sessions_batch_response(state, body).await
}

pub(super) async fn create_sessions_batch_http(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CassAwareCreateSessionsBatchRequest>,
) -> Response {
    let CassAwareCreateSessionsBatchRequest { request, mut cass } = body;
    let cass_fields_present = cass_batch_fields_present(&cass);
    let mode = match crate::types::resolve_cass_admission_mode(cass.cass_admission_mode) {
        Ok(mode) => mode,
        Err(error) => return cass_admission_error_response(CassAdmissionRouteError::from(error)),
    };
    // Snapshot the strongest mode at the HTTP boundary. A process-wide config
    // change later in this request must never downgrade an enforce launch.
    cass.cass_admission_mode = Some(mode);
    if cass_fields_present || mode.is_enforce() {
        return create_sessions_batch_with_cass(auth, state, request, cass).await;
    }
    create_sessions_batch(Extension(auth), State(state), Json(request)).await
}

fn cass_batch_fields_present(cass: &CassBatchAdmissionAttachment) -> bool {
    cass.cass_batch_id.is_some()
        || cass.cass_admission_mode.is_some()
        || !cass.cass_reservations.is_empty()
        || cass.cass_preflight_target_id.is_some()
}

pub(super) async fn create_sessions_batch_with_cass(
    auth: AuthInfo,
    state: Arc<AppState>,
    body: CreateSessionsBatchRequest,
    mut cass: CassBatchAdmissionAttachment,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let mode = match crate::types::resolve_cass_admission_mode(cass.cass_admission_mode) {
        Ok(mode) => mode,
        Err(error) => return cass_admission_error_response(CassAdmissionRouteError::from(error)),
    };
    cass.cass_admission_mode = Some(mode);

    if let Err(error) = validate_cass_batch_reservations(&body, &cass) {
        return cass_admission_error_response(error);
    }

    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return remote_sessions_batch_result_response(
            remote_sessions::create_remote_sessions_batch_with_cass(body, cass).await,
        );
    }

    create_local_sessions_batch_with_cass(state, body, cass).await
}

fn validate_cass_batch_reservations(
    body: &CreateSessionsBatchRequest,
    cass: &CassBatchAdmissionAttachment,
) -> Result<(), CassAdmissionRouteError> {
    let mode = crate::types::resolve_cass_admission_mode(cass.cass_admission_mode)
        .map_err(CassAdmissionRouteError::from)?;
    if cass.cass_reservations.is_empty() {
        if !mode.is_enforce() {
            return Ok(());
        }
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "reservation_mismatch",
            "enforce mode requires target-bound Cass reservation IDs",
        )));
    }
    let batch_id = cass.cass_batch_id.as_deref().ok_or_else(|| {
        CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "reservation_mismatch",
            "enforce mode requires the canonical Cass batch_id from preflight",
        ))
    })?;
    if cass.cass_reservations.len() != body.dirs.len() {
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "reservation_mismatch",
            "Cass reservations must match batch dirs one-for-one",
        )));
    }
    let expected_target = resolved_admission_target(body.launch_target.as_deref());
    let preflight_target = cass.cass_preflight_target_id.as_deref().ok_or_else(|| {
        CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "reservation_mismatch",
            "Cass reservations require the target id returned by admission preflight",
        ))
    })?;
    if preflight_target != expected_target {
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "reservation_mismatch",
            "routing target changed between Cass preflight and batch create",
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (index, reservation) in cass.cass_reservations.iter().enumerate() {
        reservation
            .validate()
            .map_err(CassAdmissionRouteError::Admission)?;
        let reservation_index = reservation.index.unwrap_or(reservation.batch_index) as usize;
        if reservation.batch_id != batch_id
            || reservation_index != index
            || !seen.insert(reservation.reservation_id.clone())
        {
            return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
                "reservation_mismatch",
                "wrong, replayed, or foreign Cass reservation",
            )));
        }
        if reservation.target_id.as_deref() != Some("local") {
            return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
                "reservation_mismatch",
                "Cass reservation is not bound to the target-local admission authority",
            )));
        }
    }
    Ok(())
}

fn resolved_admission_target(launch_target: Option<&str>) -> String {
    launch_target
        .map(str::trim)
        .filter(|target| !target.is_empty() && !remote_sessions::is_local_launch_target(target))
        .unwrap_or("local")
        .to_string()
}

async fn create_local_sessions_batch_with_cass(
    state: Arc<AppState>,
    body: CreateSessionsBatchRequest,
    cass: CassBatchAdmissionAttachment,
) -> Response {
    let mode = match crate::types::resolve_cass_admission_mode(cass.cass_admission_mode) {
        Ok(mode) => mode,
        Err(error) => return cass_admission_error_response(CassAdmissionRouteError::from(error)),
    };
    if !mode.is_enforce() {
        if let Err(error) = release_non_enforce_reservations(&cass).await {
            return cass_admission_error_response(error);
        }
        return create_local_sessions_batch_response(state, body).await;
    }
    match create_request_scoped_cass_batch(state, body, cass).await {
        Ok(response) => create_sessions_batch_response(response),
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

async fn release_non_enforce_reservations(
    cass: &CassBatchAdmissionAttachment,
) -> Result<(), CassAdmissionRouteError> {
    if cass.cass_reservations.is_empty() {
        return Ok(());
    }
    if !crate::session::supervisor::cass_admission_command_is_configured() {
        return Err(CassAdmissionRouteError::Admission(CassAdmissionError::new(
            "admission_command_unavailable",
            "cannot settle attached Cass reservations before legacy launch",
        )));
    }
    let reservation_ids = cass
        .cass_reservations
        .iter()
        .map(|reservation| reservation.reservation_id.clone())
        .collect::<Vec<_>>();
    let mut cancellation_guard = CassReservationCancellationGuard { reservation_ids };
    let settlement = release_preflight_reservations(&cancellation_guard.reservation_ids).await;
    if settlement.is_ok() {
        cancellation_guard.disarm();
    }
    settlement
}

async fn create_request_scoped_cass_batch(
    state: Arc<AppState>,
    body: CreateSessionsBatchRequest,
    cass: CassBatchAdmissionAttachment,
) -> Result<CreateSessionsBatchResponse, crate::api::service::ApiServiceError> {
    validate_sessions_batch_dirs(&body.dirs)?;
    let total = body.dirs.len();
    let batch_id = cass
        .cass_batch_id
        .expect("validated enforce attachment has canonical batch_id");
    let (_, batch_label, batch_created_at, prompt_excerpt) =
        super::new_batch_context(total, body.initial_request.as_deref());
    let reservations = cass.cass_reservations;
    let tasks =
        body.dirs
            .into_iter()
            .zip(reservations)
            .enumerate()
            .map(|(index, (cwd, reservation))| {
                let supervisor = state.supervisor.clone();
                let initial_request = body.initial_request.clone();
                let tmux_target = body.tmux_target.clone();
                // Futures beyond the concurrency frontier may never be
                // polled if the request is canceled. Capture an armed guard
                // in every queued future until the supervisor takes over.
                let mut cancellation_guard = CassReservationCancellationGuard::default();
                cancellation_guard.track(reservation.reservation_id.clone());
                let batch = super::session_batch_membership(
                    batch_id.clone(),
                    batch_label.clone(),
                    index,
                    total,
                    batch_created_at,
                    prompt_excerpt.clone(),
                );
                let result_cwd = cwd.clone();
                async move {
                    // Once polled, finish settlement/rollback independently of
                    // the HTTP response future. Dropping the JoinHandle on a
                    // client disconnect does not cancel the launch task.
                    let launch = tokio::spawn(async move {
                        cancellation_guard.disarm();
                        supervisor
                            .create_session_with_target_batch_and_cass(
                                None,
                                Some(cwd),
                                body.spawn_tool,
                                initial_request,
                                tmux_target,
                                CassSessionAdmission::new(batch, reservation),
                            )
                            .await
                    });
                    let created = launch.await.unwrap_or_else(|_| {
                        Err(anyhow::Error::new(CassAdmissionError::new(
                            "provider_ambiguity",
                            "Cass launch task ended before a durable result",
                        )))
                    });
                    super::create_sessions_batch_result(index, result_cwd, created)
                }
            });
    let mut results: Vec<_> = stream::iter(tasks)
        .buffer_unordered(super::BATCH_CREATE_CONCURRENCY)
        .collect()
        .await;
    results.sort_by_key(|result| result.index);
    Ok(CreateSessionsBatchResponse { results })
}

#[allow(dead_code)]
async fn create_remote_sessions_batch_response(body: CreateSessionsBatchRequest) -> Response {
    remote_sessions_batch_result_response(remote_sessions::create_remote_sessions_batch(body).await)
}

fn remote_sessions_batch_result_response(
    result: Result<CreateSessionsBatchResponse, remote_sessions::RemoteSessionError>,
) -> Response {
    match result {
        Ok(response) => create_sessions_batch_response(response),
        Err(err) => err.into_response(),
    }
}

async fn create_local_sessions_batch_response(
    state: Arc<AppState>,
    body: CreateSessionsBatchRequest,
) -> Response {
    let explicit_local_override = explicit_local_launch_override(body.launch_target.as_deref());
    match create_local_sessions_batch(
        state,
        body.dirs,
        body.spawn_tool,
        body.tmux_target,
        body.initial_request,
    )
    .await
    {
        Ok(mut response) => {
            if explicit_local_override {
                mark_batch_local_override(&mut response);
            }
            create_sessions_batch_response(response)
        }
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

fn explicit_local_launch_override(target: Option<&str>) -> bool {
    target
        .map(str::trim)
        .is_some_and(|target| target.eq_ignore_ascii_case("local"))
}

fn mark_create_session_local_override(response: &mut crate::types::CreateSessionResponse) {
    if let Some(receipt) = response.launch_receipt.as_mut() {
        receipt.mark_local_override();
    }
}

fn mark_batch_local_override(response: &mut CreateSessionsBatchResponse) {
    for result in &mut response.results {
        if let Some(receipt) = result.launch_receipt.as_mut() {
            receipt.mark_local_override();
        }
    }
}

fn create_sessions_batch_response(response: CreateSessionsBatchResponse) -> Response {
    (create_sessions_batch_status(&response), Json(response)).into_response()
}

fn create_sessions_batch_status(response: &CreateSessionsBatchResponse) -> StatusCode {
    if response.results.iter().all(|result| result.ok) {
        StatusCode::CREATED
    } else {
        StatusCode::MULTI_STATUS
    }
}

#[allow(dead_code)]
enum CassAdmissionRouteError {
    Validation(String),
    Admission(CassAdmissionError),
    Remote(remote_sessions::RemoteSessionError),
}

impl CassAdmissionRouteError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

impl From<CassAdmissionError> for CassAdmissionRouteError {
    fn from(error: CassAdmissionError) -> Self {
        Self::Admission(error)
    }
}

fn cass_admission_error_response(error: CassAdmissionRouteError) -> Response {
    match error {
        CassAdmissionRouteError::Validation(message) => validation_error(message),
        CassAdmissionRouteError::Admission(error) => {
            let status = match error.code.as_str() {
                "admission_command_unavailable" | "provider_unavailable" => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
                "transport_loss" | "partial_result" | "provider_ambiguity"
                | "refinement_failure" => StatusCode::CONFLICT,
                _ => StatusCode::BAD_REQUEST,
            };
            tracing::warn!(cass_error_code = %error.code, "Cass admission request failed");
            error_response(
                status,
                cass_error_code(&error.code),
                Some(cass_public_error_message(&error.code).to_string()),
            )
        }
        CassAdmissionRouteError::Remote(error) => error.into_response(),
    }
}

pub(crate) fn cass_error_code(code: &str) -> &'static str {
    match code {
        "reservation_mismatch" | "reservation_failed" => "CASS_RESERVATION_FAILED",
        "admission_command_unavailable" | "provider_unavailable" => "CASS_ADMISSION_UNAVAILABLE",
        "unknown_mode" | "unknown_version" | "unknown_provider" | "document_invalid"
        | "malformed_uuid" | "malformed_index" | "oversized_chunk" | "secret_shaped_key" => {
            "CASS_ADMISSION_INVALID"
        }
        "transport_loss" | "partial_result" | "provider_ambiguity" | "refinement_failure" => {
            "CASS_ADMISSION_UNRESOLVED"
        }
        _ => "CASS_ADMISSION_FAILED",
    }
}

pub(crate) fn cass_public_error_message(code: &str) -> &'static str {
    match code {
        "reservation_mismatch" | "reservation_failed" => {
            "Cass reservation was rejected; run admission preflight again"
        }
        "admission_command_unavailable" | "provider_unavailable" => {
            "Cass admission is unavailable on the selected target"
        }
        "unknown_mode" | "unknown_version" | "unknown_provider" | "document_invalid"
        | "malformed_uuid" | "malformed_index" | "oversized_chunk" | "secret_shaped_key" => {
            "Cass admission input is invalid"
        }
        "transport_loss" | "partial_result" | "provider_ambiguity" | "refinement_failure" => {
            "Cass admission outcome is unresolved; no session was launched"
        }
        _ => "Cass admission failed; no session was launched",
    }
}

// ---------------------------------------------------------------------------
// DELETE /v1/sessions/{session_id}
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct DeleteSessionQuery {
    pub(super) mode: Option<String>,
    #[serde(alias = "generation")]
    pub(super) session_generation: Option<String>,
}

pub(super) async fn delete_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<DeleteSessionQuery>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let delete_mode = match parse_delete_session_mode(query.mode.as_deref()) {
        Ok(delete_mode) => delete_mode,
        Err(response) => return response,
    };

    delete_session_response(
        &auth,
        &state,
        &session_id,
        query.session_generation.as_deref(),
        delete_mode,
    )
    .await
}

#[allow(clippy::result_large_err)]
pub(super) fn parse_delete_session_mode(mode: Option<&str>) -> Result<SessionDeleteMode, Response> {
    match mode {
        None | Some("detach_bridge") => Ok(SessionDeleteMode::DetachBridge),
        Some("kill_tmux") => Ok(SessionDeleteMode::KillTmux),
        Some(other) => Err(validation_error(format!("invalid delete mode: {other}"))),
    }
}

async fn delete_session_response(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    session_id: &str,
    session_generation: Option<&str>,
    delete_mode: SessionDeleteMode,
) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::delete_remote_session(
                &target,
                remote_session_id,
                &delete_mode,
            )
            .await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let Some(session_generation) = session_generation else {
        return match state
            .supervisor
            .delete_session(session_id, delete_mode)
            .await
        {
            Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
            Err(error) => delete_session_error_response(error),
        };
    };
    if let Err(response) = auth.require_scope(AuthScope::StreamWrite) {
        return response;
    }

    match cleanup_exact_local_session(state, session_id, session_generation, delete_mode).await {
        Ok(receipt) => (StatusCode::OK, Json(receipt)).into_response(),
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

pub(super) fn delete_session_error_response(error: anyhow::Error) -> Response {
    let msg = error.to_string();
    // "not found" identifies a genuinely untracked session, but a failed tmux
    // kill on a *live* session can surface "not found" in tmux's stderr too —
    // that must stay a 500, not a misleading 404 that claims the session is gone
    // while it is still tracked and running.
    let session_missing = msg.contains("not found") && !msg.starts_with("tmux kill-session failed");
    if session_missing {
        error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None)
    } else {
        tracing::error!("delete_session failed: {error}");
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some(msg),
        )
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/attention/dismiss
// ---------------------------------------------------------------------------

pub(super) async fn dismiss_attention(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    match remote_sessions::denamespace_for_target(&session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::dismiss_remote_attention(&target, remote_session_id).await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::error!("[session {session_id}] dismiss_attention send failed: {e}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some(e.to_string()),
        );
    }

    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/input
// ---------------------------------------------------------------------------

fn validation_error(message: impl Into<String>) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "VALIDATION_FAILED",
        Some(message.into()),
    )
}

fn input_too_large_error() -> Response {
    error_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        "INPUT_TOO_LARGE",
        Some(format!(
            "terminal input exceeds {MAX_SESSION_INPUT_BYTES} byte limit"
        )),
    )
}

pub(super) fn error_response(
    status: StatusCode,
    code: impl Into<String>,
    message: Option<String>,
) -> Response {
    (status, Json(error_body(code, message))).into_response()
}

pub(super) async fn send_input(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SessionInputRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    send_input_response(&state, session_id, body).await
}

async fn send_input_response(
    state: &Arc<AppState>,
    session_id: String,
    body: SessionInputRequest,
) -> Response {
    if body.text.is_empty() {
        return validation_error("text must not be empty");
    }
    if body.text.len() > MAX_SESSION_INPUT_BYTES {
        return input_too_large_error();
    }
    if body.submit && body.text.trim().is_empty() {
        return validation_error("submitted text must not be blank");
    }

    match remote_sessions::denamespace_for_target(&session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::send_remote_input(&target, remote_session_id, body).await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    match deliver_session_input(state, &session_id, body).await {
        Ok(delivery) => session_input_delivery_response(session_id, delivery),
        Err(response) => response,
    }
}

async fn deliver_session_input(
    state: &Arc<AppState>,
    session_id: &str,
    body: SessionInputRequest,
) -> Result<InputDeliveryResult, Response> {
    let handle = match writable_session_handle(state, session_id).await {
        Ok(handle) => handle,
        Err(response) => return Err(response),
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    let command = session_input_command(body, ack_tx);
    send_session_input_command(session_id, &handle, command).await?;
    wait_for_input_delivery(ack_rx).await
}

async fn send_session_input_command(
    session_id: &str,
    handle: &ActorHandle,
    command: SessionCommand,
) -> Result<(), Response> {
    if let Err(err) = handle.send(command).await {
        tracing::error!("[session {session_id}] send_input failed: {err}");
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            Some(err.to_string()),
        ));
    }

    Ok(())
}

pub(super) fn session_input_delivery_response(
    session_id: String,
    delivery: InputDeliveryResult,
) -> Response {
    if !delivery.delivered {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "INPUT_DELIVERY_FAILED",
            delivery.message,
        );
    }

    (
        StatusCode::OK,
        Json(SessionInputResponse {
            ok: true,
            session_id,
            delivered: true,
            // ok/delivered stay true (some-vs-none contract), but a partial
            // submit is flagged so a caller needing an all-or-nothing delivery
            // can retry without the response's ok flipping (swimmers-bjsu).
            partial: delivery.is_partial(),
            delivery_method: Some(delivery.method.to_string()),
            // Carries the partial-delivery warning ("...may not have been fully
            // submitted") so a 200 isn't silently mistaken for a complete submit.
            message: delivery.message,
        }),
    )
        .into_response()
}

async fn writable_session_handle(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<ActorHandle, Response> {
    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "SESSION_NOT_FOUND",
                None,
            ));
        }
        Err(err) => {
            tracing::error!("send_input summary lookup failed: {err}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            ));
        }
    };

    if summary.state == SessionState::Exited {
        return Err(error_response(
            StatusCode::CONFLICT,
            "SESSION_EXITED",
            Some("session has already exited".to_string()),
        ));
    }

    state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None))
}

fn session_input_command(
    body: SessionInputRequest,
    ack: oneshot::Sender<InputDeliveryResult>,
) -> SessionCommand {
    if body.submit {
        SessionCommand::SubmitLineAck {
            text: body.text,
            ack,
        }
    } else {
        SessionCommand::WriteInputAck {
            data: body.text.into_bytes(),
            ack,
        }
    }
}

async fn wait_for_input_delivery(
    ack_rx: oneshot::Receiver<InputDeliveryResult>,
) -> Result<InputDeliveryResult, Response> {
    match tokio::time::timeout(INPUT_DELIVERY_ACK_TIMEOUT, ack_rx).await {
        Ok(Ok(delivery)) => Ok(delivery),
        Ok(Err(_)) => Err(error_response(
            StatusCode::BAD_GATEWAY,
            "INPUT_DELIVERY_UNKNOWN",
            Some("session actor dropped input delivery ack".to_string()),
        )),
        Err(_) => Err(error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "INPUT_DELIVERY_TIMEOUT",
            Some("timed out waiting for input delivery confirmation".to_string()),
        )),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/snapshot
// ---------------------------------------------------------------------------

pub(super) async fn get_snapshot(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match snapshot_route(&session_id) {
        Ok(SnapshotRoute::Remote {
            target,
            remote_session_id,
        }) => remote_snapshot_response(&target, remote_session_id).await,
        Ok(SnapshotRoute::Local) => local_snapshot_response(&state, &session_id).await,
        Err(err) => err.into_response(),
    }
}

enum SnapshotRoute<'a> {
    Remote {
        target: Box<LaunchTargetSummary>,
        remote_session_id: &'a str,
    },
    Local,
}

fn snapshot_route(
    session_id: &str,
) -> Result<SnapshotRoute<'_>, remote_sessions::RemoteSessionError> {
    Ok(match remote_sessions::denamespace_for_target(session_id)? {
        Some((target, remote_session_id)) => SnapshotRoute::Remote {
            target: Box::new(target),
            remote_session_id,
        },
        None => SnapshotRoute::Local,
    })
}

pub(super) async fn remote_snapshot_response(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Response {
    match remote_sessions::fetch_remote_snapshot(target, remote_session_id).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn local_snapshot_response(state: &Arc<AppState>, session_id: &str) -> Response {
    let handle = match state.supervisor.get_session(session_id).await {
        Some(h) => h,
        None => {
            return error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
    };

    snapshot_request_response(request_terminal_snapshot(&handle).await)
}

pub(super) async fn request_terminal_snapshot(
    handle: &ActorHandle,
) -> Result<TerminalSnapshot, SnapshotRequestError> {
    let (tx, rx) = oneshot::channel::<TerminalSnapshot>();
    if handle.send(SessionCommand::GetSnapshot(tx)).await.is_err() {
        return Err(SnapshotRequestError::ActorUnavailable);
    }

    match tokio::time::timeout(SNAPSHOT_TIMEOUT, rx).await {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(_)) => Err(SnapshotRequestError::ReplyDropped),
        Err(_) => Err(SnapshotRequestError::Timeout),
    }
}

fn snapshot_request_response(result: Result<TerminalSnapshot, SnapshotRequestError>) -> Response {
    match result {
        Ok(snapshot) => (StatusCode::OK, Json(snapshot)).into_response(),
        Err(error) => snapshot_error_response(error),
    }
}

pub(super) fn snapshot_error_response(error: SnapshotRequestError) -> Response {
    let detail = match error {
        SnapshotRequestError::ActorUnavailable => "session actor unavailable",
        SnapshotRequestError::ReplyDropped => "actor dropped snapshot reply",
        SnapshotRequestError::Timeout => "snapshot request timed out",
    };
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
        Some(detail.to_string()),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SnapshotRequestError {
    ActorUnavailable,
    ReplyDropped,
    Timeout,
}

#[cfg(test)]
mod provider_resume_integration_tests {
    use super::*;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::{Config, SessionDeleteMode};
    use crate::persistence::file_store::FileStore;
    use crate::session::supervisor::{ExactSessionCleanupOutcome, SessionSupervisor};
    use crate::types::{
        LaunchReceipt, ProviderResumeCapability, SpawnTool, PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION,
    };
    use axum::body::to_bytes;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use tempfile::tempdir;

    struct EnvRestore {
        key: &'static str,
        value: Option<OsString>,
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.value.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    struct TmuxCleanup {
        name: String,
    }

    impl Drop for TmuxCleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("tmux")
                .args(["kill-session", "-t", &format!("={}", self.name)])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    fn response_fixture() -> CreateSessionResponse {
        CreateSessionResponse {
            session: None,
            repo_theme: None,
            launch_receipt: Some(LaunchReceipt::local(
                "/workspace/swimmers",
                "swimmers-session-22",
                false,
            )),
        }
    }

    fn receipt_fixture() -> AuthorizedProviderResumeLaunchReceipt {
        AuthorizedProviderResumeLaunchReceipt::new(
            response_fixture().launch_receipt.expect("launch receipt"),
            AuthorizedProviderResumeReceipt::resumable(
                ProviderResumeProvider::Codex,
                "thread-authoritative-22",
                vec![
                    "codex".to_string(),
                    "resume".to_string(),
                    "thread-authoritative-22".to_string(),
                ],
                "codex resume thread-authoritative-22",
                ProviderResumeCaptureSource::ProviderResponse,
            )
            .expect("valid Codex receipt"),
        )
    }

    async fn response_value(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("response JSON")
    }

    #[tokio::test]
    async fn provider_resume_integration_authorized_response_is_versioned_and_legacy_readable() {
        let response = provider_create_session_response_with_receipt(
            &AuthInfo::new(OPERATOR_SCOPES.to_vec()),
            response_fixture(),
            Some(receipt_fixture()),
            Some("generation-authorized"),
        );
        assert_eq!(response.status(), StatusCode::CREATED);
        let value = response_value(response).await;
        assert_eq!(
            value["launch_receipt"]["version"],
            PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION
        );
        assert_eq!(
            value["launch_receipt"]["provider_resume"]["conversation_id"],
            "thread-authoritative-22"
        );
        assert_eq!(
            value["launch_receipt"]["session_generation"],
            "generation-authorized"
        );
        let legacy: CreateSessionResponse =
            serde_json::from_value(value).expect("legacy CreateSessionResponse reader");
        assert_eq!(
            legacy
                .launch_receipt
                .and_then(|receipt| receipt.session_id)
                .as_deref(),
            Some("swimmers-session-22")
        );
    }

    #[tokio::test]
    async fn provider_resume_integration_public_projection_omits_private_identity() {
        let response = provider_create_session_response_with_receipt(
            &AuthInfo::new(vec![AuthScope::SessionsWrite]),
            response_fixture(),
            Some(receipt_fixture()),
            Some("generation-must-stay-private"),
        );
        let value = response_value(response).await;
        assert!(value.get("launch_receipt").is_none());
        assert_eq!(
            value["provider_resume"]["capability"],
            serde_json::to_value(ProviderResumeCapability::Unknown).expect("capability")
        );
        let encoded = serde_json::to_string(&value).expect("encode response");
        for private in [
            "conversation_id",
            "resume_argv",
            "resume_command",
            "session_generation",
        ] {
            assert!(
                !encoded.contains(private),
                "public response leaked {private}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_resume_integration_grok_adapter_tmux_persistence_api_path() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let fixture = dir.path().join("grok-fixture");
        std::fs::write(&fixture, "#!/bin/sh\nsleep 5\n").expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-provider-integration-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let config = Arc::new(Config::default());
        let supervisor =
            SessionSupervisor::new_with_provider_receipt_data_dir(config, dir.path().join("data"));
        let (session, _) = supervisor
            .create_session(
                Some(tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("provider launch");
        let receipt = supervisor
            .provider_launch_receipt(&session.session_id)
            .await
            .expect("provider receipt");
        assert!(receipt.provider_resume().is_resumable());
        assert_eq!(
            receipt.provider_resume().provider(),
            ProviderResumeProvider::Grok
        );
        assert_eq!(
            receipt.provider_resume().capture_source(),
            ProviderResumeCaptureSource::Preassigned
        );

        let durable_files =
            std::fs::read_dir(dir.path().join("data").join("provider_resume_receipts"))
                .expect("durable receipt directory")
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
                .count();
        assert_eq!(durable_files, 1);

        let response = provider_create_session_response_with_receipt(
            &AuthInfo::new(OPERATOR_SCOPES.to_vec()),
            CreateSessionResponse {
                launch_receipt: Some(LaunchReceipt::local(
                    session.cwd.clone(),
                    session.session_id.clone(),
                    false,
                )),
                session: Some(session.clone()),
                repo_theme: None,
            },
            Some(receipt.clone()),
            supervisor
                .exact_cleanup_generation(&session.session_id)
                .await
                .as_deref(),
        );
        let value = response_value(response).await;
        assert_eq!(
            value["launch_receipt"]["provider_resume"]["conversation_id"],
            receipt
                .provider_resume()
                .conversation_id()
                .expect("conversation id")
        );

        supervisor
            .delete_session(&session.session_id, SessionDeleteMode::KillTmux)
            .await
            .expect("delete exact fixture tmux");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exact_session_cleanup_provider_history_survives_tmux_kill_and_resume_is_available() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let history_dir = dir.path().join("provider-history");
        std::fs::create_dir(&history_dir).expect("history directory");
        let fixture = dir.path().join("grok-history-fixture");
        let history_arg = crate::launcher::shell_single_quote(&history_dir.to_string_lossy());
        std::fs::write(
            &fixture,
            format!(
                r#"#!/bin/sh
history_dir={history_arg}
if [ "$1" = "--resume" ]; then
  test -f "$history_dir/$2"
  exit $?
fi
conversation_id=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --session-id)
      conversation_id="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
test -n "$conversation_id" || exit 31
: > "$history_dir/$conversation_id"
sleep 5
"#
            ),
        )
        .expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-exact-cleanup-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let data_dir = dir.path().join("data");
        let supervisor = SessionSupervisor::new_with_provider_receipt_data_dir(
            Arc::new(Config::default()),
            data_dir.clone(),
        );
        let (session, _) = supervisor
            .create_session(
                Some(tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("provider launch");
        let generation = supervisor
            .exact_cleanup_generation(&session.session_id)
            .await
            .expect("cleanup generation");
        let before = supervisor
            .provider_launch_receipt(&session.session_id)
            .await
            .expect("provider receipt before cleanup");

        let cleanup = supervisor
            .delete_exact_session(
                &session.session_id,
                &generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect("exact cleanup");
        assert_eq!(cleanup.outcome, ExactSessionCleanupOutcome::Deleted);
        assert!(!cleanup.tmux_session_alive);
        assert!(supervisor.get_session(&session.session_id).await.is_none());

        let after = supervisor
            .provider_launch_receipt(&session.session_id)
            .await
            .expect("provider receipt survives cleanup");
        assert_eq!(after, before);
        let durable_files =
            std::fs::read_dir(dir.path().join("data").join("provider_resume_receipts"))
                .expect("durable receipt directory")
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
                .count();
        assert_eq!(durable_files, 1);

        let resume_argv = after
            .provider_resume()
            .resume_argv()
            .expect("exact resume argv survives");
        let resume = std::process::Command::new(&resume_argv[0])
            .args(&resume_argv[1..])
            .current_dir(dir.path())
            .status()
            .expect("execute exact provider resume");
        assert!(resume.success(), "provider history missing after tmux kill");

        let restarted = SessionSupervisor::new_with_provider_receipt_data_dir(
            Arc::new(Config::default()),
            data_dir,
        );
        let registry_store = FileStore::new(dir.path().join("registry"))
            .await
            .expect("restart registry store");
        restarted.init_persistence(registry_store).await;
        let replacement_tmux_name = format!("swimmers-post-restart-{}", uuid::Uuid::new_v4());
        let _replacement_tmux_cleanup = TmuxCleanup {
            name: replacement_tmux_name.clone(),
        };
        let (replacement, _) = restarted
            .create_session(
                Some(replacement_tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("post-restart provider launch");
        assert_ne!(
            replacement.session_id, session.session_id,
            "durable cleanup tombstone must reserve the old session id"
        );
        assert_eq!(
            restarted.provider_launch_receipt(&session.session_id).await,
            Some(after.clone()),
            "new launch must not overwrite old provider resume receipt"
        );
        let repeat = restarted
            .delete_exact_session(
                &session.session_id,
                &generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect("idempotent cleanup repeat");
        assert_eq!(repeat.outcome, ExactSessionCleanupOutcome::AlreadyGone);
        assert_eq!(
            restarted.provider_launch_receipt(&session.session_id).await,
            Some(after)
        );
        let replacement_generation = restarted
            .exact_cleanup_generation(&replacement.session_id)
            .await
            .expect("replacement cleanup generation");
        restarted
            .delete_exact_session(
                &replacement.session_id,
                &replacement_generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect("replacement exact cleanup");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exact_session_cleanup_stale_generation_cannot_kill_same_name_replacement() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let fixture = dir.path().join("grok-reuse-fixture");
        std::fs::write(&fixture, "#!/bin/sh\nsleep 5\n").expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-exact-reuse-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let supervisor = SessionSupervisor::new_with_provider_receipt_data_dir(
            Arc::new(Config::default()),
            dir.path().join("data"),
        );
        let (session, _) = supervisor
            .create_session(
                Some(tmux_name.clone()),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("original provider launch");
        let generation = supervisor
            .exact_cleanup_generation(&session.session_id)
            .await
            .expect("original generation");
        supervisor
            .delete_session(&session.session_id, SessionDeleteMode::DetachBridge)
            .await
            .expect("detach original actor");

        let killed = std::process::Command::new("tmux")
            .args(["kill-session", "-t", &format!("={tmux_name}")])
            .status()
            .expect("kill original tmux");
        assert!(killed.success());
        let replacement = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &tmux_name, "sleep 5"])
            .status()
            .expect("spawn same-name replacement");
        assert!(replacement.success());

        let error = supervisor
            .delete_exact_session(
                &session.session_id,
                &generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect_err("stale authority must reject replacement incarnation");
        assert_eq!(
            error,
            crate::session::supervisor::ExactSessionCleanupError::TmuxIncarnationMismatch
        );
        let replacement_alive = std::process::Command::new("tmux")
            .args(["has-session", "-t", &format!("={tmux_name}")])
            .status()
            .expect("probe replacement tmux");
        assert!(replacement_alive.success());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_resume_integration_rejected_grok_never_persists_or_returns_success() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let fixture = dir.path().join("grok-reject-fixture");
        std::fs::write(&fixture, "#!/bin/sh\nexit 29\n").expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-provider-reject-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let config = Arc::new(Config::default());
        let supervisor =
            SessionSupervisor::new_with_provider_receipt_data_dir(config, dir.path().join("data"));
        let error = supervisor
            .create_session(
                Some(tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect_err("rejected Grok launch must fail");
        assert!(
            error
                .to_string()
                .contains("did not remain running long enough"),
            "{error:#}"
        );
        let receipt_dir = dir.path().join("data").join("provider_resume_receipts");
        let durable_files = std::fs::read_dir(receipt_dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(durable_files, 0);
    }
}
