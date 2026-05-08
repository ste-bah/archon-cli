use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::rate_limits::ProviderRateLimitWindow;
use super::redaction::redact_provider_metadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthStatus {
    Healthy,
    Degraded,
    Unavailable,
    MissingCredentials,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderIdentityStatus {
    Clean,
    Spoof,
    Custom,
    AppServer,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRuntimeStatus {
    pub provider_id: String,
    pub display_name: Option<String>,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub runtime_mode: String,
    pub identity_status: ProviderIdentityStatus,
    pub health: ProviderHealthStatus,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub rate_limits: Vec<ProviderRateLimitWindow>,
    pub metadata_redacted_json: Value,
}

impl ProviderRuntimeStatus {
    pub fn new(provider_id: impl Into<String>, runtime_mode: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            display_name: None,
            profile_id: None,
            model_id: None,
            runtime_mode: runtime_mode.into(),
            identity_status: ProviderIdentityStatus::NotApplicable,
            health: ProviderHealthStatus::Unknown,
            last_success_at: None,
            last_failure_at: None,
            rate_limits: Vec::new(),
            metadata_redacted_json: Value::Object(Default::default()),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_profile(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_identity_status(mut self, identity_status: ProviderIdentityStatus) -> Self {
        self.identity_status = identity_status;
        self
    }

    pub fn with_health(mut self, health: ProviderHealthStatus) -> Self {
        self.health = health;
        self
    }

    pub fn with_last_success(mut self, last_success_at: DateTime<Utc>) -> Self {
        self.last_success_at = Some(last_success_at);
        self
    }

    pub fn with_last_failure(mut self, last_failure_at: DateTime<Utc>) -> Self {
        self.last_failure_at = Some(last_failure_at);
        self
    }

    pub fn with_rate_limits(mut self, rate_limits: Vec<ProviderRateLimitWindow>) -> Self {
        self.rate_limits = rate_limits;
        self
    }

    pub fn with_redacted_json(mut self, value: Value) -> Self {
        self.metadata_redacted_json = redact_provider_metadata(value);
        self
    }

    pub fn is_available(&self) -> bool {
        matches!(
            self.health,
            ProviderHealthStatus::Healthy | ProviderHealthStatus::Degraded
        )
    }

    pub fn exhausted_limits(&self) -> Vec<&ProviderRateLimitWindow> {
        self.rate_limits
            .iter()
            .filter(|window| window.is_exhausted())
            .collect()
    }

    pub fn recent_limits(&self, now: DateTime<Utc>) -> Vec<&ProviderRateLimitWindow> {
        self.rate_limits
            .iter()
            .filter(|window| window.is_recent(now))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RateLimitWindowKind;
    use chrono::Duration;
    use serde_json::json;

    #[test]
    fn status_reports_availability_from_health() {
        let healthy = ProviderRuntimeStatus::new("anthropic", "direct")
            .with_health(ProviderHealthStatus::Healthy);
        let missing = ProviderRuntimeStatus::new("openai", "direct")
            .with_health(ProviderHealthStatus::MissingCredentials);

        assert!(healthy.is_available());
        assert!(!missing.is_available());
    }

    #[test]
    fn status_filters_exhausted_and_recent_rate_limits() {
        let now = Utc::now();
        let exhausted = ProviderRateLimitWindow::new("openai-codex", RateLimitWindowKind::Usage)
            .with_used_percent(100.0);
        let mut stale = ProviderRateLimitWindow::new("openai-codex", RateLimitWindowKind::Tokens)
            .with_used_percent(50.0);
        stale.observed_at = now - Duration::minutes(20);
        let status = ProviderRuntimeStatus::new("openai-codex", "auto")
            .with_rate_limits(vec![exhausted, stale]);

        assert_eq!(status.exhausted_limits().len(), 1);
        assert_eq!(status.recent_limits(now).len(), 1);
    }

    #[test]
    fn status_serializes_identity_mode_for_anthropic_spoofing() {
        let status = ProviderRuntimeStatus::new("anthropic", "direct")
            .with_identity_status(ProviderIdentityStatus::Spoof)
            .with_profile("oauth-primary")
            .with_model("claude-sonnet-4-6")
            .with_redacted_json(json!({"spoof_reason": "oauth"}));

        let value = serde_json::to_value(status).unwrap();

        assert_eq!(value["identity_status"], "spoof");
        assert_eq!(value["metadata_redacted_json"]["spoof_reason"], "oauth");
    }

    #[test]
    fn status_metadata_redacts_sensitive_values() {
        let status = ProviderRuntimeStatus::new("openai", "direct").with_redacted_json(json!({
            "api_key": "secret",
            "region": "us-east-1"
        }));

        assert_eq!(status.metadata_redacted_json["api_key"], "[redacted]");
        assert_eq!(status.metadata_redacted_json["region"], "us-east-1");
    }
}
