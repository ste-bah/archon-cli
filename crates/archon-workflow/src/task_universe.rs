//! A decomposed-PRD task universe: the task files a generated run plans over.
//!
//! The topology lowering that used to live here stayed in the bin crate as
//! `crate::command::topology_task_graph`. It was the only thing in this module
//! that named `archon_topology`, and `archon-topology` depends on
//! `archon-workflow` — this crate — so the lowering could not travel with it.

// Re-exported into the child modules by their `use super::*`. They are listed
// here rather than in each child because the children are one module split
// three ways for the 500-line ceiling, not three independent units.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};

#[path = "task_status.rs"]
pub mod task_status;

#[path = "task_universe_a.rs"]
mod task_universe_a;
pub use task_universe_a::*;
#[path = "task_universe_b.rs"]
mod task_universe_b;
pub use task_universe_b::*;

#[path = "task_universe_parsing.rs"]
pub mod parsing;
use parsing::{merge_project_capabilities, parse_task_file};
