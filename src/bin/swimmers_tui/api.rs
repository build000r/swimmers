use super::*;
use swimmers::api::remote_sessions::{encode_path_segment, is_remote_launch_target};
use swimmers::openrouter_models::{
    cached_or_default_openrouter_candidates, refresh_openrouter_model_cache,
};
use swimmers::thought::probe::{run_thought_config_probe, ThoughtConfigProbeResult};
pub(crate) use swimmers::types::ThoughtConfigResponse;

pub(crate) type ThoughtConfigTestResponse = ThoughtConfigProbeResult;

pub(crate) struct ApiClient {
    pub(crate) http: Client,
    pub(crate) startup_http: Client,
    pub(crate) base_url: String,
    pub(crate) auth_token: Option<String>,
    pub(crate) startup_wait_timeout: Duration,
    pub(crate) startup_retry_interval: Duration,
}

enum StartupAccessError {
    Retryable(String),
    Fatal(String),
}

enum ThoughtConfigTestRemoteError {
    LocalFallback,
    Message(String),
}

impl StartupAccessError {
    fn into_string(self) -> String {
        match self {
            Self::Retryable(message) | Self::Fatal(message) => message,
        }
    }
}

impl ApiClient {
    pub(crate) fn normalize_base_url(base_url: impl AsRef<str>) -> String {
        let trimmed = base_url.as_ref().trim();
        let normalized = trimmed.trim_end_matches('/');
        if normalized.is_empty() {
            trimmed.to_string()
        } else {
            normalized.to_string()
        }
    }

    fn build_http_client(timeout: Duration) -> Result<Client, String> {
        Client::builder()
            .connect_timeout(API_CONNECT_TIMEOUT)
            .timeout(timeout)
            .build()
            .map_err(|err| format!("failed to build http client: {err}"))
    }

    pub(crate) fn from_env() -> Result<Self, String> {
        // External mode targets a separate server that runs its own startup
        // enforcement (main.rs `prepare_server_startup`), so we do NOT exit here
        // on config errors — a token-auth remote backend is a valid target. But
        // `Config::from_env()` silently discarded the diagnostics, hiding e.g.
        // an unknown AUTH_MODE or a token mode missing AUTH_TOKEN. Surface them
        // so the operator sees the same warnings the standalone server prints.
        let load = Config::from_env_report();
        swimmers::cli::print_config_diagnostics(&load.diagnostics);
        let config = load.config;
        let base_url = std::env::var("SWIMMERS_TUI_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", config.port));
        let base_url = Self::normalize_base_url(base_url);
        let auth_token = match config.auth_mode {
            AuthMode::Token => config.auth_token,
            AuthMode::LocalTrust | AuthMode::TailnetTrust => None,
        };
        let http = Self::build_http_client(API_REQUEST_TIMEOUT)?;
        let startup_http = Self::build_http_client(API_STARTUP_REQUEST_TIMEOUT)?;

