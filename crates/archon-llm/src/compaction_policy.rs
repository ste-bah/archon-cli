//! Provider-family compaction policy matrix.
//!
//! Compaction is applied to Archon's provider-agnostic conversation state, but
//! provider families still differ in wire shape, invariant repair, and whether
//! a native compaction API exists. Keep this matrix explicit so adding a new
//! provider family requires choosing a compaction backend deliberately.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderFamily {
    AnthropicApi,
    AnthropicOAuth,
    Bedrock,
    Vertex,
    OpenAiNative,
    OpenAiCompatible,
    CodexOAuth,
    CodexAppServer,
    Local,
}

impl ProviderFamily {
    pub const ALL: &'static [ProviderFamily] = &[
        ProviderFamily::AnthropicApi,
        ProviderFamily::AnthropicOAuth,
        ProviderFamily::Bedrock,
        ProviderFamily::Vertex,
        ProviderFamily::OpenAiNative,
        ProviderFamily::OpenAiCompatible,
        ProviderFamily::CodexOAuth,
        ProviderFamily::CodexAppServer,
        ProviderFamily::Local,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            ProviderFamily::AnthropicApi => "anthropic_api",
            ProviderFamily::AnthropicOAuth => "anthropic_oauth",
            ProviderFamily::Bedrock => "bedrock",
            ProviderFamily::Vertex => "vertex",
            ProviderFamily::OpenAiNative => "openai_native",
            ProviderFamily::OpenAiCompatible => "openai_compatible",
            ProviderFamily::CodexOAuth => "codex_oauth",
            ProviderFamily::CodexAppServer => "codex_app_server",
            ProviderFamily::Local => "local",
        }
    }

    pub fn from_provider_id(provider_id: &str) -> Self {
        match provider_id.trim().to_ascii_lowercase().as_str() {
            "anthropic" => ProviderFamily::AnthropicApi,
            "bedrock" => ProviderFamily::Bedrock,
            "vertex" => ProviderFamily::Vertex,
            "openai" => ProviderFamily::OpenAiNative,
            "openai-codex" => ProviderFamily::CodexOAuth,
            "local" => ProviderFamily::Local,
            _ => ProviderFamily::OpenAiCompatible,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireShape {
    AnthropicMessages,
    BedrockConverse,
    VertexAnthropic,
    OpenAiChatCompletions,
    OpenAiResponses,
    CodexAppServerRpc,
}

impl WireShape {
    pub const fn label(self) -> &'static str {
        match self {
            WireShape::AnthropicMessages => "anthropic_messages",
            WireShape::BedrockConverse => "bedrock_converse",
            WireShape::VertexAnthropic => "vertex_anthropic",
            WireShape::OpenAiChatCompletions => "openai_chat_completions",
            WireShape::OpenAiResponses => "openai_responses",
            WireShape::CodexAppServerRpc => "codex_app_server_rpc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionBackend {
    Generic,
    Anthropic,
    CodexNative,
    Unsupported,
}

impl CompactionBackend {
    pub const fn label(self) -> &'static str {
        match self {
            CompactionBackend::Generic => "generic",
            CompactionBackend::Anthropic => "anthropic",
            CompactionBackend::CodexNative => "codex_native",
            CompactionBackend::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPolicy {
    pub provider_family: ProviderFamily,
    pub wire_shape: WireShape,
    pub backend: CompactionBackend,
    pub generic_fallback: bool,
}

pub const COMPACTION_POLICIES: &[CompactionPolicy] = &[
    CompactionPolicy {
        provider_family: ProviderFamily::AnthropicApi,
        wire_shape: WireShape::AnthropicMessages,
        backend: CompactionBackend::Anthropic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::AnthropicOAuth,
        wire_shape: WireShape::AnthropicMessages,
        backend: CompactionBackend::Anthropic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::Bedrock,
        wire_shape: WireShape::BedrockConverse,
        backend: CompactionBackend::Anthropic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::Vertex,
        wire_shape: WireShape::VertexAnthropic,
        backend: CompactionBackend::Anthropic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::OpenAiNative,
        wire_shape: WireShape::OpenAiChatCompletions,
        backend: CompactionBackend::Generic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::OpenAiCompatible,
        wire_shape: WireShape::OpenAiChatCompletions,
        backend: CompactionBackend::Generic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::CodexOAuth,
        wire_shape: WireShape::OpenAiResponses,
        backend: CompactionBackend::Generic,
        generic_fallback: false,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::CodexAppServer,
        wire_shape: WireShape::CodexAppServerRpc,
        backend: CompactionBackend::Unsupported,
        generic_fallback: true,
    },
    CompactionPolicy {
        provider_family: ProviderFamily::Local,
        wire_shape: WireShape::OpenAiChatCompletions,
        backend: CompactionBackend::Generic,
        generic_fallback: false,
    },
];

pub fn compaction_policy_for_family(family: ProviderFamily) -> CompactionPolicy {
    COMPACTION_POLICIES
        .iter()
        .copied()
        .find(|policy| policy.provider_family == family)
        .expect("every ProviderFamily must have an explicit compaction policy")
}
