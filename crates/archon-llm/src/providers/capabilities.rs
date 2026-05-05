//! Archon-level provider capability matrix.
//!
//! This is intentionally higher-level than `ProviderFeatures`. A provider can
//! have wire support for tool calls while a given Archon surface is not yet
//! wired and verified against that provider. User-facing docs and diagnostics
//! should use this matrix so "supported" means callable in Archon.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    OneShotChat,
    InteractiveSession,
    Streaming,
    ToolUse,
    Subagents,
    PipelineCoding,
    PipelineResearch,
    PipelineGametheory,
    BtwSideQuestion,
    Vision,
    Embeddings,
    CostMetadata,
}

impl ProviderCapability {
    pub const ALL: &'static [ProviderCapability] = &[
        ProviderCapability::OneShotChat,
        ProviderCapability::InteractiveSession,
        ProviderCapability::Streaming,
        ProviderCapability::ToolUse,
        ProviderCapability::Subagents,
        ProviderCapability::PipelineCoding,
        ProviderCapability::PipelineResearch,
        ProviderCapability::PipelineGametheory,
        ProviderCapability::BtwSideQuestion,
        ProviderCapability::Vision,
        ProviderCapability::Embeddings,
        ProviderCapability::CostMetadata,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            ProviderCapability::OneShotChat => "one_shot_chat",
            ProviderCapability::InteractiveSession => "interactive_session",
            ProviderCapability::Streaming => "streaming",
            ProviderCapability::ToolUse => "tool_use",
            ProviderCapability::Subagents => "subagents",
            ProviderCapability::PipelineCoding => "pipeline_coding",
            ProviderCapability::PipelineResearch => "pipeline_research",
            ProviderCapability::PipelineGametheory => "pipeline_gametheory",
            ProviderCapability::BtwSideQuestion => "btw_side_question",
            ProviderCapability::Vision => "vision",
            ProviderCapability::Embeddings => "embeddings",
            ProviderCapability::CostMetadata => "cost_metadata",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            ProviderCapability::OneShotChat => "one-shot chat",
            ProviderCapability::InteractiveSession => "interactive TUI",
            ProviderCapability::Streaming => "streaming",
            ProviderCapability::ToolUse => "agent tool use",
            ProviderCapability::Subagents => "subagents",
            ProviderCapability::PipelineCoding => "coding pipeline",
            ProviderCapability::PipelineResearch => "research pipeline",
            ProviderCapability::PipelineGametheory => "gametheory pipeline",
            ProviderCapability::BtwSideQuestion => "/btw",
            ProviderCapability::Vision => "vision",
            ProviderCapability::Embeddings => "embeddings",
            ProviderCapability::CostMetadata => "cost metadata",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Supported,
    Unsupported,
}

impl CapabilityStatus {
    pub const fn marker(self) -> &'static str {
        match self {
            CapabilityStatus::Supported => "yes",
            CapabilityStatus::Unsupported => "no",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilityRow {
    pub provider_id: &'static str,
    pub display_name: &'static str,
    pub auth_mode: &'static str,
    pub supported: &'static [ProviderCapability],
    pub notes: &'static str,
}

impl ProviderCapabilityRow {
    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.supported.contains(&capability)
    }

    pub fn status(&self, capability: ProviderCapability) -> CapabilityStatus {
        if self.supports(capability) {
            CapabilityStatus::Supported
        } else {
            CapabilityStatus::Unsupported
        }
    }
}

const ANTHROPIC_SURFACES: &[ProviderCapability] = &[
    ProviderCapability::OneShotChat,
    ProviderCapability::InteractiveSession,
    ProviderCapability::Streaming,
    ProviderCapability::ToolUse,
    ProviderCapability::Subagents,
    ProviderCapability::PipelineCoding,
    ProviderCapability::PipelineResearch,
    ProviderCapability::PipelineGametheory,
    ProviderCapability::BtwSideQuestion,
    ProviderCapability::Vision,
    ProviderCapability::CostMetadata,
];

const ANTHROPIC_PROXY_SURFACES: &[ProviderCapability] = &[
    ProviderCapability::OneShotChat,
    ProviderCapability::InteractiveSession,
    ProviderCapability::Streaming,
    ProviderCapability::ToolUse,
    ProviderCapability::Subagents,
    ProviderCapability::PipelineCoding,
    ProviderCapability::PipelineResearch,
    ProviderCapability::PipelineGametheory,
    ProviderCapability::Vision,
    ProviderCapability::CostMetadata,
];

const CODEX_SURFACES: &[ProviderCapability] = &[
    ProviderCapability::OneShotChat,
    ProviderCapability::InteractiveSession,
    ProviderCapability::Streaming,
    ProviderCapability::ToolUse,
    ProviderCapability::Subagents,
    ProviderCapability::PipelineCoding,
    ProviderCapability::PipelineResearch,
    ProviderCapability::PipelineGametheory,
    ProviderCapability::BtwSideQuestion,
    ProviderCapability::Vision,
];

pub const PROVIDER_CAPABILITY_ROWS: &[ProviderCapabilityRow] = &[
    ProviderCapabilityRow {
        provider_id: "anthropic-oauth",
        display_name: "Anthropic OAuth spoof",
        auth_mode: "Claude/Anthropic OAuth",
        supported: ANTHROPIC_SURFACES,
        notes: "Primary path for agents, subagents, pipelines and /btw.",
    },
    ProviderCapabilityRow {
        provider_id: "anthropic-api-key",
        display_name: "Anthropic API key",
        auth_mode: "ANTHROPIC_API_KEY",
        supported: ANTHROPIC_SURFACES,
        notes: "Native Anthropic Messages API path.",
    },
    ProviderCapabilityRow {
        provider_id: "anthropic-compatible-proxy",
        display_name: "Anthropic-compatible proxy",
        auth_mode: "ANTHROPIC_BASE_URL + API key",
        supported: ANTHROPIC_PROXY_SURFACES,
        notes: "Depends on proxy fidelity; /btw OAuth-only behavior is not assumed.",
    },
    ProviderCapabilityRow {
        provider_id: "openai-codex",
        display_name: "OpenAI Codex OAuth",
        auth_mode: "ChatGPT/Codex OAuth",
        supported: CODEX_SURFACES,
        notes: "Backs one-shot chat, full TUI sessions, tool use, subagents, /btw, and provider-neutral pipelines; exact cost metadata remains unavailable when the backend omits usage pricing.",
    },
];

pub fn provider_capabilities() -> &'static [ProviderCapabilityRow] {
    PROVIDER_CAPABILITY_ROWS
}

