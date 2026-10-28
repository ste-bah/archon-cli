use archon_llm::provider::LlmError;

pub(super) fn to_llm_error(
    status: Option<u16>,
    retry_after_secs: Option<u64>,
    overloaded: bool,
    message: String,
) -> Option<LlmError> {
    if overloaded {
        Some(LlmError::Overloaded {
            status,
            retry_after_secs,
            message,
        })
    } else if let Some(status @ (429 | 529)) = status {
        Some(LlmError::Overloaded {
            status: Some(status),
            retry_after_secs,
            message,
        })
    } else if let Some(retry_after_secs) = retry_after_secs {
        Some(LlmError::RateLimited {
            retry_after_secs,
            from_provider: true,
            pipeline_retry_after_secs: None,
            message,
        })
    } else {
        None
    }
}
