//! The Rust twin of the generated scaffold's contract JS.
//!
//! Normalizes the inventory and item values a generated run produces against
//! the authoritative task universe, and reports every repair it had to make as
//! a [`GeneratedContractIssue`]. That is execution, not CLI, which is why it
//! sits beside [`crate::task_universe`] rather than in the binary; the binary
//! now reaches it as `archon_workflow::generated_contract`.
//!
//! The child modules below are one module split for the 500-line ceiling, not
//! independent units — the imports they share are declared here and reach them
//! through their `use super::*`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::task_universe::WorkflowV2TaskUniverse;

#[path = "generated_contract_a.rs"]
mod generated_contract_a;
pub use generated_contract_a::*;
#[path = "generated_contract_b.rs"]
mod generated_contract_b;
pub use generated_contract_b::*;
// The five modules below export only cluster-internal helpers (`pub(super)`),
// so these imports stay private: nothing outside this module and its children
// may name them, and a `pub`-flavoured glob here would re-export nothing.
#[path = "generated_contract_validation_noop.rs"]
mod generated_contract_validation_noop;
use generated_contract_validation_noop::*;
#[path = "generated_contract_universe_queries.rs"]
mod generated_contract_universe_queries;
#[path = "generated_contract_validation.rs"]
mod generated_contract_validation;
use generated_contract_validation::*;
#[path = "generated_contract_helpers.rs"]
mod generated_contract_helpers;
use generated_contract_helpers::*;
#[path = "generated_contract_artifacts.rs"]
mod generated_contract_artifacts;
use generated_contract_artifacts::*;
#[path = "generated_contract_retry.rs"]
mod generated_contract_retry;
use generated_contract_retry::*;
#[path = "generated_contract_invariants.rs"]
mod generated_contract_invariants;
use generated_contract_invariants::*;
#[cfg(test)]
#[path = "generated_contract_execution_tests.rs"]
mod execution_tests;
#[cfg(test)]
#[path = "generated_contract_grouping_tests.rs"]
mod grouping_tests;
#[cfg(test)]
#[path = "generated_contract_refuted_noop_tests.rs"]
mod refuted_noop_tests;
#[cfg(test)]
#[path = "generated_contract_tests.rs"]
mod tests;