pub fn capabilities_for(provider_id: &str) -> Option<&'static ProviderCapabilityRow> {
    PROVIDER_CAPABILITY_ROWS
        .iter()
        .find(|row| row.provider_id == provider_id)
}

pub fn supports(provider_id: &str, capability: ProviderCapability) -> bool {
    capabilities_for(provider_id)
        .map(|row| row.supports(capability))
        .unwrap_or(false)
}

pub fn render_capability_table() -> String {
    let mut out = String::new();
    out.push_str("Archon provider capability matrix\n\n");
    out.push_str("| Provider | Auth mode | one-shot | TUI | stream | tools | subagents | code | research | gametheory | /btw | vision | embed | cost | Notes |\n");
    out.push_str("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|\n");
    for row in provider_capabilities() {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            row.provider_id,
            row.auth_mode,
            row.status(ProviderCapability::OneShotChat).marker(),
            row.status(ProviderCapability::InteractiveSession).marker(),
            row.status(ProviderCapability::Streaming).marker(),
            row.status(ProviderCapability::ToolUse).marker(),
            row.status(ProviderCapability::Subagents).marker(),
            row.status(ProviderCapability::PipelineCoding).marker(),
            row.status(ProviderCapability::PipelineResearch).marker(),
            row.status(ProviderCapability::PipelineGametheory).marker(),
            row.status(ProviderCapability::BtwSideQuestion).marker(),
            row.status(ProviderCapability::Vision).marker(),
            row.status(ProviderCapability::Embeddings).marker(),
            row.status(ProviderCapability::CostMetadata).marker(),
            row.notes,
        ));
    }
    out
}

pub fn render_capability_markdown() -> String {
    let mut out = String::new();
    out.push_str("# Provider capabilities\n\n");
    out.push_str("Generated from `archon_llm::providers::capabilities`. This matrix is Archon surface support, not only raw model wire features.\n\n");
    out.push_str(&render_capability_table());
    out.push_str("\n## Capability keys\n\n");
    for capability in ProviderCapability::ALL {
        out.push_str(&format!(
            "- `{}` - {}\n",
            capability.key(),
            capability.label()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_supports_agentic_surfaces_after_provider_parity() {
        let row = capabilities_for("openai-codex").expect("codex row");
        assert!(row.supports(ProviderCapability::OneShotChat));
        assert!(row.supports(ProviderCapability::InteractiveSession));
        assert!(row.supports(ProviderCapability::ToolUse));
        assert!(row.supports(ProviderCapability::Subagents));
        assert!(row.supports(ProviderCapability::PipelineCoding));
        assert!(row.supports(ProviderCapability::PipelineResearch));
        assert!(row.supports(ProviderCapability::PipelineGametheory));
        assert!(row.supports(ProviderCapability::BtwSideQuestion));
        assert!(!row.supports(ProviderCapability::CostMetadata));
    }

    #[test]
    fn anthropic_oauth_supports_agentic_surfaces() {
        let row = capabilities_for("anthropic-oauth").expect("anthropic oauth row");
        assert!(row.supports(ProviderCapability::ToolUse));
        assert!(row.supports(ProviderCapability::Subagents));
        assert!(row.supports(ProviderCapability::PipelineCoding));
        assert!(row.supports(ProviderCapability::PipelineResearch));
        assert!(row.supports(ProviderCapability::PipelineGametheory));
        assert!(row.supports(ProviderCapability::BtwSideQuestion));
    }

    #[test]
    fn markdown_contains_status_vocabulary_without_unicode_markers() {
        let markdown = render_capability_markdown();
        assert!(markdown.contains("`openai-codex`"));
        assert!(markdown.contains("| `openai-codex` |"));
        assert!(markdown.contains("| `anthropic-oauth` |"));
        assert!(markdown.contains("yes"));
        assert!(markdown.contains("no"));
    }
}
