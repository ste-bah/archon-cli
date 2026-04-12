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
pub mod id;
pub mod spec;
pub mod run;
pub mod error;

pub use id::PipelineId;
pub use spec::{PipelineSpec, StepSpec, RetrySpec, BackoffKind, OnFailurePolicy, PipelineFormat};
pub use run::{PipelineRun, PipelineState, StepRun, StepRunState};
pub mod parser;

pub mod dag;
pub mod vars;
pub mod backoff;

pub mod store;

pub mod condition;
pub mod executor;
pub mod rollback;
pub mod engine;

pub use dag::{Dag, DagBuilder};
pub use error::PipelineError;
pub use executor::PipelineExecutor;
pub use parser::PipelineParser;
pub use rollback::{RollbackEngine, FailureAction};
pub use store::{PipelineStateStore, AuditEvent};
pub use backoff::delay as backoff_delay;
pub use condition::ConditionEvaluator;
pub use vars::{VarRef, extract_refs, validate_refs, substitute};
pub use engine::{PipelineEngine, DefaultPipelineEngine};
