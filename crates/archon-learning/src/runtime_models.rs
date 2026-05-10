use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderRuntimeEventRecord {
    pub event_id: String,
    pub provider_id: String,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub runtime_mode: String,
    pub event_type: String,
    pub severity: String,
    pub reason_code: Option<String>,
    pub message: Option<String>,
    pub retry_count: Option<u32>,
    pub fallback_from: Option<String>,
    pub fallback_to: Option<String>,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub pipeline_id: Option<String>,
    pub raw_redacted_json: serde_json::Value,
    pub created_at: String,
}

impl ProviderRuntimeEventRecord {
    pub fn new(
        event_id: impl Into<String>,
        provider_id: impl Into<String>,
        runtime_mode: impl Into<String>,
        event_type: impl Into<String>,
        severity: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            provider_id: provider_id.into(),
            profile_id: None,
            model_id: None,
            runtime_mode: runtime_mode.into(),
            event_type: event_type.into(),
            severity: severity.into(),
            reason_code: None,
            message: None,
            retry_count: None,
            fallback_from: None,
            fallback_to: None,
            request_id: None,
            run_id: None,
            pipeline_id: None,
            raw_redacted_json: serde_json::Value::Object(Default::default()),
            created_at: created_at.into(),
        }
    }

    pub fn with_profile(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_reason(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_retry_count(mut self, retry_count: u32) -> Self {
        self.retry_count = Some(retry_count);
        self
    }

    pub fn with_fallback(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.fallback_from = Some(from.into());
        self.fallback_to = Some(to.into());
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_pipeline_id(mut self, pipeline_id: impl Into<String>) -> Self {
        self.pipeline_id = Some(pipeline_id.into());
        self
    }

    pub fn with_redacted_json(mut self, raw_redacted_json: serde_json::Value) -> Self {
        self.raw_redacted_json = raw_redacted_json;
        self
    }
}
