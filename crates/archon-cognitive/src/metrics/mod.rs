//! R8 measurement foundation: versioned cognitive metrics derived from an
//! append-only event log.
//!
//! Deliberately out of scope here (tracked separately): baseline/canary
//! comparison with confidence intervals and automatic rollback. Windows
//! already carry a [`window::CohortRole`] so that work can attach without
//! rewriting anything written today.

mod codec;
pub mod definitions;
pub mod derive;
pub mod emit;
pub mod event;
pub mod event_store;
pub mod window;
mod window_store;

pub use definitions::{
    METRIC_DEFINITION_VERSION, MetricAggregation, MetricDefinition, metric_definitions,
};
pub use derive::{CognitiveMetricSnapshot, DerivedMetric, by_cohort, derive_snapshot};
pub use emit::{MetricEmitter, UNWINDOWED_EVALUATION_WINDOW, policy_version, runtime_cohort};
pub use event::{CognitiveMetricEvent, METRIC_EVENT_SCHEMA_VERSION, MetricEventKind};
pub use event_store::{MetricEventStore, MetricWriteOutcome};
pub use window::{CohortRole, EvaluationWindow, MetricCohort};
pub use window_store::WindowDeclaration;
