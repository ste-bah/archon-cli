use super::*;

pub(super) fn serialized_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

pub(super) fn approx_tokens_from_bytes(bytes: usize) -> u64 {
    bytes.div_ceil(4) as u64
}

pub fn build_reasoning_config(model: &str, effort: Option<&str>) -> Option<ReasoningConfig> {
    effort.map(|effort| ReasoningConfig {
        effort: Some(clamp_reasoning_effort(model, effort)),
        summary: Some("auto".into()),
    })
}

pub fn clamp_reasoning_effort(model_id: &str, effort: &str) -> String {
    let id = model_id.rsplit('/').next().unwrap_or(model_id);
    if (id.starts_with("gpt-5.2")
        || id.starts_with("gpt-5.3")
        || id.starts_with("gpt-5.4")
        || id.starts_with("gpt-5.5"))
        && effort == "minimal"
    {
        return "low".into();
    }
    if id == "gpt-5.1" && effort == "xhigh" {
        return "high".into();
    }
    if id == "gpt-5.3-codex-mini" {
        if effort == "high" || effort == "xhigh" {
            return "high".into();
        }
        return "medium".into();
    }
    effort.into()
}

pub fn build_codex_headers(
    creds: &CodexCredentials,
    spoof: &SpoofConfig,
    session_id: &str,
) -> Result<HeaderMap, LlmError> {
    let mut headers = HeaderMap::new();
    insert_header(
        &mut headers,
        "Authorization",
        &format!("Bearer {}", creds.access_token.expose()),
    )?;
    insert_header(&mut headers, "chatgpt-account-id", &creds.account_id)?;
    insert_header(&mut headers, "originator", &spoof.originator)?;
    insert_header(&mut headers, "User-Agent", &spoof.user_agent)?;
    insert_header(&mut headers, "OpenAI-Beta", &spoof.openai_beta)?;
    insert_header(&mut headers, "accept", "text/event-stream")?;
    insert_header(&mut headers, "content-type", "application/json")?;
    insert_header(&mut headers, "session_id", session_id)?;
    insert_header(&mut headers, "x-client-request-id", session_id)?;

    for (key, value) in &spoof.extra_headers {
        insert_header(&mut headers, key, value)?;
    }
    Ok(headers)
}

pub fn validate_spoof_headers(spoof: &SpoofConfig) -> Result<(), LlmError> {
    for key in spoof.extra_headers.keys() {
        let lower = key.to_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str()) {
            return Err(LlmError::Auth(
                format!("spoof.extra_headers contains reserved header: {key}").into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn insert_header(
    headers: &mut HeaderMap,
    key: &str,
    value: &str,
) -> Result<(), LlmError> {
    let name = HeaderName::from_bytes(key.as_bytes())
        .map_err(|e| LlmError::Serialize(format!("invalid header name `{key}`: {e}").into()))?;
    let value = HeaderValue::from_str(value).map_err(|e| {
        LlmError::Serialize(format!("invalid header value for `{key}`: {e}").into())
    })?;
    headers.insert(name, value);
    Ok(())
}

pub(super) fn is_retryable(status: u16, body: &str) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
        || crate::provider_fault::rate_limit_matches(
            &body.to_ascii_lowercase(),
            crate::provider_fault::RateLimitEvidence::Codex,
        )
        || contains_any(
            body,
            &[
                "overloaded",
                "service unavailable",
                "upstream connect",
                "connection refused",
            ],
        )
}

pub(super) fn is_quota_exceeded(body: &str) -> bool {
    body.to_lowercase().contains("usage limit")
}

pub(super) fn contains_any(body: &str, needles: &[&str]) -> bool {
    let body = body.to_lowercase();
    needles.iter().any(|needle| body.contains(needle))
}

pub(super) fn map_codex_error(status: u16, body: String, refreshed: bool) -> LlmError {
    if let Some(error) = crate::provider_fault::structured_rejection(status, &body) {
        return error;
    }
    if status == 401 && body.to_lowercase().contains("originator mismatch") {
        return LlmError::Auth(
            "Codex spoof manifest outdated, run `archon update-codex-compat`".into(),
        );
    }
    let error = crate::provider_fault::from_status(status, &Default::default(), body.clone());
    match status {
        401 if refreshed => LlmError::Auth("Codex token rejected after refresh".into()),
        401 | 403 => LlmError::Auth(body.into()),
        429 if is_quota_exceeded(&body) => LlmError::QuotaExceeded(body.into()),
        429 => LlmError::RateLimited {
            retry_after_secs: 1,
            from_provider: false,
            pipeline_retry_after_secs: None,
            message: crate::provider::DiagnosticMessage::redacted_at_source(&body),
        },
        500..=599 => LlmError::Server {
            status,
            message: body.into(),
            pipeline_retry_after_secs: None,
        },
        _ => error,
    }
}

pub(super) fn auth_to_llm_error(err: AuthError) -> LlmError {
    err.into()
}

#[cfg(test)]
mod diagnostic_redaction_tests {
    fn convert(status: u16, body: &str) -> crate::provider::LlmError {
        super::map_codex_error(status, body.into(), false)
    }
    include!("../../provider_fault_redaction_cases.rs");
}

#[cfg(test)]
#[path = "client_protocol_characterization_tests.rs"]
mod c0_characterization;
