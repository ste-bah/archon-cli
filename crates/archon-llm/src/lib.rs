// TASK-AGS-709: ActiveProvider — ArcSwap-backed live-swappable handle.
pub mod active;
pub mod anthropic;
pub mod auth;
// TASK-AGS-706: LlmConfig + resolve_descriptor — feeds build_llm_provider.
pub mod config;
pub(crate) mod debug_body;
pub mod effort;
pub mod fast_mode;
pub mod identity;
// TASK-P0-B.1a (#178): Multi-modal content helpers (image, future PDF/audio).
pub mod multimodal;
pub mod oauth;
pub mod provider;
pub mod providers;
// TASK-AGS-708: RetryProvider<P> decorator with exponential backoff.
pub mod retry;
pub mod secrets;
pub mod streaming;
pub mod thinking;
pub mod tokens;
pub mod types;

// TASK-AGS-706: re-export LlmConfig at crate root for call sites that
// don't want to reach into `config::`.
pub use config::LlmConfig;

// TASK-AGS-700: crate-root re-exports for the Phase 7 descriptor scaffolding.
// Kept next to `pub mod providers;` so the surface stays discoverable.
pub use providers::{AuthFlavor, CompatKind, ProviderDescriptor, ProviderFeatures, ProviderQuirks};

// TASK-AGS-701: re-export ApiKey at crate root so every Phase 7 provider
// impl can `use archon_llm::ApiKey` without reaching into `secrets::`.
pub use secrets::ApiKey;

// TASK-AGS-708: surface RetryProvider/RetryPolicy at the crate root so
// `build_llm_provider` consumers don't need to reach into `retry::`.
pub use retry::{RetryDecision, RetryPolicy, RetryProvider, classify as classify_retry};

// TASK-AGS-709: surface ActiveProvider so all new call sites (and
// TASK-AGS-710's archon-cli main.rs rework) can `use archon_llm::ActiveProvider`.
pub use active::ActiveProvider;

// TASK-TUI-702: re-export TokenUsage at crate root for SessionStats struct.
pub use types::TokenUsage;
