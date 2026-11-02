use super::*;

impl CodexProvider {
    pub fn new(
        credentials_path: PathBuf,
        spoof: SpoofConfig,
        http: reqwest::Client,
    ) -> Result<Self, LlmError> {
        Self::new_inner(
            credentials_path,
            spoof,
            http,
            DEFAULT_BASE_URL.into(),
            DEFAULT_PREFLIGHT_URL.into(),
            Duration::from_millis(BASE_DELAY_MS),
        )
    }

    pub fn new_with_base_url(
        credentials_path: PathBuf,
        spoof: SpoofConfig,
        http: reqwest::Client,
        base_url: String,
    ) -> Result<Self, LlmError> {
        Self::new_inner(
            credentials_path,
            spoof,
            http,
            base_url.clone(),
            format!("{base_url}/"),
            Duration::from_millis(10),
        )
    }

    fn new_inner(
        credentials_path: PathBuf,
        spoof: SpoofConfig,
        http: reqwest::Client,
        base_url: String,
        preflight_url: String,
        retry_base_delay: Duration,
    ) -> Result<Self, LlmError> {
        validate_spoof_headers(&spoof)?;
        let oauth_client = Arc::new(CodexOAuthClient::new(http.clone()));
        Ok(Self {
            http,
            credentials_path,
            spoof,
            base_url,
            oauth_client,
            tls_preflight_cache: OnceCell::new(),
            refresh_lock: Mutex::new(()),
            preflight_url,
            retry_base_delay,
            aliases: CodexAliasMap::default(),
        })
    }

    /// Builder: attach an alias map sourced from operator config.
    pub fn with_alias_map(mut self, aliases: CodexAliasMap) -> Self {
        self.aliases = aliases;
        self
    }

    pub fn resolve_url(&self) -> String {
        format!("{}/codex/responses", self.base_url.trim_end_matches('/'))
    }

    pub fn build_request_body(&self, req: &LlmRequest) -> Result<ResponsesRequest, LlmError> {
        let input = messages_to_responses_input(req)?;
        let instructions = join_system_prompt(&req.system);
        let tools = if req.tools.is_empty() {
            None
        } else {
            Some(tools_to_responses_tools(&req.tools)?)
        };
        let reasoning = build_reasoning_config(&req.model, req.effort.as_deref());
        let cache_key = prompt_cache_key(&req.model, instructions.as_deref(), tools.as_deref());

        Ok(ResponsesRequest {
            model: req.model.clone(),
            store: false,
            stream: true,
            instructions,
            input,
            tools,
            tool_choice: Some("auto".into()),
            parallel_tool_calls: Some(true),
            temperature: None,
            reasoning,
            service_tier: None,
            text: Some(TextConfig {
                verbosity: Some("low".into()),
            }),
            include: Some(vec!["reasoning.encrypted_content".into()]),
            prompt_cache_key: Some(cache_key),
        })
    }

    pub(super) async fn ensure_preflight(&self) -> Result<(), LlmError> {
        self.tls_preflight_cache
            .get_or_try_init(|| run_codex_tls_preflight_url(&self.http, &self.preflight_url))
            .await
            .map(|_| ())
    }

    pub(super) async fn send_with_retry(
        &self,
        body: &ResponsesRequest,
    ) -> Result<reqwest::Response, LlmError> {
        let mut refreshed_after_401 = false;

        for attempt in 0..=MAX_RETRIES {
            crate::provider_fault::begin_request();
            let creds = ensure_codex_token_valid(&self.credentials_path, &self.oauth_client)
                .await
                .map_err(auth_to_llm_error)?;
            let session_id = Uuid::new_v4().to_string();
            let headers = build_codex_headers(&creds, &self.spoof, &session_id)?;
            // Only ever serialised when the level is enabled — the body is
            // large and this is the hot path. Without it there is no way to see
            // what actually went out, which is what made a silent prompt-cache
            // miss on this provider impossible to diagnose from the outside:
            // the counters read zero whether the cache missed or the service
            // simply did not report a hit.
            if tracing::enabled!(tracing::Level::DEBUG)
                && let Ok(rendered) = serde_json::to_string(body)
            {
                tracing::debug!(
                    provider = "openai-codex",
                    "Codex request body: {}",
                    crate::debug_body::debug_body(&rendered)
                );
            }
            let response = self
                .http
                .post(self.resolve_url())
                .headers(headers)
                .json(body)
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(e) if attempt < MAX_RETRIES && !crate::provider_fault::retry_is_deferred() => {
                    self.sleep_before_retry(attempt).await;
                    if e.is_request() {
                        continue;
                    }
                    continue;
                }
                Err(e) => return Err(crate::provider_fault::from_transport(e)),
            };

            let retry_after = crate::transport_http::response_retry_after(&response);
            let status = response.status().as_u16();
            crate::provider_fault::note_response(status, retry_after);
            if status < 400 {
                return Ok(response);
            }

            let headers = response.headers().clone();
            let body_text = response
                .text()
                .await
                .map_err(|error| crate::provider_fault::body_read_error(status, &headers, error))?;
            if status == 401 && !refreshed_after_401 {
                refreshed_after_401 = true;
                crate::provider_fault::begin_request();
                self.refresh_token_once().await?;
                continue;
            }
            if status == 401 {
                return Err(map_codex_error(status, body_text, true));
            }
            if let Some(error) = crate::provider_fault::structured_rejection(status, &body_text) {
                return Err(error);
            }
            if is_quota_exceeded(&body_text) {
                return Err(LlmError::QuotaExceeded(body_text.into()));
            }
            if is_retryable(status, &body_text)
                && attempt < MAX_RETRIES
                && !crate::provider_fault::retry_is_deferred()
            {
                self.sleep_before_retry(attempt).await;
                continue;
            }
            return Err(crate::transport_http::with_retry_after(
                map_codex_error(status, body_text, false),
                retry_after,
            ));
        }

        Err(LlmError::Aborted)
    }

    async fn refresh_token_once(&self) -> Result<(), LlmError> {
        let _guard = self.refresh_lock.lock().await;
        let (creds, _) =
            read_codex_credentials_locked(&self.credentials_path).map_err(auth_to_llm_error)?;
        let refreshed = self
            .oauth_client
            .refresh(creds.refresh_token.expose())
            .await
            .map_err(auth_to_llm_error)?;
        write_codex_credentials_atomic(&self.credentials_path, &refreshed)
            .map_err(auth_to_llm_error)
    }

    async fn sleep_before_retry(&self, attempt: u32) {
        let delay = self.retry_base_delay * 2u32.saturating_pow(attempt);
        tokio::time::sleep(delay).await;
    }
}
