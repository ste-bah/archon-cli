pub mod anthropic;
pub mod auth;
pub mod effort;
pub mod fast_mode;
pub mod identity;
pub mod oauth;
pub mod provider;
pub mod providers;
pub mod streaming;
pub mod thinking;
pub mod tokens;
pub mod types;

// TASK-AGS-700: crate-root re-exports for the Phase 7 descriptor scaffolding.
// Kept next to `pub mod providers;` so the surface stays discoverable.
pub use providers::{
    AuthFlavor, CompatKind, ProviderDescriptor, ProviderFeatures, ProviderQuirks,
};
