//! TASK-AGS-702: static registry of the 31 OpenAI-compatible providers
//! required by REQ-FOR-D6.
//!
//! Each entry is a `ProviderDescriptor` literal — *data*, not code.
//! `OpenAiCompatProvider` (TASK-AGS-703) pairs any one descriptor with
//! a `SecretString` + `reqwest::Client` at runtime; adding a new
//! OpenAI-compatible provider means appending one entry here, not
//! writing a new `impl LlmProvider`.
//!
//! Quirk values (per-provider header/tool-format overrides) are
//! populated by TASK-AGS-705. This task leaves quirks unset; only
//! the base routing data (URL, env var name, default model) is
//! populated.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use url::Url;

use super::descriptor::{AuthFlavor, CompatKind, ProviderDescriptor};
use super::features::ProviderFeatures;

fn parse_url(s: &str) -> Url {
    Url::parse(s).expect("TASK-AGS-702 registry url literal must parse")
}

/// 31-entry OpenAI-compatible provider registry. Built once at first
/// access via `once_cell::sync::Lazy`. Keyed by the lowercase provider
/// slug used in config files (`provider = "groq"`).
pub static OPENAI_COMPAT_REGISTRY: Lazy<HashMap<&'static str, ProviderDescriptor>> =
    Lazy::new(|| {
        let mut m: HashMap<&'static str, ProviderDescriptor> = HashMap::new();

        // -- Local providers (no auth) ----------------------------------

        m.insert(
            "ollama",
            ProviderDescriptor {
                id: "ollama".into(),
                display_name: "Ollama".into(),
                base_url: parse_url("http://localhost:11434/v1"),
                auth_flavor: AuthFlavor::None,
                env_key_var: String::new(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "llama3.2".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "lm_studio",
            ProviderDescriptor {
                id: "lm_studio".into(),
                display_name: "LM Studio".into(),
                base_url: parse_url("http://localhost:1234/v1"),
                auth_flavor: AuthFlavor::None,
                env_key_var: String::new(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "local-model".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "llama_cpp",
            ProviderDescriptor {
                id: "llama_cpp".into(),
                display_name: "llama.cpp".into(),
                base_url: parse_url("http://localhost:8080/v1"),
                auth_flavor: AuthFlavor::None,
                env_key_var: String::new(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "local-model".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        // -- Remote providers (Bearer) ----------------------------------

        m.insert(
            "deepseek",
            ProviderDescriptor {
                id: "deepseek".into(),
                display_name: "DeepSeek".into(),
                base_url: parse_url("https://api.deepseek.com/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "DEEPSEEK_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "deepseek-chat".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "groq",
            ProviderDescriptor {
                id: "groq".into(),
                display_name: "Groq".into(),
                base_url: parse_url("https://api.groq.com/openai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "GROQ_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "llama-3.3-70b-versatile".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "xai",
            ProviderDescriptor {
                id: "xai".into(),
                display_name: "xAI".into(),
                base_url: parse_url("https://api.x.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "XAI_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "grok-2".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "deepinfra",
            ProviderDescriptor {
                id: "deepinfra".into(),
                display_name: "DeepInfra".into(),
                base_url: parse_url("https://api.deepinfra.com/v1/openai"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "DEEPINFRA_API_TOKEN".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama/Llama-3.3-70B-Instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "cerebras",
            ProviderDescriptor {
                id: "cerebras".into(),
                display_name: "Cerebras".into(),
                base_url: parse_url("https://api.cerebras.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "CEREBRAS_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "llama3.1-70b".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "together_ai",
            ProviderDescriptor {
                id: "together_ai".into(),
                display_name: "Together AI".into(),
                base_url: parse_url("https://api.together.xyz/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "TOGETHER_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "perplexity",
            ProviderDescriptor {
                id: "perplexity".into(),
                display_name: "Perplexity".into(),
                base_url: parse_url("https://api.perplexity.ai"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "PERPLEXITY_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "llama-3.1-sonar-large-128k-online".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "openrouter",
            ProviderDescriptor {
                id: "openrouter".into(),
                display_name: "OpenRouter".into(),
                base_url: parse_url("https://openrouter.ai/api/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "OPENROUTER_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "anthropic/claude-3.5-sonnet".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "mistral",
            ProviderDescriptor {
                id: "mistral".into(),
                display_name: "Mistral".into(),
                base_url: parse_url("https://api.mistral.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "MISTRAL_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "mistral-large-latest".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "sambanova",
            ProviderDescriptor {
                id: "sambanova".into(),
                display_name: "SambaNova".into(),
                base_url: parse_url("https://api.sambanova.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "SAMBANOVA_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "Meta-Llama-3.3-70B-Instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "huggingface",
            ProviderDescriptor {
                id: "huggingface".into(),
                display_name: "Hugging Face".into(),
                base_url: parse_url("https://api-inference.huggingface.co/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "HUGGINGFACE_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama/Llama-3.3-70B-Instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "nvidia",
            ProviderDescriptor {
                id: "nvidia".into(),
                display_name: "NVIDIA NIM".into(),
                base_url: parse_url("https://integrate.api.nvidia.com/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "NVIDIA_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta/llama-3.3-70b-instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "siliconflow",
            ProviderDescriptor {
                id: "siliconflow".into(),
                display_name: "SiliconFlow".into(),
                base_url: parse_url("https://api.siliconflow.com/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "SILICONFLOW_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "deepseek-ai/DeepSeek-V3".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "moonshot",
            ProviderDescriptor {
                id: "moonshot".into(),
                display_name: "Moonshot".into(),
                base_url: parse_url("https://api.moonshot.cn/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "MOONSHOT_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "moonshot-v1-8k".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "zhipu",
            ProviderDescriptor {
                id: "zhipu".into(),
                display_name: "Zhipu AI".into(),
                base_url: parse_url("https://open.bigmodel.cn/api/paas/v4"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "ZHIPU_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "glm-4".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "zai",
            ProviderDescriptor {
                id: "zai".into(),
                display_name: "Z.AI".into(),
                base_url: parse_url("https://api.z.ai/api/paas/v4"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "ZAI_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "glm-4.6".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "nebius",
            ProviderDescriptor {
                id: "nebius".into(),
                display_name: "Nebius".into(),
                base_url: parse_url("https://api.studio.nebius.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "NEBIUS_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama/Llama-3.3-70B-Instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "novita",
            ProviderDescriptor {
                id: "novita".into(),
                display_name: "Novita".into(),
                base_url: parse_url("https://api.novita.ai/v3/openai"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "NOVITA_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama/llama-3.3-70b-instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "ovhcloud",
            ProviderDescriptor {
                id: "ovhcloud".into(),
                display_name: "OVHcloud".into(),
                base_url: parse_url(
                    "https://gra.endpoints.kepler.ai.cloud.ovh.net/oai/api/openai_compat/v1",
                ),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "OVHCLOUD_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "Meta-Llama-3_3-70B-Instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "scaleway",
            ProviderDescriptor {
                id: "scaleway".into(),
                display_name: "Scaleway".into(),
                base_url: parse_url("https://api.scaleway.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "SCALEWAY_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "llama-3.3-70b-instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "vultr",
            ProviderDescriptor {
                id: "vultr".into(),
                display_name: "Vultr Inference".into(),
                base_url: parse_url("https://api.vultrinference.com/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "VULTR_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "llama-3.3-70b-instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "baseten",
            ProviderDescriptor {
                id: "baseten".into(),
                display_name: "Baseten".into(),
                base_url: parse_url("https://inference.baseten.co/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "BASETEN_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama/Llama-3-70b-chat-hf".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "friendli",
            ProviderDescriptor {
                id: "friendli".into(),
                display_name: "FriendliAI".into(),
                base_url: parse_url("https://inference.friendli.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "FRIENDLI_TOKEN".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "meta-llama-3.1-70b-instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "upstage",
            ProviderDescriptor {
                id: "upstage".into(),
                display_name: "Upstage".into(),
                base_url: parse_url("https://api.upstage.ai/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "UPSTAGE_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "solar-pro".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "stepfun",
            ProviderDescriptor {
                id: "stepfun".into(),
                display_name: "StepFun".into(),
                base_url: parse_url("https://api.stepfun.com/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "STEPFUN_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "step-2-16k".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "fireworks",
            ProviderDescriptor {
                id: "fireworks".into(),
                display_name: "Fireworks AI".into(),
                base_url: parse_url("https://api.fireworks.ai/inference/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "FIREWORKS_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "accounts/fireworks/models/llama-v3p3-70b-instruct".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "qwen",
            ProviderDescriptor {
                id: "qwen".into(),
                display_name: "Qwen (DashScope)".into(),
                base_url: parse_url("https://dashscope.aliyuncs.com/compatible-mode/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "DASHSCOPE_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "qwen-max".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        m.insert(
            "venice",
            ProviderDescriptor {
                id: "venice".into(),
                display_name: "Venice AI".into(),
                base_url: parse_url("https://api.venice.ai/api/v1"),
                auth_flavor: AuthFlavor::BearerApiKey,
                env_key_var: "VENICE_API_KEY".into(),
                compat_kind: CompatKind::OpenAiCompat,
                default_model: "venice-uncensored".into(),
                supports: ProviderFeatures::chat_only(),
                headers: HashMap::new(),
            },
        );

        debug_assert_eq!(m.len(), 31, "TASK-AGS-702: registry must have 31 entries");
        m
    });

/// All registry entries in unspecified order. Intended for iteration in
/// diagnostic commands (`archon providers list`) and validation tests.
pub fn list_compat() -> Vec<&'static ProviderDescriptor> {
    OPENAI_COMPAT_REGISTRY.values().collect()
}

/// Look up one descriptor by its slug.
pub fn get(id: &str) -> Option<&'static ProviderDescriptor> {
    OPENAI_COMPAT_REGISTRY.get(id)
}

/// Registry size. Equivalent to `OPENAI_COMPAT_REGISTRY.len()` but usable
/// in `const`-ish contexts where `Lazy::force` is awkward.
pub fn count() -> usize {
    OPENAI_COMPAT_REGISTRY.len()
}
