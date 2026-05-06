use crate::models::{EffectivePolicy, PolicyDecision};

impl EffectivePolicy {
    pub fn docs_vlm_decision(&self) -> PolicyDecision {
        let vlm = &self.docs.vlm;
        if !vlm.enabled || vlm.mode == "disabled" || vlm.provider == "disabled" {
            return PolicyDecision::deny("VLM descriptions are disabled by policy");
        }
        match vlm.provider.as_str() {
            "ollama" => allow_provider_mode(vlm.mode.as_str(), "local", || {
                allow_local_vlm(&self.workers.vlm)
            }),
            "gemini" | "anthropic" => {
                allow_provider_mode(vlm.mode.as_str(), "cloud", || allow_cloud_vlm(self))
            }
            "openai-compat" => allow_openai_compat_vlm(self),
            other => PolicyDecision::deny(format!("unsupported VLM provider '{other}'")),
        }
    }

    pub fn docs_vlm_mode_decision(&self) -> PolicyDecision {
        let vlm = &self.docs.vlm;
        if !vlm.enabled || vlm.mode == "disabled" {
            return PolicyDecision::deny("VLM descriptions are disabled by policy");
        }
        match vlm.mode.as_str() {
            "local" => allow_local_vlm(&self.workers.vlm),
            "cloud" => allow_cloud_vlm(self),
            "hybrid" => {
                if allow_local_vlm(&self.workers.vlm).allowed {
                    PolicyDecision::allow("hybrid VLM allowed through local provider")
                } else {
                    allow_cloud_vlm(self)
                }
            }
            other => PolicyDecision::deny(format!("unsupported VLM mode '{other}'")),
        }
    }

    pub fn gametheory_tier11_decision(&self) -> PolicyDecision {
        if self.gametheory.enable_tier11 {
            PolicyDecision::allow("Tier 11 is enabled by policy")
        } else {
            PolicyDecision::deny("Tier 11 is disabled by policy")
        }
    }

    pub fn gametheory_web_tools_decision(&self) -> PolicyDecision {
        if self.gametheory.allow_web_tools && self.network.allow_web_strategy_agents {
            PolicyDecision::allow("web strategy tools are enabled by policy")
        } else {
            PolicyDecision::deny(
                "web strategy tools require policy.gametheory.allow_web_tools and policy.network.allow_web_strategy_agents",
            )
        }
    }

    pub fn learning_auto_apply_decision(
        &self,
        manifest_kind: &str,
        risk_level: &str,
    ) -> PolicyDecision {
        if !self.learning.auto_apply_low_risk {
            return PolicyDecision::deny("learning auto-apply is disabled by policy");
        }
        if matches!(risk_level, "High" | "Critical" | "high" | "critical") {
            return PolicyDecision::deny("high-risk behaviour changes require approval");
        }
        if manifest_kind == "PromptProfile" && self.learning.require_approval_for_prompt_changes {
            return PolicyDecision::deny("prompt changes require approval by policy");
        }
        if manifest_kind == "PipelineGates" && self.learning.require_approval_for_blocking_gates {
            return PolicyDecision::deny("blocking gate changes require approval by policy");
        }
        if manifest_kind == "PolicyOverride"
            || self.learning.require_approval_for_network_changes
                && manifest_kind == "NetworkAccess"
        {
            return PolicyDecision::deny("policy or network changes require approval by policy");
        }
        PolicyDecision::allow("low-risk auto-apply is enabled by policy")
    }

    pub fn mcp_exposure_decision(&self) -> PolicyDecision {
        if self.network.allow_mcp_server_exposure {
            PolicyDecision::allow("MCP server exposure is enabled by policy")
        } else {
            PolicyDecision::deny("MCP server exposure is disabled by policy")
        }
    }
}

fn allow_provider_mode(
    configured_mode: &str,
    provider_class: &str,
    decision: impl FnOnce() -> PolicyDecision,
) -> PolicyDecision {
    match (configured_mode, provider_class) {
        ("local", "local") | ("cloud", "cloud") | ("hybrid", _) => decision(),
        ("disabled", _) => PolicyDecision::deny("VLM descriptions are disabled by policy"),
        ("local", "cloud") => PolicyDecision::deny(
            "cloud VLM provider requires policy.docs.vlm.mode = \"cloud\" or \"hybrid\"",
        ),
        ("cloud", "local") => PolicyDecision::deny(
            "local VLM provider requires policy.docs.vlm.mode = \"local\" or \"hybrid\"",
        ),
        (other, _) => PolicyDecision::deny(format!("unsupported VLM mode '{other}'")),
    }
}

fn allow_local_vlm(worker_policy: &str) -> PolicyDecision {
    if matches!(worker_policy, "allow-local" | "allow" | "local") {
        PolicyDecision::allow("local VLM provider is allowed by policy")
    } else {
        PolicyDecision::deny("local VLM provider is denied by policy.workers.vlm")
    }
}

fn allow_cloud_vlm(policy: &EffectivePolicy) -> PolicyDecision {
    if !matches!(
        policy.workers.vlm.as_str(),
        "allow-cloud" | "allow" | "cloud"
    ) {
        return PolicyDecision::deny("cloud VLM provider is denied by policy.workers.vlm");
    }
    if policy.docs.vlm.allow_cloud && policy.network.allow_cloud_vlm {
        PolicyDecision::allow("cloud VLM provider is allowed by policy")
    } else {
        PolicyDecision::deny(
            "cloud VLM requires policy.docs.vlm.allow_cloud and policy.network.allow_cloud_vlm",
        )
    }
}

fn allow_openai_compat_vlm(policy: &EffectivePolicy) -> PolicyDecision {
    let vlm = &policy.docs.vlm;
    match vlm.mode.as_str() {
        "local" => allow_local_vlm(&policy.workers.vlm),
        "cloud" => allow_cloud_vlm(policy),
        "hybrid" => {
            if endpoint_looks_local(&vlm.openai_compat.endpoint) {
                allow_local_vlm(&policy.workers.vlm)
            } else {
                allow_cloud_vlm(policy)
            }
        }
        "disabled" => PolicyDecision::deny("VLM descriptions are disabled by policy"),
        other => PolicyDecision::deny(format!("unsupported VLM mode '{other}'")),
    }
}

fn endpoint_looks_local(endpoint: &str) -> bool {
    let endpoint = endpoint.trim().to_ascii_lowercase();
    endpoint.starts_with("http://localhost")
        || endpoint.starts_with("https://localhost")
        || endpoint.starts_with("http://127.")
        || endpoint.starts_with("https://127.")
        || endpoint.starts_with("http://[::1]")
        || endpoint.starts_with("https://[::1]")
        || endpoint.starts_with("http://0.0.0.0")
        || endpoint.starts_with("https://0.0.0.0")
}
