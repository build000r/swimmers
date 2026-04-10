use super::*;
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

impl StartupAccessError {
    fn into_string(self) -> String {
        match self {
            Self::Retryable(message) | Self::Fatal(message) => message,
        }
    }
}

impl ApiClient {
    fn build_http_client(timeout: Duration) -> Result<Client, String> {
        Client::builder()
            .connect_timeout(API_CONNECT_TIMEOUT)
            .timeout(timeout)
            .build()
            .map_err(|err| format!("failed to build http client: {err}"))
    }

    pub(crate) fn from_env() -> Result<Self, String> {
        let config = Config::from_env();
        let base_url = std::env::var("SWIMMERS_TUI_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", config.port));
        let auth_token = match config.auth_mode {
            AuthMode::Token => config.auth_token,
            AuthMode::LocalTrust => None,
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

    pub(crate) fn transport_error(&self, action: &str, err: reqwest::Error) -> String {
        friendly_transport_error(&self.base_url, action, &err)
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
        match status {
            reqwest::StatusCode::UNAUTHORIZED => format!(
                "backend at {} requires valid auth for {}. Set AUTH_MODE=token and AUTH_TOKEN to match the target API.",
                self.base_url, path
            ),
            reqwest::StatusCode::FORBIDDEN => format!(
                "backend at {} denied startup access to {}. Use a token with the required session scope for this TUI instance.",
                self.base_url, path
            ),
            _ => format!(
                "backend at {} rejected startup access to {} ({status})",
                self.base_url, path
            ),
        }
    }

    fn startup_transport_error(&self, action: &str, err: reqwest::Error) -> StartupAccessError {
        let retryable = err.is_connect() || err.is_timeout();
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
        loop {
            match probe().await {
                Ok(()) => return Ok(()),
                Err(StartupAccessError::Fatal(message)) => return Err(message),
                Err(StartupAccessError::Retryable(message)) => {
                    if Instant::now() >= deadline {
                        return Err(message);
                    }
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
        if self.targets_local_backend() {
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

pub(crate) fn friendly_transport_error(
    base_url: &str,
    action: &str,
    err: &reqwest::Error,
) -> String {
    let detail = root_error_message(err);
    let summary = if err.is_timeout() {
        format!("timed out while trying to {action}")
    } else {
        format!("could not {action}")
    };

    format!(
        "swimmers API unavailable at {base_url}: {summary} ({detail}). Start `swimmers` or set SWIMMERS_TUI_URL."
    )
}

pub(crate) trait TuiApi: Send + Sync + 'static {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>>;
    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>>;
    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>>;
    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>>;
    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>>;
    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>>;
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
    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
    ) -> BoxFuture<'_, Result<DirListResponse, String>>;
    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>>;
}

impl TuiApi for ApiClient {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/sessions", self.base_url);
            let response = self
                .with_auth(self.http.get(url).timeout(API_SESSION_LIST_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("refresh sessions", err))?;

            if response.status().is_success() {
                let payload = response
                    .json::<SessionListResponse>()
                    .await
                    .map_err(|err| format!("failed to parse sessions response: {err}"))?;
                return Ok(payload.sessions);
            }

            Err(read_error(response).await)
        })
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

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        Box::pin(async move {
            let url = format!("{}/v1/thought-config", self.base_url);
            let response = self
                .with_auth(self.http.put(url))
                .json(&config)
                .send()
                .await
                .map_err(|err| self.transport_error("update thought config", err))?;

            if response.status().is_success() {
                return response
                    .json::<ThoughtConfig>()
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
            let url = format!("{}/v1/thought-config/test", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .json(&config)
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(err) if self.targets_local_backend() => {
                    return self.local_test_thought_config(config).await;
                }
                Err(err) => return Err(self.transport_error("test thought config", err)),
            };

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
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move {
            match self.refresh_openrouter_candidates_inner().await {
                Ok(models) if !models.is_empty() => Ok(models),
                Ok(_) => Ok(cached_or_default_openrouter_candidates()),
                Err(err) => Err(err),
            }
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
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

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
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
                .map_err(|err| self.transport_error("check native desktop status", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopStatusResponse>()
                    .await
                    .map_err(|err| format!("failed to parse native status: {err}"));
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
                .map_err(|err| self.transport_error("switch the native desktop target", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopStatusResponse>()
                    .await
                    .map_err(|err| format!("failed to parse native status: {err}"));
            }

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "backend at {} does not support runtime native target switching yet. If this is your local server, restart `swimmers` or relaunch via `make tui`.",
                    self.base_url
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
                .map_err(|err| self.transport_error("switch the Ghostty preview mode", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopStatusResponse>()
                    .await
                    .map_err(|err| format!("failed to parse native status: {err}"));
            }

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "backend at {} does not support runtime Ghostty preview mode switching yet. If this is your local server, restart `swimmers` or relaunch via `make tui`.",
                    self.base_url
                ));
            }

            Err(read_error(response).await)
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
                    .map_err(|err| format!("failed to parse native open response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let path = path.map(|value| value.to_string());
        Box::pin(async move {
            let url = format!("{}/v1/dirs", self.base_url);
            let mut request = self.http.get(url);
            if let Some(path) = path {
                request = request.query(&[("path", path)]);
            }
            if managed_only {
                request = request.query(&[("managed_only", true)]);
            }

            let response = self
                .with_auth(request.timeout(API_DIRECTORY_LIST_TIMEOUT))
                .send()
                .await
                .map_err(|err| self.transport_error("list directories", err))?;

            if response.status().is_success() {
                return response
                    .json::<DirListResponse>()
                    .await
                    .map_err(|err| format!("failed to parse dirs response: {err}"));
            }

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "backend at {} does not expose /v1/dirs. Click-to-spawn directory browsing requires a `swimmers` build with `--features personal-workflows`; if this is your local server, relaunch via `make tui`.",
                    self.base_url
                ));
            }

            Err(read_error(response).await)
        })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
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
}

pub(crate) async fn read_error(response: reqwest::Response) -> String {
    let status = response.status();
    match response.json::<ErrorResponse>().await {
        Ok(body) => body
            .message
            .unwrap_or_else(|| format!("request failed: {}", status)),
        Err(_) => format!("request failed: {}", status),
    }
}
