use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitWindowKind {
    Requests,
    Tokens,
    Spend,
    Usage,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRateLimitWindow {
    pub id: String,
    pub provider_id: String,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub window_kind: RateLimitWindowKind,
    pub used_percent: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
    pub raw_redacted_json: Value,
    pub observed_at: DateTime<Utc>,
}

impl ProviderRateLimitWindow {
    pub fn new(provider_id: impl Into<String>, window_kind: RateLimitWindowKind) -> Self {
        Self {
            id: rate_limit_window_id(),
            provider_id: provider_id.into(),
            profile_id: None,
            model_id: None,
            limit_id: None,
            limit_name: None,
            window_kind,
            used_percent: None,
            resets_at: None,
            raw_redacted_json: Value::Object(Default::default()),
            observed_at: Utc::now(),
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

    pub fn with_limit(
        mut self,
        limit_id: impl Into<String>,
        limit_name: impl Into<String>,
    ) -> Self {
        self.limit_id = Some(limit_id.into());
        self.limit_name = Some(limit_name.into());
        self
    }

    pub fn with_used_percent(mut self, used_percent: f64) -> Self {
        self.used_percent = Some(used_percent.clamp(0.0, 100.0));
        self
    }

    pub fn with_resets_at(mut self, resets_at: DateTime<Utc>) -> Self {
        self.resets_at = Some(resets_at);
        self
    }

    pub fn with_redacted_json(mut self, value: Value) -> Self {
        self.raw_redacted_json = value;
        self
    }

    pub fn is_exhausted(&self) -> bool {
        self.used_percent
            .map(|used_percent| used_percent >= 100.0)
            .unwrap_or(false)
    }

    pub fn is_recent(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.observed_at) <= Duration::minutes(10)
    }
}

pub fn rate_limit_window_id() -> String {
    format!("provider-limit-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rate_limit_window_clamps_used_percent() {
        let window = ProviderRateLimitWindow::new("openai-codex", RateLimitWindowKind::Usage)
            .with_used_percent(175.0);

        assert_eq!(window.used_percent, Some(100.0));
        assert!(window.is_exhausted());
    }

    #[test]
    fn rate_limit_window_tracks_recent_cache_window() {
        let mut window = ProviderRateLimitWindow::new("anthropic", RateLimitWindowKind::Requests)
            .with_profile("oauth-primary")
            .with_model("claude-sonnet-4-6")
            .with_limit("requests", "Requests")
            .with_redacted_json(json!({"source": "headers"}));
        window.observed_at = Utc::now() - Duration::minutes(9);

        assert!(window.is_recent(Utc::now()));
        assert_eq!(window.raw_redacted_json["source"], "headers");
        assert!(window.id.starts_with("provider-limit-"));
    }

    #[test]
    fn stale_rate_limit_window_is_not_recent() {
        let mut window = ProviderRateLimitWindow::new("local", RateLimitWindowKind::Unknown);
        window.observed_at = Utc::now() - Duration::minutes(11);

        assert!(!window.is_recent(Utc::now()));
    }
}
