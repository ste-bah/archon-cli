//! WASM plugin host for Archon (TASK-CLI-301).
//!
//! Provides a sandboxed execution environment for `.wasm` plugins using wasmtime,
//! with capability-based security, host-guest ABI, and resource limits.

pub mod abi;
pub mod adapter_tool;
pub mod api;
pub mod cache;
pub mod capability;
pub mod error;
pub mod host;
pub mod instance;
pub mod loader;
pub mod manifest;
pub mod result;
pub mod types;

pub use api::tools_from_plugin_instance;
pub use capability::{CapabilityChecker, PluginCapability};
pub use error::PluginError;
pub use host::{PluginHostConfig, WasmPluginHost};
pub use instance::{PluginInstance, RegisteredTool};
pub use loader::instantiate_wasm_plugins;
pub use types::{PluginConfig, PluginManifest, PluginMetadata};
