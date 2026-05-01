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

// GHOST-003: native registry (5 descriptors). The 4 stub providers
// (azure, cohere, copilot, minimax) were removed — they had no real
// wire implementations and returned LlmError::Unsupported.
pub mod native_registry;

// TASK-AGS-706: runtime dispatcher routing LlmConfig -> concrete provider.
pub mod builder;

// TASK-AGS-707: SSE + NDJSON line decoders used by `OpenAiCompatProvider::stream`.
pub(crate) mod stream_decode;

pub use anthropic::AnthropicProvider;
pub use bedrock::BedrockProvider;
pub use local::LocalProvider;
pub use openai::OpenAiProvider;
pub use vertex::VertexProvider;

pub use builder::{build_llm_provider, build_llm_provider_with_policy};
pub use descriptor::{AuthFlavor, CompatKind, ProviderDescriptor};
pub use error::ProviderError;
pub use features::ProviderFeatures;
pub use native_registry::{NATIVE_REGISTRY, count_native, get_native, list_native};
pub use openai_compat::OpenAiCompatProvider;
pub use quirks::{ProviderQuirks, StreamDelimiter, ToolCallFormat};
pub use registry::{OPENAI_COMPAT_REGISTRY, count as count_compat, get as get_compat, list_compat};