        Ok(Self {
            http,
            startup_http,
            base_url,
            auth_token,
            startup_wait_timeout: API_STARTUP_WAIT_TIMEOUT,
            startup_retry_interval: API_STARTUP_RETRY_INTERVAL,
        })
    }

    pub(crate) fn with_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    fn display_base_url(&self) -> String {
        redacted_backend_url(&self.base_url)
    }

    fn redact_backend_url_text(&self, text: &str) -> String {
        redact_backend_url_text(&self.base_url, text)
    }

    pub(crate) fn transport_error(&self, action: &str, err: reqwest::Error) -> String {
        let detail = self.redact_backend_url_text(&root_error_message(&err));
        let display_url = self.display_base_url();
        tracing::warn!(
            url = %display_url,
            action,
            is_timeout = err.is_timeout(),
            is_connect = err.is_connect(),
            is_request = err.is_request(),
            status = ?err.status(),
            detail = %detail,
            "tui http transport error"
        );
        friendly_transport_error_with_detail(&display_url, action, &err, &detail)
    }

    pub(crate) fn targets_local_backend(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };
        match url.host_str() {
            Some("localhost") => true,
            Some(host) => host
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false),
            None => false,
        }
    }

    pub(crate) fn startup_access_error(&self, path: &str, status: reqwest::StatusCode) -> String {
        let base_url = self.display_base_url();
        match status {
            reqwest::StatusCode::UNAUTHORIZED => format!(
                "backend at {} requires valid auth for {}. Set AUTH_MODE=token and AUTH_TOKEN to match the target API.",
                base_url, path
            ),
            reqwest::StatusCode::FORBIDDEN => format!(
                "backend at {} denied startup access to {}. Use a token with the required session scope for this TUI instance.",
                base_url, path
            ),
            _ => format!(
                "backend at {} rejected startup access to {} ({status})",
                base_url, path
            ),
        }
    }

    fn startup_transport_error(&self, action: &str, err: reqwest::Error) -> StartupAccessError {
        let retryable = err.is_connect() || err.is_timeout();
        // transport_error already emits a structured warn; this just labels
        // whether the preflight loop will retry or give up.
        tracing::debug!(action, retryable, "startup transport error classified");
        let message = self.transport_error(action, err);
        if retryable {
            StartupAccessError::Retryable(message)
        } else {
            StartupAccessError::Fatal(message)
        }
    }

    async fn ensure_startup_access_probe(
        &self,
        response: reqwest::Response,
        path: &str,
    ) -> Result<(), StartupAccessError> {
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(
                StartupAccessError::Fatal(self.startup_access_error(path, status)),
            ),
            _ => Err(StartupAccessError::Fatal(read_error(response).await)),
        }
    }

    async fn preflight_session_refresh_access_with_client(
        &self,
        http: &Client,
    ) -> Result<(), StartupAccessError> {
        let url = format!("{}/v1/sessions", self.base_url);
        tracing::debug!(
            url = %redacted_backend_url(&url),
            "preflight: GET /v1/sessions"
        );
        let response = self
            .with_auth(http.get(url))
            .send()
            .await
            .map_err(|err| self.startup_transport_error("refresh sessions", err))?;

        self.ensure_startup_access_probe(response, "/v1/sessions")
            .await
    }

    async fn preflight_session_refresh_access(&self) -> Result<(), String> {
        self.preflight_session_refresh_access_with_client(&self.http)
            .await
            .map_err(StartupAccessError::into_string)
    }

    async fn preflight_selection_sync_access_with_client(
        &self,
        http: &Client,
    ) -> Result<(), StartupAccessError> {
        let url = format!("{}/v1/selection", self.base_url);
        tracing::debug!(
            url = %redacted_backend_url(&url),
            "preflight: PUT /v1/selection (clear)"
        );
        let response = self
            .with_auth(http.put(url))
            .json(&PublishSelectionRequest { session_id: None })
            .send()
            .await
            .map_err(|err| self.startup_transport_error("clear the published selection", err))?;

        self.ensure_startup_access_probe(response, "/v1/selection")
            .await
    }

    async fn preflight_selection_sync_access(&self) -> Result<(), String> {
        self.preflight_selection_sync_access_with_client(&self.http)
            .await
            .map_err(StartupAccessError::into_string)
    }

    async fn wait_for_local_startup_probe<F, Fut>(
        &self,
        deadline: Instant,
        mut probe: F,
    ) -> Result<(), String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<(), StartupAccessError>>,
    {
        let started = Instant::now();
        let mut attempt: u32 = 0;
        let display_url = self.display_base_url();
        loop {
            attempt += 1;
            match probe().await {
                Ok(()) => {
                    tracing::info!(
                        attempt,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        url = %display_url,
                        "preflight probe ready"
                    );
                    return Ok(());
                }
                Err(StartupAccessError::Fatal(message)) => {
                    tracing::error!(
                        attempt,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        url = %display_url,
                        message = %message,
                        "preflight probe failed (fatal)"
                    );
                    return Err(message);
                }
                Err(StartupAccessError::Retryable(message)) => {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    if Instant::now() >= deadline {
                        tracing::error!(
                            attempt,
                            elapsed_ms,
                            url = %display_url,
                            message = %message,
                            "preflight probe deadline exceeded"
                        );
                        return Err(message);
                    }
                    tracing::warn!(
                        attempt,
                        elapsed_ms,
                        url = %display_url,
                        message = %message,
                        "preflight probe retrying"
                    );
                }
            }
            tokio::time::sleep(self.startup_retry_interval).await;
        }
    }

    async fn preflight_local_startup_access(&self) -> Result<(), String> {
        let deadline = Instant::now() + self.startup_wait_timeout;
        self.wait_for_local_startup_probe(deadline, || {
            self.preflight_session_refresh_access_with_client(&self.startup_http)
        })
        .await?;
        self.wait_for_local_startup_probe(deadline, || {
            self.preflight_selection_sync_access_with_client(&self.startup_http)
        })
        .await
    }

    pub(crate) async fn preflight_startup_access(&self) -> Result<(), String> {
        let local = self.targets_local_backend();
        tracing::info!(
            url = %self.display_base_url(),
            local,
            wait_timeout_ms = self.startup_wait_timeout.as_millis() as u64,
            "preflight startup access begin"
        );
        if local {
            return self.preflight_local_startup_access().await;
        }
        self.preflight_session_refresh_access().await?;
        self.preflight_selection_sync_access().await?;
        Ok(())
    }

    async fn local_test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> Result<ThoughtConfigTestResponse, String> {
        let config = config
            .normalize_and_validate()
            .map_err(|err| err.to_string())?;
        Ok(run_thought_config_probe(&config).await)
    }

    async fn refresh_openrouter_candidates_inner(&self) -> Result<Vec<String>, String> {
        refresh_openrouter_model_cache(&self.http)
            .await
            .map(|cache| cache.models)
    }

    async fn fetch_environment_metadata_from_sessions(
        &self,
    ) -> Result<EnvironmentListResponse, String> {
        let url = format!("{}/v1/sessions", self.base_url);
        let response = self
            .with_auth(self.http.get(url).timeout(API_SESSION_LIST_TIMEOUT))
            .send()
            .await
            .map_err(|err| self.transport_error("refresh environments", err))?;

        if response.status().is_success() {
            let payload = response
                .json::<SessionListResponse>()
                .await
                .map_err(|err| format!("failed to parse environments response: {err}"))?;
            return Ok(EnvironmentListResponse {
                environments: payload.environments,
                fleet_presets: payload.fleet_presets,
            });
        }

        Err(read_error(response).await)
    }

    async fn send_thought_config_test(
        &self,
        config: &ThoughtConfig,
    ) -> Result<reqwest::Response, ThoughtConfigTestRemoteError> {
        let url = format!("{}/v1/thought-config/test", self.base_url);
        self.with_auth(self.http.post(url))
            .json(config)
            .send()
            .await
            .map_err(|err| {
                if self.targets_local_backend() {
                    ThoughtConfigTestRemoteError::LocalFallback
                } else {
                    ThoughtConfigTestRemoteError::Message(
                        self.transport_error("test thought config", err),
                    )
                }
            })
    }

    async fn decode_thought_config_test_response(
        &self,
        response: reqwest::Response,
        config: ThoughtConfig,
    ) -> Result<ThoughtConfigTestResponse, String> {
        if response.status().is_success() {
            return response
                .json::<ThoughtConfigTestResponse>()
                .await
                .map_err(|err| format!("failed to parse thought config test: {err}"));
        }

        if response.status() == reqwest::StatusCode::NOT_FOUND && self.targets_local_backend() {
            return self.local_test_thought_config(config).await;
        }

        Err(read_error(response).await)
    }

    fn personal_workflows_route_missing_message(&self, route: &str, feature: &str) -> String {
        format!(
            "backend at {} does not expose {route}. {feature} requires SWIMMERS_PERSONAL_WORKFLOWS=1 on the target backend; if this is your local server, relaunch via `make up` or `make tui`.",
            self.display_base_url()
        )
    }

    fn session_skills_missing_message(&self) -> String {
        format!(
            "backend at {} does not expose session skills. Skill context requires SWIMMERS_PERSONAL_WORKFLOWS=1 on the target backend; if this is your local server, relaunch via `make up` or `make tui`.",
            self.display_base_url()
        )
    }
}

