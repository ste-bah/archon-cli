/// Provider implementations for the `LlmProvider` trait.
pub mod anthropic;
pub mod aws_auth;
pub mod bedrock;
pub mod gcp_auth;
pub mod local;
pub mod openai;
pub mod vertex;

// TASK-AGS-700: descriptor-driven Phase 7 scaffolding. Coexists with the
// existing hand-written provider impls above; TASK-AGS-706+ will migrate
// call sites onto the registry-backed `build_llm_provider`.
pub mod descriptor;
pub mod features;
pub mod quirks;

// TASK-AGS-701: unified provider error enum used by every Phase 7
// descriptor-driven provider impl (702..706) and the retry layer (708).
pub mod error;

// TASK-AGS-702: static 31-entry OpenAI-compatible provider registry.
pub mod registry;

// TASK-AGS-703: parametric OpenAI-compatible provider impl backed by the
// registry (TASK-AGS-702) and credential wrapper (TASK-AGS-701).
pub mod openai_compat;

// TASK-AGS-704: native registry (9 descriptors) + stub impls for the 4
// gap-filler natives (azure, cohere, copilot, minimax).
pub mod native_registry;
pub mod native_gap;

// TASK-AGS-706: runtime dispatcher routing LlmConfig -> concrete provider.
pub mod builder;

// TASK-AGS-707: SSE + NDJSON line decoders used by `OpenAiCompatProvider::stream`.
pub(crate) mod stream_decode;

pub use anthropic::AnthropicProvider;
pub use bedrock::BedrockProvider;
pub use local::LocalProvider;
pub use openai::OpenAiProvider;
pub use vertex::VertexProvider;

pub use descriptor::{AuthFlavor, CompatKind, ProviderDescriptor};
pub use error::ProviderError;
pub use features::ProviderFeatures;
pub use openai_compat::OpenAiCompatProvider;
pub use quirks::{ProviderQuirks, StreamDelimiter, ToolCallFormat};
pub use registry::{
    count as count_compat, get as get_compat, list_compat, OPENAI_COMPAT_REGISTRY,
};
pub use native_gap::{AzureProvider, CohereProvider, CopilotProvider, MinimaxProvider};
pub use native_registry::{count_native, get_native, list_native, NATIVE_REGISTRY};
pub use builder::build_llm_provider;
