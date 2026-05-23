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

    pub fn world_model_third_party_embeddings_decision(&self) -> PolicyDecision {
        if !self.world_model.allow_third_party_embeddings {
            return PolicyDecision::deny(
                "third-party world-model embeddings are disabled by policy.world_model",
            );
        }
        if matches!(
            self.workers.embedding.as_str(),
            "allow-cloud" | "allow" | "cloud"
        ) && self.network.default == "allow"
        {
            PolicyDecision::allow("third-party world-model embeddings are enabled by policy")
        } else {
            PolicyDecision::deny(
                "third-party world-model embeddings require policy.workers.embedding = \"allow-cloud\" and policy.network.default = \"allow\"",
            )
        }
    }

    pub fn world_model_llm_labeler_decision(&self) -> PolicyDecision {
        if self.world_model.allow_llm_labeler {
            PolicyDecision::allow("world-model LLM labeling is enabled by policy")
        } else {
            PolicyDecision::deny("world-model LLM labeling is disabled by policy")
        }
    }

    pub fn world_model_behavior_change_decision(&self) -> PolicyDecision {
        if self.world_model.allow_behavior_changes {
            PolicyDecision::allow("world-model behavior-changing use is enabled by policy")
        } else {
            PolicyDecision::deny("world-model behavior-changing use is disabled by policy")
        }
    }

    pub fn reasoning_quality_llm_critic_decision(
        &self,
        data_flow_class: &str,
        third_party: bool,
    ) -> PolicyDecision {
        if !self.reasoning_quality.allow_llm_critic {
            return PolicyDecision::deny("reasoning-quality LLM critic is disabled by policy");
        }
        if third_party && !self.reasoning_quality.allow_third_party_critic {
            return PolicyDecision::deny(
                "third-party reasoning-quality critic is disabled by policy",
            );
        }
        if data_flow_class == "cloud" && !self.reasoning_quality.allow_critic_cloud_data_flow {
            return PolicyDecision::deny(
                "cloud critic data flow is disabled by policy.reasoning_quality",
            );
        }
        PolicyDecision::allow("reasoning-quality LLM critic is enabled by policy")
    }

    pub fn reasoning_quality_raw_text_decision(&self) -> PolicyDecision {
        if self.reasoning_quality.allow_raw_text_storage {
            PolicyDecision::allow("reasoning-quality raw text storage is enabled by policy")
        } else {
            PolicyDecision::deny("reasoning-quality raw text storage is disabled by policy")
        }
    }

    pub fn reasoning_quality_session_injection_decision(&self) -> PolicyDecision {
        if self.reasoning_quality.allow_session_start_injection {
            PolicyDecision::allow("reasoning-quality session briefing injection is enabled")
        } else {
            PolicyDecision::deny("reasoning-quality session briefing injection is disabled")
        }
    }

    pub fn video_ingest_decision(&self) -> PolicyDecision {
        if self.video.enabled {
            PolicyDecision::allow("video ingest is enabled by policy")
        } else {
            PolicyDecision::deny("video ingest is disabled by policy.video.enabled")
        }
    }

    pub fn video_acquisition_decision(&self, method: &str) -> PolicyDecision {
        let ingest = self.video_ingest_decision();
        if !ingest.allowed {
            return ingest;
        }
        match method {
            "LocalFile" | "None" => PolicyDecision::allow("local video acquisition is allowed"),
            "YouTube" => {
                if self.video.allow_youtube {
                    PolicyDecision::allow("YouTube video sources are enabled by policy")
                } else {
                    PolicyDecision::deny("YouTube video sources are disabled by policy.video")
                }
            }
            "DirectUrl" | "DirectDownload" => {
                if self.video.allow_direct_urls {
                    PolicyDecision::allow("direct video URLs are enabled by policy")
                } else {
                    PolicyDecision::deny("direct video URLs are disabled by policy.video")
                }
            }
            "ExternalDownloader" => {
                if self.video.allow_external_downloaders {
                    PolicyDecision::allow("external video downloader is enabled by policy")
                } else {
                    PolicyDecision::deny("external video downloader is disabled by policy.video")
                }
            }
            "BrowserAutomation" => {
                if self.video.allow_browser_automation {
                    PolicyDecision::allow("browser video acquisition is enabled by policy")
                } else {
                    PolicyDecision::deny("browser video acquisition is disabled by policy.video")
                }
            }
            other => {
                PolicyDecision::deny(format!("unsupported video acquisition method '{other}'"))
            }
        }
    }

    pub fn video_asr_decision(&self) -> PolicyDecision {
        let ingest = self.video_ingest_decision();
        if !ingest.allowed {
            return ingest;
        }
        if video_asr_is_local(&self.video.asr.provider) {
            return PolicyDecision::allow("local video ASR is allowed by policy");
        }
        if !self.video.allow_cloud_asr {
            return PolicyDecision::deny("cloud video ASR is disabled by policy.video");
        }
        if !cloud_worker_allowed(&self.workers.ocr) || !self.network.allow_cloud_vlm {
            return PolicyDecision::deny(
                "cloud video ASR requires policy.workers.ocr and policy.network.allow_cloud_vlm",
            );
        }
        PolicyDecision::allow("cloud video ASR is allowed by policy")
    }

    pub fn video_vlm_decision(&self) -> PolicyDecision {
        let ingest = self.video_ingest_decision();
        if !ingest.allowed {
            return ingest;
        }
        if !self.video.frames.vlm {
            return PolicyDecision::deny("video frame VLM is disabled by policy.video.frames");
        }
        if docs_vlm_looks_cloud(self) && !self.video.allow_cloud_vlm {
            return PolicyDecision::deny("cloud video VLM is disabled by policy.video");
        }
        self.docs_vlm_decision()
    }

    pub fn video_summary_decision(&self) -> PolicyDecision {
        let ingest = self.video_ingest_decision();
        if !ingest.allowed {
            return ingest;
        }
        if !self.video.summary.enabled {
            return PolicyDecision::deny("video summary is disabled by policy.video.summary");
        }
        if !self.video.summary.allow_llm_summary {
            return PolicyDecision::deny("video LLM summary is disabled by policy.video.summary");
        }
        if summary_provider_is_cloud(&self.video.summary.provider) {
            if !self.video.summary.allow_cloud_summary {
                return PolicyDecision::deny(
                    "cloud video summary is disabled by policy.video.summary",
                );
            }
            if !cloud_worker_allowed(&self.workers.vlm)
                || !self.network.allow_cloud_vlm
                || !self.docs.vlm.allow_cloud
            {
                return PolicyDecision::deny(
                    "cloud video summary requires workers, network, and docs VLM cloud gates",
                );
            }
        }
        PolicyDecision::allow("video summary is allowed by policy")
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

fn video_asr_is_local(provider: &str) -> bool {
    matches!(
        provider,
        "whisper-rs" | "whisper-cli" | "local" | "disabled" | ""
    )
}

fn cloud_worker_allowed(worker_policy: &str) -> bool {
    matches!(worker_policy, "allow-cloud" | "allow" | "cloud")
}

fn docs_vlm_looks_cloud(policy: &EffectivePolicy) -> bool {
    let vlm = &policy.docs.vlm;
    match vlm.provider.as_str() {
        "gemini" | "anthropic" => true,
        "openai-compat" => {
            vlm.mode == "cloud" || !endpoint_looks_local(&vlm.openai_compat.endpoint)
        }
        _ => vlm.mode == "cloud",
    }
}

fn summary_provider_is_cloud(provider: &str) -> bool {
    !matches!(provider, "disabled" | "local" | "ollama" | "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_quality_cloud_critic_is_denied_by_default() {
        let policy = EffectivePolicy::default();
        let decision = policy.reasoning_quality_llm_critic_decision("cloud", false);
        assert!(!decision.allowed);
    }

    #[test]
    fn reasoning_quality_cloud_critic_requires_cloud_flow_gate() {
        let mut policy = EffectivePolicy::default();
        policy.reasoning_quality.allow_llm_critic = true;
        let denied = policy.reasoning_quality_llm_critic_decision("cloud", false);
        assert!(!denied.allowed);

        policy.reasoning_quality.allow_critic_cloud_data_flow = true;
        let allowed = policy.reasoning_quality_llm_critic_decision("cloud", false);
        assert!(allowed.allowed);
    }
}