async fn decode_personal_workflows_response<T>(
    response: reqwest::Response,
    parse_error: &str,
    missing_route_message: String,
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    if response.status().is_success() {
        return response
            .json::<T>()
            .await
            .map_err(|err| format!("{parse_error}: {err}"));
    }

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(missing_route_message);
    }

    Err(read_error(response).await)
}

async fn decode_native_mode_response(
    response: reqwest::Response,
    base_url: &str,
) -> Result<NativeDesktopStatusResponse, String> {
    if response.status().is_success() {
        return response
            .json::<NativeDesktopStatusResponse>()
            .await
            .map_err(|err| format!("failed to parse terminal handoff status: {err}"));
    }

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        let base_url = redacted_backend_url(base_url);
        return Err(format!(
            "backend at {base_url} does not support runtime Ghostty handoff placement switching yet. If this is your local server, restart `swimmers` or relaunch via `make tui`."
        ));
    }

    Err(read_error(response).await)
}

fn refreshed_openrouter_candidates_or_fallback(
    result: Result<Vec<String>, String>,
) -> Result<Vec<String>, String> {
    let models = result?;
    if models.is_empty() {
        return Ok(cached_or_default_openrouter_candidates());
    }
    Ok(models)
}

fn dir_list_query_params(
    path: Option<&str>,
    managed_only: bool,
    group: Option<&str>,
    target: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(path) = path {
        query.push(("path", path.to_string()));
    }
    if managed_only {
        query.push(("managed_only", true.to_string()));
    }
    if let Some(group) = group {
        query.push(("group", group.to_string()));
    }
    if let Some(target) = target
        .map(str::trim)
        .filter(|target| is_remote_launch_target(Some(*target)))
    {
        query.push(("target", target.to_string()));
    }
    query
}

pub(crate) fn root_error_message(err: &(dyn StdError + 'static)) -> String {
    let mut current = Some(err);
    let mut last = err.to_string();

    while let Some(next) = current.and_then(StdError::source) {
        let next_text = next.to_string();
        if !next_text.is_empty() {
            last = next_text;
        }
        current = Some(next);
    }

    last
}

fn friendly_transport_error_with_detail(
    base_url: &str,
    action: &str,
    err: &reqwest::Error,
    detail: &str,
) -> String {
    let summary = if err.is_timeout() {
        format!("timed out while trying to {action}")
    } else {
        format!("could not {action}")
    };
    let mut msg = format!(
        "swimmers API unavailable at {base_url}: {summary} ({detail}). Start `swimmers` or set SWIMMERS_TUI_URL."
    );
    if let Some(path) = client_log_path() {
        msg.push_str(&format!(" Tail logs: {}", path.display()));
    }
    msg
}

fn redacted_backend_url(raw_url: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw_url) else {
        return "[invalid backend URL]".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);

    let mut display = url.to_string();
    if display.ends_with('/') {
        display.pop();
    }
    display
}

fn redact_backend_url_text(raw_url: &str, text: &str) -> String {
    let mut redacted = text.replace(raw_url, &redacted_backend_url(raw_url));
    let Ok(url) = reqwest::Url::parse(raw_url) else {
        return redacted;
    };

    if !url.username().is_empty() {
        redacted = redacted.replace(url.username(), "[redacted]");
    }
    if let Some(password) = url.password().filter(|password| !password.is_empty()) {
        redacted = redacted.replace(password, "[redacted]");
    }
    if let Some(query) = url.query().filter(|query| !query.is_empty()) {
        redacted = redacted.replace(query, "[redacted]");
        for (_key, value) in url.query_pairs() {
            if !value.is_empty() {
                redacted = redacted.replace(value.as_ref(), "[redacted]");
            }
        }
    }
    if let Some(fragment) = url.fragment().filter(|fragment| !fragment.is_empty()) {
        redacted = redacted.replace(fragment, "[redacted]");
    }

    redacted
}

