use super::*;

impl BedrockProvider {
    /// Create a new Bedrock provider.
    pub fn new(region: String, model_id: String) -> Self {
        Self {
            region,
            model_id,
            http: crate::transport_http::client_builder()
                .build()
                .expect("default HTTP client configuration is valid"),
        }
    }

    /// Build the Bedrock Converse request body.
    pub fn build_converse_body(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        max_tokens: u32,
    ) -> serde_json::Value {
        Self::build_converse_body_cached(system, messages, tools, max_tokens, None)
    }

    /// Build the Converse body, optionally placing prompt-cache checkpoints.
    ///
    /// Converse takes a checkpoint as its own array element rather than as an
    /// attribute on a neighbouring block, so a checkpoint is appended after the
    /// content it should cache:
    ///
    /// ```json
    /// { "cachePoint": { "type": "default", "ttl": "1h" } }
    /// ```
    ///
    /// Placement follows Bedrock's own evaluation order, `tools` -> `system` ->
    /// `messages`. The sections are chained: changing `tools` invalidates the
    /// `system` and `messages` caches, so the checkpoints go after the most
    /// stable content first. The messages checkpoint deliberately lands on the
    /// last message, which advances each turn and keeps the growing prefix
    /// cached behind it.
    pub fn build_converse_body_cached(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        max_tokens: u32,
        cache: Option<crate::cache_strategy::CachePointPlacement>,
    ) -> serde_json::Value {
        // System prompt: array of {text: "..."} objects.
        let mut system_arr: Vec<serde_json::Value> = system
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|text| serde_json::json!({"text": text}))
            })
            .collect();

        // Messages: convert Archon format to Bedrock Converse format.
        //
        // `filter_map`, because a message with no Converse-representable content
        // must be dropped rather than sent: Converse rejects an empty content
        // field and fails the entire request, so one such message makes every
        // subsequent turn in the session fail too.
        let mut bedrock_messages: Vec<serde_json::Value> = messages
            .iter()
            .filter_map(convert_message_to_bedrock)
            .collect();

        if let Some(cache) = cache {
            if cache.system && !system_arr.is_empty() {
                // After the stable head, not after everything.
                //
                // Archon appends per-turn content to the end of the system
                // prompt — recalled memories, the inner voice, reminders — and
                // that changes most turns. A checkpoint behind it is rewritten
                // every turn and almost never read, and a cache write bills at
                // 1.25x plain input (2x at the one-hour TTL), so it costs more
                // than not caching at all.
                //
                // Clamped, because `system_arr` is built by `filter_map` over
                // the caller's blocks: anything without a `text` field is
                // dropped, so the index can be shorter than the caller's count.
                // Out of range falls back to the end, which is the old
                // behaviour and always valid.
                let at = cache
                    .system_stable_blocks
                    .filter(|n| *n > 0 && *n < system_arr.len())
                    .unwrap_or(system_arr.len());
                system_arr.insert(at, cache.point());
            }
            if cache.messages
                && let Some(last) = bedrock_messages.last_mut()
                && let Some(content) = last
                    .get_mut("content")
                    .and_then(serde_json::Value::as_array_mut)
                && !content.is_empty()
            {
                content.push(cache.point());
            }
        }

        let mut body = serde_json::json!({
            "inferenceConfig": {
                "maxTokens": max_tokens
            },
            "messages": bedrock_messages
        });

        if !system_arr.is_empty() {
            body["system"] = serde_json::Value::Array(system_arr);
        }

        if !tools.is_empty() {
            let mut bedrock_tools: Vec<serde_json::Value> = tools
                .iter()
                .map(|tool| {
                    let name = tool.get("name").cloned().unwrap_or(serde_json::Value::Null);
                    let description = tool
                        .get("description")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let input_schema = tool
                        .get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({"type": "object"}));
                    serde_json::json!({
                        "toolSpec": {
                            "name": name,
                            "description": description,
                            "inputSchema": {
                                "json": input_schema
                            }
                        }
                    })
                })
                .collect();

            // Tools are the first section Bedrock evaluates and the most stable
            // thing Archon sends, so caching them is the largest single saving
            // available — and changing them invalidates everything downstream.
            if let Some(cache) = cache
                && cache.tools
            {
                bedrock_tools.push(cache.point());
            }

            body["toolConfig"] = serde_json::json!({"tools": bedrock_tools});
        }

        body
    }

    /// Build the Bedrock runtime URL for this provider.
    fn endpoint_url(&self) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/converse-stream",
            self.region,
            urlencoding::encode(&self.model_id)
        )
    }

    /// Where to place checkpoints for this request, if anywhere.
    ///
    /// The decision is not made here. It arrives resolved on the request, under
    /// [`crate::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY`] in `extra`, where
    /// `archon-core` put it after weighing everything this provider cannot see:
    /// `prompt_cache` on or off, the emission mode, the configured TTL, and any
    /// `[context.prompt_cache_models]` override. No directive means no
    /// checkpoints — an earlier draft decided down here from the model table
    /// alone, which meant `prompt_cache = false` did not turn caching off and
    /// the five-minute TTL preference was ignored in favour of one-hour writes,
    /// the expensive tier, on every request.
    ///
    /// What is decided here is what only the wire layer knows: whether the
    /// prompt clears the minimum. Bedrock measures that against `tools`,
    /// `system` and `messages` **combined** rather than per section, so the
    /// estimate spans all three. A checkpoint under the floor is discarded
    /// without an error, so emitting one anyway would not fail — it would just
    /// put fields on the wire that do nothing and report a cache that was never
    /// written.
    pub fn cache_placement(
        &self,
        request: &LlmRequest,
    ) -> Option<crate::cache_strategy::CachePointPlacement> {
        let directive = request
            .extra
            .get(crate::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY)?;
        // Belt and braces: the directive is only written for a BedrockCachePoint
        // strategy, which requires Claude — but the provider is the last line
        // before the wire, and a non-Claude body with a cachePoint is a 400.
        if !is_claude_model_id(&self.model_id) {
            return None;
        }
        let max = directive.get("max")?.as_u64()? as usize;
        let min_tokens = directive.get("min_tokens")?.as_u64()? as usize;
        let ttl_1h = directive
            .get("ttl_1h")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if estimated_prompt_tokens(&request.system, &request.messages, &request.tools) < min_tokens
        {
            return None;
        }

        let stable_system_blocks = directive
            .get("stable_system_blocks")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as usize);

        let mut placement = crate::cache_strategy::CachePointPlacement::all(ttl_1h)
            .with_stable_system_blocks(stable_system_blocks);
        // `prompt_cache_conversation = false` declines a checkpoint on the
        // message history and nothing else. Absent means yes, so a directive
        // written before this field existed keeps its old behaviour.
        placement.messages = directive
            .get("conversation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        // Exceeding the checkpoint limit is a hard 400, unlike falling short of
        // the minimum. Drop from the least stable section first: the messages
        // checkpoint moves every turn and is worth the least.
        if placement.count() > max {
            placement.messages = false;
        }
        if placement.count() > max {
            placement.system = false;
        }
        (placement.count() > 0).then_some(placement)
    }

    /// Whether `model_id` names an Anthropic model.
    ///
    /// Current Claude models are only reachable through a cross-region
    /// inference profile, whose ID carries a geo prefix (`us.anthropic....`)
    /// and may be given as a full ARN. A bare `starts_with("anthropic.")` misses
    /// both forms and silently drops thinking, prompt caching, and vision.
    pub(super) fn is_claude_model(&self) -> bool {
        is_claude_model_id(&self.model_id)
    }

    pub(super) async fn do_stream(
        &self,
        request: LlmRequest,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        crate::provider_fault::begin_request();
        let creds = resolve_credentials().await?;

        self.stream_at(request, &self.endpoint_url(), &creds).await
    }

    pub(super) async fn stream_at(
        &self,
        request: LlmRequest,
        url: &str,
        creds: &crate::providers::aws_auth::AwsCredentials,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        let body = Self::build_converse_body_cached(
            &request.system,
            &request.messages,
            &request.tools,
            request.max_tokens,
            self.cache_placement(&request),
        );

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| LlmError::Serialize(e.to_string().into()))?;

        // The one place a missing or misplaced `cachePoint` is visible. Bedrock
        // discards a checkpoint under the model's minimum without an error, so
        // a body that looks right and a body that silently caches nothing are
        // indistinguishable from the response alone.
        if tracing::enabled!(tracing::Level::DEBUG) {
            tracing::debug!(
                provider = "bedrock",
                model_id = %self.model_id,
                "Bedrock Converse request body: {}",
                crate::debug_body::debug_body(&String::from_utf8_lossy(&body_bytes))
            );
        }

        let url_parsed = Url::parse(&url)
            .map_err(|e| LlmError::Serialize(format!("invalid URL: {e}").into()))?;
        let host = url_parsed.host_str().unwrap_or("").to_string();
        let path = url_parsed.path().to_string();

        let signed = signed_headers(&creds, &host, &path, &self.region, &body_bytes);

        let mut req = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .header("x-amz-date", &signed.x_amz_date)
            .header("authorization", &signed.authorization);

        // Temporary credentials are only accepted with the session token
        // attached; it is part of the signature computed above.
        if let Some(ref token) = signed.security_token {
            req = req.header("x-amz-security-token", token);
        }

        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(crate::provider_fault::from_transport)?;

        let status = resp.status().as_u16();
        crate::provider_fault::note_response(
            status,
            crate::transport_http::response_retry_after(&resp),
        );
        if status >= 400 {
            return Err(crate::transport_http::response_error(resp, map_http_error).await);
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut buffer = Vec::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx.send(crate::provider_fault::transport_event(e)).await;
                        return;
                    }
                };
                buffer.extend_from_slice(&chunk);

                // ConverseStream replies in `application/vnd.amazon.eventstream`
                // — binary frames whose `:event-type` header names the event.
                // This used to run `String::from_utf8_lossy` over those frames
                // and scan for JSON, which found the payloads but never the
                // wrapper key `parse_bedrock_event` matches on, so no text ever
                // reached the caller and every turn ended empty without error.
                let (events, consumed) = decode_eventstream_frames(&buffer);
                if consumed > 0 {
                    buffer.drain(..consumed);
                }

                for event in events {
                    for stream_event in parse_bedrock_event(&event) {
                        if tx.send(stream_event).await.is_err() {
                            return;
                        }
                    }
                }
            }

            // Drain any remaining buffer content.
            if !buffer.is_empty() {
                let (events, _) = decode_eventstream_frames(&buffer);
                for event in events {
                    for stream_event in parse_bedrock_event(&event) {
                        if tx.send(stream_event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}
