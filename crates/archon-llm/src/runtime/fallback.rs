use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFallbackPolicy {
    pub enabled: bool,
    pub allow_codex_app_server_to_direct: bool,
    pub allow_identity_surface_change: bool,
    pub allow_cross_provider_family: bool,
}

impl Default for ProviderFallbackPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_codex_app_server_to_direct: false,
            allow_identity_surface_change: false,
            allow_cross_provider_family: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderFallbackRequest {
    pub from_provider_id: String,
    pub to_provider_id: String,
    pub from_profile_id: Option<String>,
    pub to_profile_id: Option<String>,
    pub from_model_id: Option<String>,
    pub to_model_id: Option<String>,
    pub from_runtime_mode: String,
    pub to_runtime_mode: String,
    pub reason_code: String,
    pub identity_surface_change: bool,
    pub capability_surface_change: bool,
    pub metadata_redacted_json: Value,
}

impl ProviderFallbackRequest {
    pub fn new(
        from_provider_id: impl Into<String>,
        to_provider_id: impl Into<String>,
        from_runtime_mode: impl Into<String>,
        to_runtime_mode: impl Into<String>,
        reason_code: impl Into<String>,
    ) -> Self {
        Self {
            from_provider_id: from_provider_id.into(),
            to_provider_id: to_provider_id.into(),
            from_profile_id: None,
            to_profile_id: None,
            from_model_id: None,
            to_model_id: None,
            from_runtime_mode: from_runtime_mode.into(),
            to_runtime_mode: to_runtime_mode.into(),
            reason_code: reason_code.into(),
            identity_surface_change: false,
            capability_surface_change: false,
            metadata_redacted_json: Value::Object(Default::default()),
        }
    }

    pub fn with_profiles(
        mut self,
        from_profile_id: impl Into<String>,
        to_profile_id: impl Into<String>,
    ) -> Self {
        self.from_profile_id = Some(from_profile_id.into());
        self.to_profile_id = Some(to_profile_id.into());
        self
    }

    pub fn with_models(
        mut self,
        from_model_id: impl Into<String>,
        to_model_id: impl Into<String>,
    ) -> Self {
        self.from_model_id = Some(from_model_id.into());
        self.to_model_id = Some(to_model_id.into());
        self
    }

    pub fn with_identity_surface_change(mut self) -> Self {
        self.identity_surface_change = true;
        self
    }

    pub fn with_capability_surface_change(mut self) -> Self {
        self.capability_surface_change = true;
        self
    }

    pub fn with_redacted_json(mut self, value: Value) -> Self {
        self.metadata_redacted_json = value;
        self
    }

    pub fn same_provider(&self) -> bool {
        self.from_provider_id == self.to_provider_id
    }

    pub fn is_codex_app_server_to_direct(&self) -> bool {
        self.from_provider_id == "openai-codex"
            && self.to_provider_id == "openai-codex"
            && self.from_runtime_mode == "app_server"
            && self.to_runtime_mode == "direct"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFallbackVerdict {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFallbackDecision {
    pub verdict: ProviderFallbackVerdict,
    pub reason_code: String,
}

impl ProviderFallbackDecision {
    pub fn allowed(reason_code: impl Into<String>) -> Self {
        Self {
            verdict: ProviderFallbackVerdict::Allowed,
            reason_code: reason_code.into(),
        }
    }

    pub fn denied(reason_code: impl Into<String>) -> Self {
        Self {
            verdict: ProviderFallbackVerdict::Denied,
            reason_code: reason_code.into(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        self.verdict == ProviderFallbackVerdict::Allowed
    }
}

pub fn decide_fallback(
    policy: &ProviderFallbackPolicy,
    request: &ProviderFallbackRequest,
) -> ProviderFallbackDecision {
    if request.is_codex_app_server_to_direct() && policy.allow_codex_app_server_to_direct {
        return ProviderFallbackDecision::allowed("codex_direct_fallback_allowed");
    }
    if !policy.enabled {
        return ProviderFallbackDecision::denied("fallback_disabled");
    }
    if request.identity_surface_change && !policy.allow_identity_surface_change {
        return ProviderFallbackDecision::denied("identity_surface_change_denied");
    }
    if !request.same_provider() && !policy.allow_cross_provider_family {
        return ProviderFallbackDecision::denied("cross_provider_family_denied");
    }
    ProviderFallbackDecision::allowed("fallback_policy_allowed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_app_server_to_direct_can_be_allowed_explicitly() {
        let policy = ProviderFallbackPolicy {
            allow_codex_app_server_to_direct: true,
            ..ProviderFallbackPolicy::default()
        };
        let request = ProviderFallbackRequest::new(
            "openai-codex",
            "openai-codex",
            "app_server",
            "direct",
            "app_server_unavailable",
        );

        let decision = decide_fallback(&policy, &request);

        assert!(decision.is_allowed());
        assert_eq!(decision.reason_code, "codex_direct_fallback_allowed");
    }

    #[test]
    fn anthropic_spoof_to_clean_is_denied_without_identity_opt_in() {
        let policy = ProviderFallbackPolicy {
            enabled: true,
            ..ProviderFallbackPolicy::default()
        };
        let request = ProviderFallbackRequest::new(
            "anthropic",
            "anthropic",
            "spoof",
            "clean",
            "oauth_failed",
        )
        .with_identity_surface_change();

        let decision = decide_fallback(&policy, &request);

        assert!(!decision.is_allowed());
        assert_eq!(decision.reason_code, "identity_surface_change_denied");
    }

    #[test]
    fn cross_provider_family_is_denied_by_default() {
        let policy = ProviderFallbackPolicy {
            enabled: true,
            ..ProviderFallbackPolicy::default()
        };
        let request = ProviderFallbackRequest::new(
            "anthropic",
            "openai",
            "direct",
            "direct",
            "quota_exceeded",
        );

        let decision = decide_fallback(&policy, &request);

        assert!(!decision.is_allowed());
        assert_eq!(decision.reason_code, "cross_provider_family_denied");
    }

    #[test]
    fn enabled_policy_allows_same_provider_non_identity_fallback() {
        let policy = ProviderFallbackPolicy {
            enabled: true,
            ..ProviderFallbackPolicy::default()
        };
        let request = ProviderFallbackRequest::new(
            "anthropic",
            "anthropic",
            "direct",
            "direct",
            "profile_cooldown",
        )
        .with_profiles("primary", "secondary");

        let decision = decide_fallback(&policy, &request);

        assert!(decision.is_allowed());
        assert_eq!(decision.reason_code, "fallback_policy_allowed");
    }
}
