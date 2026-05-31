use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddingProviderSelection {
    Auto,
    Local,
    OpenAiCompatible,
    Disabled,
}

#[derive(Clone, Debug)]
pub struct EmbeddingProviderConfig {
    pub selection: EmbeddingProviderSelection,
    pub local_load_timeout: Duration,
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: Option<String>,
    pub openai_timeout: Duration,
}

impl EmbeddingProviderConfig {
    pub fn from_env() -> Self {
        let selection = match std::env::var("ARCHON_DOCS_EMBEDDING_PROVIDER")
            .unwrap_or_else(|_| "auto".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "disabled" | "none" | "off" => EmbeddingProviderSelection::Disabled,
            "local" | "fastembed" => EmbeddingProviderSelection::Local,
            "openai" | "openai-compatible" | "openai_compat" => {
                EmbeddingProviderSelection::OpenAiCompatible
            }
            _ => EmbeddingProviderSelection::Auto,
        };
        Self {
            selection,
            local_load_timeout: default_load_timeout(),
            openai_api_key: docs_openai_key(),
            openai_base_url: env_nonempty("ARCHON_DOCS_EMBEDDING_BASE_URL")
                .or_else(|| env_nonempty("OPENAI_BASE_URL")),
            openai_model: env_nonempty("ARCHON_DOCS_EMBEDDING_MODEL"),
            openai_timeout: Duration::from_secs(env_u64("ARCHON_DOCS_EMBEDDING_TIMEOUT_SECS", 60)),
        }
    }
}

pub(crate) fn default_load_timeout() -> Duration {
    Duration::from_secs(env_u64("ARCHON_DOCS_EMBEDDING_LOAD_TIMEOUT_SECS", 180))
}

fn docs_openai_key() -> Option<String> {
    env_nonempty("ARCHON_DOCS_OPENAIKEY")
        .or_else(|| env_nonempty("ARCHON_MEMORY_OPENAIKEY"))
        .or_else(|| env_nonempty("OPENAI_API_KEY"))
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
