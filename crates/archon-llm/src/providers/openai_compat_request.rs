use super::*;

impl OpenAiCompatProvider {
    /// Construct a new provider bound to `descriptor`, reusing the supplied
    /// HTTP client and API key.
    pub fn new(
        descriptor: &'static ProviderDescriptor,
        http: Arc<reqwest::Client>,
        api_key: ApiKey,
    ) -> Self {
        Self {
            descriptor,
            http,
            api_key,
        }
    }

    // -----------------------------------------------------------------
    // URL construction
    // -----------------------------------------------------------------

    /// Join `descriptor.base_url` with `chat/completions`.
    ///
    /// `Url::join` replaces the final path segment unless the base ends in
    /// `/`, so we explicitly append one before joining. That way a base of
    /// `http://localhost:11434/v1` correctly produces
    /// `http://localhost:11434/v1/chat/completions` instead of
    /// `http://localhost:11434/chat/completions`.
    pub(super) fn build_chat_url(&self) -> String {
        self.build_endpoint_url("chat/completions")
    }

    pub(super) fn build_embeddings_url(&self) -> String {
        self.build_endpoint_url("embeddings")
    }

    pub(super) fn build_endpoint_url(&self, endpoint: &str) -> String {
        let mut base = self.descriptor.base_url.clone();
        // Ensure trailing slash on the base path so `Url::join` appends
        // rather than replaces the final segment.
        if !base.path().ends_with('/') {
            let new_path = format!("{}/", base.path());
            base.set_path(&new_path);
        }
        match base.join(endpoint) {
            Ok(u) => u.to_string(),
            // Fallback: manually concatenate. `endpoint` here is a static
            // string so this branch is essentially unreachable, but we
            // prefer a deterministic string over a panic.
            Err(_) => format!("{}{}", base, endpoint),
        }
    }

    /// TASK-AGS-705: delimiter bytes for the provider's streaming wire
    /// format. Crate-visible so TASK-AGS-707 can drive its chunk parser
    /// without knowing which provider it's talking to. Marked
    /// `#[allow(dead_code)]` until TASK-AGS-707 consumes it.
    #[allow(dead_code)]
    pub(crate) fn delimiter_bytes(&self) -> &'static [u8] {
        self.descriptor.quirks.delimiter_bytes()
    }

    // -----------------------------------------------------------------
    // Auth + static headers
    // -----------------------------------------------------------------

    /// Attach auth + descriptor headers to a `RequestBuilder`. No
    /// per-provider-id branching: the descriptor's `auth_flavor` decides
    /// how to sign the request, and custom schemes are deferred to
    /// TASK-AGS-705.
    pub(super) fn apply_auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = match &self.descriptor.auth_flavor {
            AuthFlavor::BearerApiKey => rb.bearer_auth(self.api_key.expose()),
            AuthFlavor::None => rb,
            // BasicAuth + Custom schemes are reserved for TASK-AGS-705.
            // Return `rb` unchanged for now so those descriptors compile
            // without stubbing behavior they don't need in this task.
            AuthFlavor::BasicAuth | AuthFlavor::Custom(_) => rb,
        };
        let mut rb = rb;
        for (k, v) in &self.descriptor.headers {
            rb = rb.header(k, v);
        }
        rb
    }

    // -----------------------------------------------------------------
    // Response parsing
    // -----------------------------------------------------------------

    /// Apply `quirks.ignore_response_fields` to a response body in place.
    /// Strips the named keys from the top-level object and from each
    /// element of `choices` if present. Used for DeepSeek's `logprobs`
    /// bag whose shape deviates from OpenAI's canonical form and whose
    /// contents archon never consumes. Pure data transform — no
    /// provider-id branching.
    pub(super) fn strip_ignored_fields(body: &mut Value, quirks: &ProviderQuirks) {
        if quirks.ignore_response_fields.is_empty() {
            return;
        }
        if let Some(obj) = body.as_object_mut() {
            for field in quirks.ignore_response_fields {
                obj.remove(*field);
            }
            if let Some(Value::Array(choices)) = obj.get_mut("choices") {
                for choice in choices.iter_mut() {
                    if let Some(choice_obj) = choice.as_object_mut() {
                        for field in quirks.ignore_response_fields {
                            choice_obj.remove(*field);
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // HTTP error mapping
    // -----------------------------------------------------------------

    /// Map a non-success HTTP status into an `LlmError`.
    ///
    pub(super) fn map_http_error(
        status: reqwest::StatusCode,
        headers: &reqwest::header::HeaderMap,
        body: String,
        provider: &str,
    ) -> LlmError {
        let error = crate::provider_fault::from_status(status.as_u16(), headers, body.clone());
        match error {
            LlmError::Auth(_) => LlmError::Auth(format!("{provider}: {body}").into()),
            error => error,
        }
    }

    #[cfg(test)]
    pub(super) fn parse_retry_after(value: &str) -> Option<u64> {
        if let Ok(seconds) = value.trim().parse::<u64>() {
            return Some(seconds);
        }
        let date = chrono::DateTime::parse_from_rfc2822(value.trim()).ok()?;
        let remaining = date
            .timestamp()
            .saturating_sub(chrono::Utc::now().timestamp());
        Some(remaining.max(0) as u64)
    }

    // -----------------------------------------------------------------
    // Embeddings — inherent method, not on the trait
    // -----------------------------------------------------------------

    /// Embeddings endpoint — inherent method, not on the `LlmProvider`
    /// trait.
    ///
    /// TASK-AGS-703 spec deviation: the trait does not expose `embed()`,
    /// so this lives as an inherent method for OpenAI-compat callers that
    /// need it. TASK-AGS-711 acceptance tests may exercise this via a
    /// downcast helper. Minimal implementation using the same auth / URL
    /// / error patterns as `complete()`.
    pub async fn embed(&self, model: &str, input: Vec<String>) -> Result<Vec<Vec<f32>>, LlmError> {
        let url = self.build_embeddings_url();
        let body = json!({
            "model": if model.is_empty() { self.descriptor.default_model.as_str() } else { model },
            "input": input,
        });
        let rb = self.http.post(&url).json(&body);
        let rb = self.apply_auth(rb);
        let resp = rb
            .send()
            .await
            .map_err(crate::provider_fault::from_transport)?;
        let status = resp.status();
        if !status.is_success() {
            let headers = resp.headers().clone();
            let text = resp.text().await.map_err(|error| {
                crate::provider_fault::body_read_error(status.as_u16(), &headers, error)
            })?;
            return Err(Self::map_http_error(status, &headers, text, self.name()));
        }
        let json: Value = resp
            .json()
            .await
            .map_err(crate::provider_fault::from_transport)?;

        let data = json.get("data").and_then(|d| d.as_array()).ok_or_else(|| {
            LlmError::Serialize("missing `data` array in embeddings response".into())
        })?;

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(data.len());
        for entry in data {
            let emb = entry
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| LlmError::Serialize("missing `data[].embedding` array".into()))?;
            let mut vec: Vec<f32> = Vec::with_capacity(emb.len());
            for v in emb {
                let f = v
                    .as_f64()
                    .ok_or_else(|| LlmError::Serialize("embedding entry was not a number".into()))?
                    as f32;
                vec.push(f);
            }
            out.push(vec);
        }
        Ok(out)
    }
}