pub(crate) trait TuiApi: Send + Sync + 'static {
    fn fetch_session_snapshot(&self) -> BoxFuture<'_, Result<SessionListResponse, String>> {
        Box::pin(async move {
            let sessions = self.fetch_sessions().await?;
            let metadata = self.fetch_environment_metadata().await?;
            Ok(SessionListResponse {
                fleet_lens: swimmers::fleet_lens::build_fleet_lens_summary(&sessions),
                fleet_presets: metadata.fleet_presets,
                sessions,
                version: 0,
                repo_themes: Default::default(),
                environments: metadata.environments,
            })
        })
    }
    fn fetch_session_snapshot_for_initial_frame(
        &self,
    ) -> BoxFuture<'_, Result<SessionListResponse, String>> {
        self.fetch_session_snapshot()
    }
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>>;
    fn fetch_environments(&self) -> BoxFuture<'_, Result<Vec<EnvironmentSummary>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn fetch_fleet_presets(&self) -> BoxFuture<'_, Result<Vec<FleetLensPreset>, String>> {
        Box::pin(async {
            Ok(swimmers::fleet_lens::build_fleet_lens_presets(
                swimmers::session::overlay::default_overlay()
                    .map(|overlay| overlay.all_fleet_presets())
                    .unwrap_or_default(),
            ))
        })
    }
    fn fetch_environment_metadata(&self) -> BoxFuture<'_, Result<EnvironmentListResponse, String>> {
        Box::pin(async move {
            Ok(EnvironmentListResponse {
                environments: self.fetch_environments().await?,
                fleet_presets: self.fetch_fleet_presets().await?,
            })
        })
    }
    fn fetch_backend_health(&self) -> BoxFuture<'_, Result<BackendHealthResponse, String>>;
    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>>;
    fn update_thought_config(
        &self,
        config: ThoughtConfig,
        version: Option<u64>,
    ) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>>;
    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>>;
    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>>;
    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>>;
    fn fetch_session_skills(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<SessionSkillListResponse, String>>;
    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>>;
    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>>;
    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>>;
    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>>;
    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>>;
    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>>;
    fn open_attention_group(
        &self,
        max_sessions: usize,
        current_session_ids: Vec<String>,
        focus: bool,
        include_unnumbered_sessions: bool,
        layout: AttentionGroupLayout,
    ) -> BoxFuture<'_, Result<NativeAttentionGroupOpenResponse, String>>;
    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        group: Option<&str>,
        target: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>>;
    fn list_repo_dirs(&self) -> BoxFuture<'_, Result<DirRepoSearchResponse, String>>;
    fn update_dir_group_memberships(
        &self,
        path: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> BoxFuture<'_, Result<DirGroupMembershipUpdateResponse, String>>;
    fn start_repo_action(
        &self,
        path: &str,
        kind: RepoActionKind,
    ) -> BoxFuture<'_, Result<DirRepoActionResponse, String>>;
    fn fetch_overlay_plans(&self) -> BoxFuture<'_, Result<Vec<PlanPanelEntry>, String>>;
    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>>;
    fn adopt_session(
        &self,
        tmux_name: &str,
        session_id: Option<&str>,
    ) -> BoxFuture<'_, Result<AdoptSessionResponse, String>>;
    fn create_sessions_batch(
        &self,
        dirs: Vec<String>,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionsBatchResponse, String>>;
    fn send_group_input(
        &self,
        session_ids: Vec<String>,
        text: String,
    ) -> BoxFuture<'_, Result<SessionGroupInputResponse, String>>;
}

impl TuiApi for ApiClient {
    fn fetch_session_snapshot(&self) -> BoxFuture<'_, Result<SessionListResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/sessions", self.base_url);
            let started = Instant::now();
            let response = self
                .with_auth(self.http.get(url).timeout(API_SESSION_LIST_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("refresh sessions", err))?;

            let status = response.status();
            tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                status = %status,
                "fetch_session_snapshot response"
            );
            if status.is_success() {
                return response
                    .json::<SessionListResponse>()
                    .await
                    .map_err(|err| format!("failed to parse sessions response: {err}"));
            }

            let body = read_error(response).await;
            tracing::warn!(
                status = %status,
                body = %body,
                "fetch_session_snapshot non-success status"
            );
            Err(body)
        })
    }

    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        Box::pin(async move { Ok(self.fetch_session_snapshot().await?.sessions) })
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/thought-config", self.base_url);
            let response = self
                .with_auth(self.http.get(url))
                .send()
                .await
                .map_err(|err| self.transport_error("fetch thought config", err))?;

            if response.status().is_success() {
                return response
                    .json::<ThoughtConfigResponse>()
                    .await
                    .map_err(|err| format!("failed to parse thought config: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn fetch_environments(&self) -> BoxFuture<'_, Result<Vec<EnvironmentSummary>, String>> {
        Box::pin(async move { Ok(self.fetch_environment_metadata().await?.environments) })
    }

    fn fetch_fleet_presets(&self) -> BoxFuture<'_, Result<Vec<FleetLensPreset>, String>> {
        Box::pin(async move { Ok(self.fetch_environment_metadata().await?.fleet_presets) })
    }

    fn fetch_environment_metadata(&self) -> BoxFuture<'_, Result<EnvironmentListResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/environments", self.base_url);
            let response = self
                .with_auth(self.http.get(url).timeout(API_SESSION_LIST_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("refresh environments", err))?;

            if response.status().is_success() {
                return response
                    .json::<EnvironmentListResponse>()
                    .await
                    .map_err(|err| format!("failed to parse environments response: {err}"));
            }

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return self.fetch_environment_metadata_from_sessions().await;
            }

            Err(read_error(response).await)
        })
    }

    fn fetch_backend_health(&self) -> BoxFuture<'_, Result<BackendHealthResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/health", self.base_url);
            let response = self
                .http
                .get(url)
                .timeout(API_REQUEST_TIMEOUT)
                .send()
                .await
                .map_err(|err| self.transport_error("refresh backend health", err))?;

            if response.status().is_success() {
                return response
                    .json::<BackendHealthResponse>()
                    .await
                    .map_err(|err| format!("failed to parse backend health response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
        version: Option<u64>,
    ) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/thought-config", self.base_url);
            let mut request = self.with_auth(self.http.put(url)).json(&config);
            if let Some(version) = version {
                request = request.header(reqwest::header::IF_MATCH, version.to_string());
            }
            let response = request
                .send()
                .await
                .map_err(|err| self.transport_error("update thought config", err))?;

            if response.status().is_success() {
                return response
                    .json::<ThoughtConfigResponse>()
                    .await
                    .map_err(|err| format!("failed to parse updated thought config: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        Box::pin(async move {
            let response = match self.send_thought_config_test(&config).await {
                Ok(response) => response,
                Err(ThoughtConfigTestRemoteError::LocalFallback) => {
                    return self.local_test_thought_config(config).await;
                }
                Err(ThoughtConfigTestRemoteError::Message(message)) => return Err(message),
            };
            self.decode_thought_config_test_response(response, config)
                .await
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move {
            refreshed_openrouter_candidates_or_fallback(
                self.refresh_openrouter_candidates_inner().await,
            )
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            let session_id = encode_path_segment(&session_id);
            let url = format!(
                "{}/v1/sessions/{}/mermaid-artifact",
                self.base_url, session_id
            );
            let response = self
                .with_auth(self.http.get(url).timeout(API_MERMAID_ARTIFACT_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("fetch mermaid artifact", err))?;

            if response.status().is_success() {
                return response
                    .json::<MermaidArtifactResponse>()
                    .await
                    .map_err(|err| format!("failed to parse mermaid artifact: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn fetch_session_skills(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<SessionSkillListResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            let session_id = encode_path_segment(&session_id);
            let url = format!("{}/v1/sessions/{}/skills", self.base_url, session_id);
            let response = self
                .with_auth(
                    self.http
                        .get(url)
                        .query(&[("source", "sbp")])
                        .timeout(API_SESSION_SKILLS_TIMEOUT),
                )
                .send()
                .await
                .map_err(|err| self.transport_error("fetch session skills", err))?;

            decode_personal_workflows_response(
                response,
                "failed to parse session skills",
                self.session_skills_missing_message(),
            )
            .await
        })
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
            let session_id = encode_path_segment(&session_id);
            let url = format!("{}/v1/sessions/{}/plan-file", self.base_url, session_id);
            let response = self
                .with_auth(self.http.get(url))
                .query(&[("name", &name)])
                .send()
                .await
                .map_err(|err| self.transport_error("fetch plan file", err))?;

            if response.status().is_success() {
                return response
                    .json::<PlanFileResponse>()
                    .await
                    .map_err(|err| format!("failed to parse plan file: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/native/status", self.base_url);
            let response = self
                .with_auth(self.http.get(url))
                .send()
                .await
                .map_err(|err| self.transport_error("check terminal handoff status", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopStatusResponse>()
                    .await
                    .map_err(|err| format!("failed to parse terminal handoff status: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/native/app", self.base_url);
            let response = self
                .with_auth(self.http.put(url))
                .json(&NativeDesktopConfigRequest { app })
                .send()
                .await
                .map_err(|err| self.transport_error("switch the terminal handoff target", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopStatusResponse>()
                    .await
                    .map_err(|err| format!("failed to parse terminal handoff status: {err}"));
            }

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "backend at {} does not support runtime terminal handoff target switching yet. If this is your local server, restart `swimmers` or relaunch via `make tui`.",
                    self.display_base_url()
                ));
            }

            Err(read_error(response).await)
        })
    }

    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/native/mode", self.base_url);
            let response = self
                .with_auth(self.http.put(url))
                .json(&NativeDesktopModeRequest { mode })
                .send()
                .await
                .map_err(|err| self.transport_error("switch the Ghostty handoff placement", err))?;

            decode_native_mode_response(response, &self.base_url).await
        })
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        let session_id = session_id.map(|value| value.to_string());
        Box::pin(async move {
            let url = format!("{}/v1/selection", self.base_url);
            let response = self
                .with_auth(self.http.put(url))
                .json(&PublishSelectionRequest { session_id })
                .send()
                .await
                .map_err(|err| self.transport_error("publish the selected session", err))?;

            if response.status().is_success() {
                return Ok(());
            }

            Err(read_error(response).await)
        })
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            let url = format!("{}/v1/native/open", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_NATIVE_OPEN_TIMEOUT)
                .json(&NativeDesktopOpenRequest { session_id })
                .send()
                .await
                .map_err(|err| self.transport_error("open the selected session", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopOpenResponse>()
                    .await
                    .map_err(|err| format!("failed to parse terminal handoff response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn open_attention_group(
        &self,
        max_sessions: usize,
        current_session_ids: Vec<String>,
        focus: bool,
        include_unnumbered_sessions: bool,
        layout: AttentionGroupLayout,
    ) -> BoxFuture<'_, Result<NativeAttentionGroupOpenResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/native/attention-group/open", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_NATIVE_OPEN_TIMEOUT)
                .json(&NativeAttentionGroupOpenRequest {
                    max_sessions: Some(max_sessions),
                    current_session_ids,
                    include_unnumbered_sessions,
                    layout: Some(layout),
                    focus,
                })
                .send()
                .await
                .map_err(|err| self.transport_error("open the attention group", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeAttentionGroupOpenResponse>()
                    .await
                    .map_err(|err| format!("failed to parse attention group response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        group: Option<&str>,
        target: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let path = path.map(|value| value.to_string());
        let group = group.map(|value| value.to_string());
        let target = target.map(|value| value.to_string());
        Box::pin(async move {
            let url = format!("{}/v1/dirs", self.base_url);
            let query = dir_list_query_params(
                path.as_deref(),
                managed_only,
                group.as_deref(),
                target.as_deref(),
            );
            let request = self.http.get(url).query(&query);

            let response = self
                .with_auth(request.timeout(API_DIRECTORY_LIST_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("list directories", err))?;

            decode_personal_workflows_response(
                response,
                "failed to parse dirs response",
                format!(
                    "backend at {} does not expose /v1/dirs. Click-to-spawn directory browsing requires SWIMMERS_PERSONAL_WORKFLOWS=1 on the target backend; if this is your local server, relaunch via `make up` or `make tui`.",
                    self.display_base_url()
                ),
            )
            .await
        })
    }

    fn list_repo_dirs(&self) -> BoxFuture<'_, Result<DirRepoSearchResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/dirs/repositories", self.base_url);
            let response = self
                .with_auth(self.http.get(url).timeout(API_DIRECTORY_SEARCH_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("search repositories", err))?;

            decode_personal_workflows_response(
                response,
                "failed to parse repository search response",
                self.personal_workflows_route_missing_message(
                    "/v1/dirs/repositories",
                    "Repository search",
                ),
            )
            .await
        })
    }

    fn start_repo_action(
        &self,
        path: &str,
        kind: RepoActionKind,
    ) -> BoxFuture<'_, Result<DirRepoActionResponse, String>> {
        let path = path.to_string();
        Box::pin(async move {
            let url = format!("{}/v1/dirs/actions", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_DIRECTORY_ACTION_TIMEOUT)
                .json(&DirRepoActionRequest {
                    path,
                    target: None,
                    kind,
                })
                .send()
                .await
                .map_err(|err| self.transport_error("start the repo action", err))?;

            decode_personal_workflows_response(
                response,
                "failed to parse repo action response",
                self.personal_workflows_route_missing_message("/v1/dirs/actions", "Repo actions"),
            )
            .await
        })
    }

    fn update_dir_group_memberships(
        &self,
        path: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> BoxFuture<'_, Result<DirGroupMembershipUpdateResponse, String>> {
        let path = path.to_string();
        Box::pin(async move {
            let url = format!("{}/v1/dirs/group-memberships", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_DIRECTORY_ACTION_TIMEOUT)
                .json(&DirGroupMembershipUpdateRequest {
                    path,
                    target: None,
                    add,
                    remove,
                })
                .send()
                .await
                .map_err(|err| self.transport_error("update directory groups", err))?;

            decode_personal_workflows_response(
                response,
                "failed to parse directory group response",
                self.personal_workflows_route_missing_message(
                    "/v1/dirs/group-memberships",
                    "Directory group editing",
                ),
            )
            .await
        })
    }

    fn fetch_overlay_plans(&self) -> BoxFuture<'_, Result<Vec<PlanPanelEntry>, String>> {
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        let cwd = cwd.to_string();
        Box::pin(async move {
            let url = format!("{}/v1/sessions", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_CREATE_SESSION_TIMEOUT)
                .json(&CreateSessionRequest {
                    name: None,
                    cwd: Some(cwd),
                    spawn_tool: Some(spawn_tool),
                    launch_target,
                    initial_request,
                })
                .send()
                .await
                .map_err(|err| self.transport_error("create a session", err))?;

            if response.status().is_success() {
                return response
                    .json::<CreateSessionResponse>()
                    .await
                    .map_err(|err| format!("failed to parse create session response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn create_sessions_batch(
        &self,
        dirs: Vec<String>,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionsBatchResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/sessions/batch", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_CREATE_SESSION_TIMEOUT)
                .json(&CreateSessionsBatchRequest {
                    dirs,
                    spawn_tool: Some(spawn_tool),
                    launch_target,
                    initial_request,
                })
                .send()
                .await
                .map_err(|err| self.transport_error("create sessions", err))?;

            if response.status().is_success() {
                return response
                    .json::<CreateSessionsBatchResponse>()
                    .await
                    .map_err(|err| format!("failed to parse batch create response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn adopt_session(
        &self,
        tmux_name: &str,
        session_id: Option<&str>,
    ) -> BoxFuture<'_, Result<AdoptSessionResponse, String>> {
        let tmux_name = tmux_name.to_string();
        let session_id = session_id.map(str::to_string);
        Box::pin(async move {
            let url = format!("{}/v1/sessions/adopt", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_CREATE_SESSION_TIMEOUT)
                .json(&AdoptSessionRequest {
                    tmux_name,
                    session_id,
                })
                .send()
                .await
                .map_err(|err| self.transport_error("adopt a tmux session", err))?;

            if response.status().is_success() {
                return response
                    .json::<AdoptSessionResponse>()
                    .await
                    .map_err(|err| format!("failed to parse adopt session response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn send_group_input(
        &self,
        session_ids: Vec<String>,
        text: String,
    ) -> BoxFuture<'_, Result<SessionGroupInputResponse, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/sessions/group-input", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .timeout(API_REQUEST_TIMEOUT)
                .json(&SessionGroupInputRequest { session_ids, text })
                .send()
                .await
                .map_err(|err| self.transport_error("send group input", err))?;

            if response.status().is_success() {
                return response
                    .json::<SessionGroupInputResponse>()
                    .await
                    .map_err(|err| format!("failed to parse group input response: {err}"));
            }

            Err(read_error(response).await)
        })
    }
}

pub(crate) async fn read_error(response: reqwest::Response) -> String {
    let status = response.status();
    match response.json::<ErrorResponse>().await {
        Ok(body) => body.display_message(status),
        Err(_) => format!("request failed: {}", status),
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;

    fn test_api_client(base_url: &str) -> ApiClient {
        ApiClient {
            http: ApiClient::build_http_client(API_REQUEST_TIMEOUT).expect("test http client"),
            startup_http: ApiClient::build_http_client(API_STARTUP_REQUEST_TIMEOUT)
                .expect("test startup http client"),
            base_url: ApiClient::normalize_base_url(base_url),
            auth_token: None,
            startup_wait_timeout: API_STARTUP_WAIT_TIMEOUT,
            startup_retry_interval: API_STARTUP_RETRY_INTERVAL,
        }
    }

    async fn response_with(
        status: axum::http::StatusCode,
        content_type: &'static str,
        body: String,
    ) -> reqwest::Response {
        use axum::response::{IntoResponse, Response};
        use axum::routing::get;
        use axum::Router;

        let app = Router::new().route(
            "/",
            get(move || async move {
                let mut response: Response = body.into_response();
                *response.status_mut() = status;
                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    content_type.parse().unwrap(),
                );
                response
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind response helper server");
        let addr = listener.local_addr().expect("response helper addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve response helper");
        });
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("fetch helper response");

        handle.abort();
        response
    }

    #[tokio::test]
    async fn personal_workflows_response_decodes_success_json() {
        let response = response_with(
            axum::http::StatusCode::OK,
            "application/json",
            r#"{"repositories":[]}"#.to_string(),
        )
        .await;

        let decoded = decode_personal_workflows_response::<serde_json::Value>(
            response,
            "failed to parse repository search response",
            "missing route".to_string(),
        )
        .await
        .expect("success response should decode");

        assert_eq!(decoded["repositories"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn personal_workflows_response_reports_parse_error_shape() {
        let response = response_with(
            axum::http::StatusCode::OK,
            "application/json",
            "not json".to_string(),
        )
        .await;

        let error = decode_personal_workflows_response::<serde_json::Value>(
            response,
            "failed to parse repository search response",
            "missing route".to_string(),
        )
        .await
        .expect_err("invalid success JSON should report parse error");

        assert!(error.starts_with("failed to parse repository search response:"));
    }

    #[tokio::test]
    async fn personal_workflows_response_uses_feature_hint_for_404() {
        let response = response_with(
            axum::http::StatusCode::NOT_FOUND,
            "application/json",
            String::new(),
        )
        .await;

        let error = decode_personal_workflows_response::<serde_json::Value>(
            response,
            "failed to parse repository search response",
            "backend at http://127.0.0.1:3210 does not expose /v1/dirs/repositories. Repository search requires SWIMMERS_PERSONAL_WORKFLOWS=1 on the target backend; if this is your local server, relaunch via `make up` or `make tui`.".to_string(),
        )
        .await
        .expect_err("missing route should return feature hint");

        assert!(error.contains("does not expose /v1/dirs/repositories"));
        assert!(error.contains("SWIMMERS_PERSONAL_WORKFLOWS=1"));
        assert!(error.contains("make tui"));
    }

    #[tokio::test]
    async fn personal_workflows_response_preserves_generic_api_error_body() {
        let body = serde_json::to_string(&ErrorResponse::with_message(
            "VALIDATION_FAILED",
            "bad repo action",
        ))
        .expect("serialize error body");
        let response = response_with(
            axum::http::StatusCode::BAD_REQUEST,
            "application/json",
            body,
        )
        .await;

        let error = decode_personal_workflows_response::<serde_json::Value>(
            response,
            "failed to parse repo action response",
            "missing route".to_string(),
        )
        .await
        .expect_err("generic error should preserve API body");

        assert_eq!(error, "VALIDATION_FAILED: bad repo action");
    }

    #[tokio::test]
    async fn native_mode_response_decodes_success_json() {
        let response = response_with(
            axum::http::StatusCode::OK,
            "application/json",
            serde_json::to_string(&NativeDesktopStatusResponse {
                supported: true,
                platform: Some("macos".to_string()),
                app_id: Some(NativeDesktopApp::Ghostty),
                ghostty_mode: Some(GhosttyOpenMode::Add),
                app: Some("Ghostty".to_string()),
                reason: None,
            })
            .expect("serialize native status"),
        )
        .await;

        let decoded = decode_native_mode_response(response, "http://127.0.0.1:3210")
            .await
            .expect("success response should decode");

        assert!(decoded.supported);
        assert_eq!(decoded.app_id, Some(NativeDesktopApp::Ghostty));
        assert_eq!(decoded.ghostty_mode, Some(GhosttyOpenMode::Add));
        assert_eq!(decoded.app.as_deref(), Some("Ghostty"));
    }

    #[tokio::test]
    async fn native_mode_response_reports_parse_error_shape() {
        let response = response_with(
            axum::http::StatusCode::OK,
            "application/json",
            "not json".to_string(),
        )
        .await;

        let error = decode_native_mode_response(response, "http://127.0.0.1:3210")
            .await
            .expect_err("invalid success JSON should report parse error");

        assert!(error.starts_with("failed to parse terminal handoff status:"));
    }

    #[tokio::test]
    async fn native_mode_response_reports_restart_hint_on_404() {
        let response = response_with(
            axum::http::StatusCode::NOT_FOUND,
            "application/json",
            String::new(),
        )
        .await;

        let error = decode_native_mode_response(response, "http://127.0.0.1:3210")
            .await
            .expect_err("missing native mode route should return restart hint");

        assert!(error.contains(
            "backend at http://127.0.0.1:3210 does not support runtime Ghostty handoff placement switching yet"
        ));
        assert!(error.contains("restart `swimmers`"));
        assert!(error.contains("make tui"));
    }

    #[tokio::test]
    async fn native_mode_response_preserves_generic_api_error_body() {
        let body = serde_json::to_string(&ErrorResponse::with_message(
            "VALIDATION_FAILED",
            "bad native mode",
        ))
        .expect("serialize error body");
        let response = response_with(
            axum::http::StatusCode::BAD_REQUEST,
            "application/json",
            body,
        )
        .await;

        let error = decode_native_mode_response(response, "http://127.0.0.1:3210")
            .await
            .expect_err("generic error should preserve API body");

        assert_eq!(error, "VALIDATION_FAILED: bad native mode");
    }

    #[test]
    fn openrouter_candidates_keep_non_empty_refresh_result() {
        let models =
            refreshed_openrouter_candidates_or_fallback(Ok(vec!["openrouter/test".to_string()]))
                .expect("non-empty models");

        assert_eq!(models, vec!["openrouter/test"]);
    }

    #[test]
    fn openrouter_candidates_fall_back_when_refresh_is_empty() {
        let models =
            refreshed_openrouter_candidates_or_fallback(Ok(Vec::new())).expect("fallback models");

        assert!(!models.is_empty());
        assert_eq!(models, cached_or_default_openrouter_candidates());
    }

    #[test]
    fn openrouter_candidates_preserve_refresh_errors() {
        let error = refreshed_openrouter_candidates_or_fallback(Err("network down".to_string()))
            .expect_err("refresh errors should pass through");

        assert_eq!(error, "network down");
    }

    #[test]
    fn dir_list_query_params_preserve_path_managed_and_group_semantics() {
        let query = dir_list_query_params(Some("/srv/repos"), true, Some("core"), Some("skillbox"));

        assert_eq!(
            query,
            vec![
                ("path", "/srv/repos".to_string()),
                ("managed_only", "true".to_string()),
                ("group", "core".to_string()),
                ("target", "skillbox".to_string())
            ]
        );
    }

    #[test]
    fn dir_list_query_params_omit_absent_and_false_filters() {
        assert!(dir_list_query_params(None, false, None, None).is_empty());
        assert_eq!(
            dir_list_query_params(Some("/srv/repos"), false, None, Some("local")),
            vec![("path", "/srv/repos".to_string())]
        );
        assert_eq!(
            dir_list_query_params(Some("/srv/repos"), false, None, Some(" LOCAL ")),
            vec![("path", "/srv/repos".to_string())]
        );
        assert_eq!(
            dir_list_query_params(None, false, Some("personal"), Some("skillbox")),
            vec![
                ("group", "personal".to_string()),
                ("target", "skillbox".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn thought_config_test_response_decodes_success_json() {
        let response = response_with(
            axum::http::StatusCode::OK,
            "application/json",
            serde_json::to_string(&ThoughtConfigTestResponse {
                ok: true,
                message: "probe succeeded".to_string(),
                last_backend_error: None,
                llm_calls: 1,
            })
            .expect("serialize thought config probe result"),
        )
        .await;
        let client = test_api_client("http://100.64.0.1:3210");

        let decoded = client
            .decode_thought_config_test_response(response, ThoughtConfig::default())
            .await
            .expect("success response should decode");

        assert!(decoded.ok);
        assert_eq!(decoded.message, "probe succeeded");
        assert_eq!(decoded.llm_calls, 1);
    }

    #[tokio::test]
    async fn thought_config_test_response_reports_parse_error_shape() {
        let response = response_with(
            axum::http::StatusCode::OK,
            "application/json",
            "not json".to_string(),
        )
        .await;
        let client = test_api_client("http://100.64.0.1:3210");

        let error = client
            .decode_thought_config_test_response(response, ThoughtConfig::default())
            .await
            .expect_err("invalid success JSON should report parse error");

        assert!(error.starts_with("failed to parse thought config test:"));
    }

    #[tokio::test]
    async fn thought_config_test_response_preserves_generic_api_error_body() {
        let body = serde_json::to_string(&ErrorResponse::with_message(
            "VALIDATION_FAILED",
            "bad thought config",
        ))
        .expect("serialize error body");
        let response = response_with(
            axum::http::StatusCode::BAD_REQUEST,
            "application/json",
            body,
        )
        .await;
        let client = test_api_client("http://100.64.0.1:3210");

        let error = client
            .decode_thought_config_test_response(response, ThoughtConfig::default())
            .await
            .expect_err("generic error should preserve API body");

        assert_eq!(error, "VALIDATION_FAILED: bad thought config");
    }
}
