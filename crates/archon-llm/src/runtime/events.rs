use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRuntimeEventType {
    RequestStarted,
    RequestSucceeded,
    RequestFailed,
    TokenRefreshed,
    TokenRefreshFailed,
    RateLimitObserved,
    UsageLimitObserved,
    ProfileCooldownStarted,
    ProfileCooldownCleared,
    FallbackSelected,
    FallbackDenied,
    SpoofIdentitySelected,
    SpoofIdentityRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRuntimeSeverity {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRuntimeEvent {
    pub event_id: String,
    pub provider_id: String,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub runtime_mode: String,
    pub event_type: ProviderRuntimeEventType,
    pub severity: ProviderRuntimeSeverity,
    pub reason_code: Option<String>,
    pub message: Option<String>,
    pub retry_count: Option<u32>,
    pub fallback_from: Option<String>,
    pub fallback_to: Option<String>,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub pipeline_id: Option<String>,
    pub raw_redacted_json: Value,
    pub created_at: DateTime<Utc>,
}

impl ProviderRuntimeEvent {
    pub fn new(
        provider_id: impl Into<String>,
        runtime_mode: impl Into<String>,
        event_type: ProviderRuntimeEventType,
        severity: ProviderRuntimeSeverity,
    ) -> Self {
        Self {
            event_id: provider_runtime_event_id(),
            provider_id: provider_id.into(),
            profile_id: None,
            model_id: None,
            runtime_mode: runtime_mode.into(),
            event_type,
            severity,
            reason_code: None,
            message: None,
            retry_count: None,
            fallback_from: None,
            fallback_to: None,
            request_id: None,
            run_id: None,
            pipeline_id: None,
            raw_redacted_json: Value::Object(Default::default()),
            created_at: Utc::now(),
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

    pub fn with_fallback(
        mut self,
        fallback_from: impl Into<String>,
        fallback_to: impl Into<String>,
    ) -> Self {
        self.fallback_from = Some(fallback_from.into());
        self.fallback_to = Some(fallback_to.into());
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

    pub fn with_redacted_json(mut self, value: Value) -> Self {
        self.raw_redacted_json = value;
        self
    }
}

pub fn provider_runtime_event_id() -> String {
    format!("provider-event-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_runtime_event_serializes_snake_case_types() {
        let event = ProviderRuntimeEvent::new(
            "anthropic",
            "direct",
            ProviderRuntimeEventType::SpoofIdentitySelected,
            ProviderRuntimeSeverity::Info,
        )
        .with_profile("oauth-primary")
        .with_model("claude-sonnet-4-6")
        .with_reason("oauth")
        .with_redacted_json(json!({"auth_kind": "oauth"}));

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["event_type"], "spoof_identity_selected");
        assert_eq!(value["severity"], "info");
        assert_eq!(value["raw_redacted_json"]["auth_kind"], "oauth");
    }

    #[test]
    fn fallback_event_records_source_and_target() {
        let event = ProviderRuntimeEvent::new(
            "openai-codex",
            "auto",
            ProviderRuntimeEventType::FallbackSelected,
            ProviderRuntimeSeverity::Warn,
        )
        .with_fallback("app_server", "direct")
        .with_retry_count(1)
        .with_request_id("req-123");

        assert_eq!(event.fallback_from.as_deref(), Some("app_server"));
        assert_eq!(event.fallback_to.as_deref(), Some("direct"));
        assert_eq!(event.retry_count, Some(1));
        assert!(event.event_id.starts_with("provider-event-"));
    }
}
