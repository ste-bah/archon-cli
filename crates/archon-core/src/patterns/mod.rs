//! TASK-AGS-500: Pattern system foundation.
//!
//! Defines the `Pattern` trait, `PatternRegistry`, shared error/context
//! types, and declares sub-modules for each concrete pattern.

pub mod broker;
pub mod circuit_breaker;
pub mod composite;
pub mod fanout;
pub mod pipeline;
pub mod plugin;
pub mod remote;
pub mod spec;

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// Re-export spec types at the patterns:: level.
pub use spec::*;

// Re-export concrete pattern types.
pub use broker::{AgentRegistryHandle, BrokerPattern, Candidate, CustomSelectorFn};
pub use circuit_breaker::{BreakerState, CircuitBreaker, wrap_with_breaker};
pub use composite::{
    CompiledComposite, CompositeAgentPattern, CompositeConfig, CompositeEdge, CompositeNode,
};
pub use fanout::FanOutFanInPattern;
pub use pipeline::{PipelineAdapterConfig, PipelineEngineHandle, PipelinePattern};
pub use plugin::{NativePluginDescriptor, PatternPluginLoader, WasmPattern, WasmPluginConfig};
pub use remote::{DiscoveryResolver, RemoteAgentPattern, StaticResolver};

// ---------------------------------------------------------------------------
// PatternKind
// ---------------------------------------------------------------------------

/// Identifies which pattern a `PatternSpec` targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PatternKind {
    Pipeline,
    FanOut,
    Broker,
    Composite,
    Remote,
    Custom(String),
}

// ---------------------------------------------------------------------------
// PatternError
// ---------------------------------------------------------------------------

/// Unified error type for all pattern operations.
///
/// Maps to PRD error states ERR-PAT-01..03 and EC-ARCH-001..006.
#[derive(thiserror::Error, Debug)]
pub enum PatternError {
    #[error("pattern execution timed out")]
    Timeout,

    #[error("circuit breaker open for agent '{name}'")]
    CircuitOpen { name: String },

    #[error("remote agent '{url}' unreachable: {cause}")]
    RemoteUnreachable { url: String, cause: String },

    #[error("composite agent cycle detected: {path:?}")]
    CompositeCycle { path: Vec<String> },

    #[error("broker found no suitable candidate: {reasons:?}")]
    BrokerNoCandidate { reasons: Vec<String> },

    #[error("partial result (some workers failed): {errors:?}")]
    PartialResult { merged: Value, errors: Vec<String> },

    #[error("pattern execution error: {0}")]
    Execution(String),
}

// ---------------------------------------------------------------------------
// TaskServiceHandle — slim trait to avoid circular deps
// ---------------------------------------------------------------------------

/// Slim abstraction over the full TaskService (phase 2).
///
/// Concrete patterns depend on this; the real TaskService from
/// `archon_core::tasks` supplies the impl at composition time.
#[async_trait]
pub trait TaskServiceHandle: Send + Sync {
    async fn submit(&self, agent: &str, input: Value) -> Result<Value, PatternError>;
}

// ---------------------------------------------------------------------------
// PatternCtx — execution context passed to every pattern
// ---------------------------------------------------------------------------

/// Execution context threaded through every `Pattern::execute` call.
pub struct PatternCtx {
    /// Handle for dispatching agent work.
    pub task_service: Arc<dyn TaskServiceHandle>,
    /// Registry for resolving nested patterns.
    pub registry: Arc<PatternRegistry>,
    /// Distributed trace ID for correlation.
    pub trace_id: String,
    /// Optional hard deadline for the entire operation.
    pub deadline: Option<Instant>,
}

// ---------------------------------------------------------------------------
// Pattern trait
// ---------------------------------------------------------------------------

/// Core trait that all execution patterns implement.
///
/// Patterns are composable and nestable (INT-ARCH-PATTERN-01 key behavior #2).
#[async_trait]
pub trait Pattern: Send + Sync {
    /// Which kind of pattern this is.
    fn kind(&self) -> PatternKind;

    /// Execute the pattern with the given input and context.
    async fn execute(&self, input: Value, ctx: PatternCtx) -> Result<Value, PatternError>;
}

// ---------------------------------------------------------------------------
// PatternRegistry
// ---------------------------------------------------------------------------

/// Thread-safe registry for named pattern instances.
///
/// Patterns are registered at startup and resolved by name at runtime.
pub struct PatternRegistry {
    inner: DashMap<String, Arc<dyn Pattern>>,
}

impl PatternRegistry {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Register a pattern under the given name.
    pub fn register(&self, name: &str, p: Arc<dyn Pattern>) {
        self.inner.insert(name.to_owned(), p);
    }

    /// Resolve a pattern by name.
    pub fn resolve(&self, name: &str) -> Option<Arc<dyn Pattern>> {
        self.inner.get(name).map(|r| Arc::clone(r.value()))
    }

    /// List all registered pattern names.
    pub fn list_names(&self) -> Vec<String> {
        self.inner.iter().map(|r| r.key().clone()).collect()
    }
}

impl Default for PatternRegistry {
    fn default() -> Self {
        Self::new()
    }
}
