use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub network: NetworkPolicy,
    pub workers: WorkersPolicy,
    pub gametheory: GameTheoryPolicy,
    pub learning: LearningPolicy,
    pub docs: DocsPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub default: String,
    pub allow_cloud_vlm: bool,
    pub allow_web_strategy_agents: bool,
    pub allow_mcp_server_exposure: bool,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            default: "deny".into(),
            allow_cloud_vlm: false,
            allow_web_strategy_agents: false,
            allow_mcp_server_exposure: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkersPolicy {
    pub ocr: String,
    pub embedding: String,
    pub vlm: String,
    pub web_fetch: String,
}

impl Default for WorkersPolicy {
    fn default() -> Self {
        Self {
            ocr: "allow-local".into(),
            embedding: "allow-local".into(),
            vlm: "deny".into(),
            web_fetch: "deny".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameTheoryPolicy {
    pub max_agents_per_council: usize,
    pub max_cost_usd: f64,
    pub enable_tier11: bool,
    pub allow_web_tools: bool,
}

impl Default for GameTheoryPolicy {
    fn default() -> Self {
        Self {
            max_agents_per_council: 12,
            max_cost_usd: 20.0,
            enable_tier11: false,
            allow_web_tools: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningPolicy {
    pub auto_apply_low_risk: bool,
    pub require_approval_for_prompt_changes: bool,
    pub require_approval_for_blocking_gates: bool,
    pub require_approval_for_network_changes: bool,
}

impl Default for LearningPolicy {
    fn default() -> Self {
        Self {
            auto_apply_low_risk: false,
            require_approval_for_prompt_changes: true,
            require_approval_for_blocking_gates: true,
            require_approval_for_network_changes: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocsPolicy {
    pub vlm: VlmPolicy,
    pub pdf: PdfPolicy,
    pub retrieval: RetrievalPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfPolicy {
    pub extract_embedded_images: bool,
    pub min_image_dimension: u32,
    pub min_image_bytes: u64,
    pub vlm_per_page_image: bool,
    pub render_text_pdf_pages: bool,
}

impl Default for PdfPolicy {
    fn default() -> Self {
        Self {
            extract_embedded_images: true,
            min_image_dimension: 200,
            min_image_bytes: 4096,
            vlm_per_page_image: true,
            render_text_pdf_pages: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalPolicy {
    pub exact_weight: f64,
    pub semantic_weight: f64,
}

impl Default for RetrievalPolicy {
    fn default() -> Self {
        Self {
            exact_weight: 0.45,
            semantic_weight: 0.55,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlmPolicy {
    pub enabled: bool,
    pub mode: String,
    pub provider: String,
    pub allow_cloud: bool,
    pub require_user_confirmation_for_cloud: bool,
    pub ollama: OllamaVlmPolicy,
    pub gemini: GeminiVlmPolicy,
    pub anthropic: AnthropicVlmPolicy,
}

impl Default for VlmPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "disabled".into(),
            provider: "disabled".into(),
            allow_cloud: false,
            require_user_confirmation_for_cloud: true,
            ollama: OllamaVlmPolicy::default(),
            gemini: GeminiVlmPolicy::default(),
            anthropic: AnthropicVlmPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OllamaVlmPolicy {
    pub endpoint: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl Default for OllamaVlmPolicy {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".into(),
            model: "gemma4:e4b".into(),
            timeout_secs: 120,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiVlmPolicy {
    pub api_key_env: String,
    pub model: String,
    pub endpoint_base: String,
    pub rpm_limit: u32,
}

impl Default for GeminiVlmPolicy {
    fn default() -> Self {
        Self {
            api_key_env: "GOOGLE_API_KEY".into(),
            model: "gemini-3-flash-preview".into(),
            endpoint_base: "https://generativelanguage.googleapis.com/v1beta".into(),
            rpm_limit: 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicVlmPolicy {
    pub model: String,
}

impl Default for AnthropicVlmPolicy {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: String,
}

impl PolicyDecision {
    pub fn allow(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
        }
    }
}
