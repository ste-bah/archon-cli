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

pub use anthropic::AnthropicProvider;
pub use bedrock::BedrockProvider;
pub use local::LocalProvider;
pub use openai::OpenAiProvider;
pub use vertex::VertexProvider;

pub use descriptor::{AuthFlavor, CompatKind, ProviderDescriptor};
pub use features::ProviderFeatures;
pub use quirks::ProviderQuirks;
