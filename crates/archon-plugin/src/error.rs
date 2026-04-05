//! Plugin error types for TASK-CLI-301.

use std::path::PathBuf;

use crate::capability::PluginCapability;

/// All errors from the WASM plugin host.
///
/// Each variant carries contextual information to aid debugging.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The WASM module could not be compiled or instantiated.
    #[error("plugin load failed: {0}")]
    LoadFailed(String),

    /// The guest's declared API version is incompatible with the host.
    #[error("ABI mismatch: host supports v{expected}, guest reports v{got}")]
    AbiMismatch { expected: u32, got: u32 },

    /// A host function call was blocked because a required capability is missing.
    #[error("capability denied: {0:?}")]
    CapabilityDenied(PluginCapability),

    /// Plugin execution exceeded its time/fuel budget.
    #[error("plugin timed out (fuel_exhausted={fuel_exhausted})")]
    Timeout { fuel_exhausted: bool },

    /// Plugin attempted to grow memory beyond its configured limit.
    #[error("memory violation: requested {requested} bytes, limit {limit} bytes")]
    MemoryViolation { requested: usize, limit: usize },

    /// The plugin manifest JSON could not be parsed.
    #[error("manifest parse error at {path}: {reason}")]
    ManifestParseError { path: PathBuf, reason: String },

    /// The plugin manifest is syntactically valid but semantically invalid.
    #[error("manifest validation error at {path}: missing or invalid fields: {fields:?}")]
    ManifestValidationError { path: PathBuf, fields: Vec<String> },

    /// A required plugin dependency is not loaded.
    #[error("plugin '{plugin}' requires '{dependency}' which is not loaded")]
    DependencyUnsatisfied { plugin: String, dependency: String },

    /// The WASM component (or module) could not be linked.
    #[error("component load failed at {path}: {reason}")]
    ComponentLoadFailed { path: PathBuf, reason: String },
}
