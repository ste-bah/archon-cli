//! The canonical task graph behind a dynamic write or verification wave.
//!
//! A generated run hands the host an array of `source_data` items. This module
//! resolves those items against the authoritative task universe, rejects the
//! shapes a wave kind requires and does not have, expands their declared Rust
//! targets, and fingerprints the result. That fingerprint is what a later
//! attempt compares to decide whether a completed call may be reused, so the
//! decision belongs beside the result store holding the record and the task
//! universe it resolves against rather than in the binary that drives it.
//!
//! The child modules below are one module split for the 500-line ceiling, not
//! independent units -- the imports they share are declared here and reach them
//! through their `use super::*`.

use std::collections::{BTreeMap, BTreeSet};

use crate::generated_contract::{GeneratedContractIssueKind, normalize_generated_item_value};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::stable_value_hash;
use crate::v2::target_expansion::expand_declared_rust_module_targets;
use crate::{
    WorkflowV2CallExecution, WorkflowV2HostMethod, WorkflowV2SourceTargetExpansion,
    WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2WriteMode,
};

#[path = "source_graph_core.rs"]
mod source_graph_core;
pub use source_graph_core::*;

// The four modules below export only cluster-internal helpers (`pub(super)`),
// so these imports stay private: nothing outside this module and its children
// may name them, and a `pub`-flavoured glob here would re-export nothing.
#[path = "source_graph_build.rs"]
mod source_graph_build;
use source_graph_build::*;

#[path = "source_graph_fields.rs"]
mod source_graph_fields;
use source_graph_fields::*;

#[path = "source_graph_diagnostics.rs"]
mod source_graph_diagnostics;
use source_graph_diagnostics::*;

#[path = "source_graph_helpers.rs"]
mod source_graph_helpers;
use source_graph_helpers::*;

#[cfg(test)]
#[path = "source_graph_tests.rs"]
mod tests;
