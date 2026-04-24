//! Archon Pipeline — shared pipeline orchestration engine for coding and research pipelines.

pub mod agent_loader;
pub mod artefacts;
pub mod capture;
pub mod coding;
pub mod compression;
pub mod kb;
pub mod layered_context;
pub mod learning;
pub mod ledgers;
pub mod llm_adapter;
pub mod manifest;
pub mod memory;
pub mod prompt_cap;
pub mod rehydration;
pub mod research;
pub mod retry;
pub mod runner;
pub mod session;

// --- Phase 4: Declarative pipeline execution ---
pub mod error;
pub mod id;
pub mod run;
pub mod spec;

pub use id::PipelineId;
pub use run::{PipelineRun, PipelineState, StepRun, StepRunState};
pub use spec::{BackoffKind, OnFailurePolicy, PipelineFormat, PipelineSpec, RetrySpec, StepSpec};
pub mod parser;

pub mod backoff;
pub mod dag;
pub mod vars;

pub mod store;

pub mod condition;
pub mod engine;
pub mod executor;
pub mod rollback;

pub use backoff::delay as backoff_delay;
pub use condition::ConditionEvaluator;
pub use dag::{Dag, DagBuilder};
pub use engine::{DefaultPipelineEngine, PipelineEngine};
pub use error::PipelineError;
pub use executor::PipelineExecutor;
pub use parser::PipelineParser;
pub use rollback::{FailureAction, RollbackEngine};
pub use store::{AuditEvent, PipelineStateStore};
pub use vars::{VarRef, extract_refs, substitute, validate_refs};
