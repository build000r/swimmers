use super::*;

pub(crate) struct ApiClient {
    pub(crate) http: Client,
    pub(crate) base_url: String,
    pub(crate) auth_token: Option<String>,
}

impl ApiClient {
    pub(crate) fn from_env() -> Result<Self, String> {
        let config = Config::from_env();
        let base_url = std::env::var("THRONGTERM_TUI_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", config.port));
        let auth_token = match config.auth_mode {
            AuthMode::Token => config.auth_token,
            AuthMode::LocalTrust => None,
        };

        let http = Client::builder()
            .connect_timeout(API_CONNECT_TIMEOUT)
            .timeout(API_REQUEST_TIMEOUT)
            .build()
            .map_err(|err| format!("failed to build http client: {err}"))?;

        Ok(Self {
            http,
            base_url,
            auth_token,
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

    async fn ensure_startup_access(
        &self,
        response: reqwest::Response,
        path: &str,
    ) -> Result<(), String> {
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(self.startup_access_error(path, status))
            }
            _ => Err(read_error(response).await),
        }
    }

    async fn preflight_session_refresh_access(&self) -> Result<(), String> {
        let url = format!("{}/v1/sessions", self.base_url);
        let response = self
            .with_auth(self.http.get(url))
            .send()
            .await
            .map_err(|err| self.transport_error("refresh sessions", err))?;

        self.ensure_startup_access(response, "/v1/sessions").await
    }

    async fn preflight_selection_sync_access(&self) -> Result<(), String> {
        let url = format!("{}/v1/selection", self.base_url);
        let response = self
            .with_auth(self.http.put(url))
            .json(&PublishSelectionRequest { session_id: None })
            .send()
            .await
            .map_err(|err| self.transport_error("clear the published selection", err))?;

        self.ensure_startup_access(response, "/v1/selection").await
    }

    pub(crate) async fn preflight_startup_access(&self) -> Result<(), String> {
        self.preflight_session_refresh_access().await?;
        self.preflight_selection_sync_access().await?;
        Ok(())
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

pub(crate) fn friendly_transport_error(base_url: &str, action: &str, err: &reqwest::Error) -> String {
    let detail = root_error_message(err);
    let summary = if err.is_timeout() {
        format!("timed out while trying to {action}")
    } else {
        format!("could not {action}")
    };

    format!(
        "backend unavailable at {base_url}: {summary} ({detail}). Start `throngterm` or set THRONGTERM_TUI_URL."
    )
}

pub(crate) trait TuiApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>>;
    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>>;
    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>>;
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
                .with_auth(self.http.get(url))
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
                .with_auth(self.http.get(url))
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
                .with_auth(request)
                .send()
                .await
                .map_err(|err| self.transport_error("list directories", err))?;

            if response.status().is_success() {
                return response
                    .json::<DirListResponse>()
                    .await
                    .map_err(|err| format!("failed to parse dirs response: {err}"));
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
