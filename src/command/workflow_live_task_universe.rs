//! A decomposed-PRD task universe: the task files a generated run plans over.
//!
//! The topology lowering that used to live here moved to
//! [`crate::command::topology_task_graph`]. It was the only thing in this
//! module that named `archon_topology`, and `archon-topology` depends on
//! `archon-workflow`, which this file is destined for.

// Re-exported into the child modules by their `use super::*`. They are listed
// here rather than in each child because the children are one module split
// three ways for the 500-line ceiling, not three independent units.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[path = "workflow_live_task_status.rs"]
pub(crate) mod task_status;

#[path = "workflow_live_task_universe_a.rs"]
mod workflow_live_task_universe_a;
pub(crate) use workflow_live_task_universe_a::*;
#[path = "workflow_live_task_universe_b.rs"]
mod workflow_live_task_universe_b;
pub(crate) use workflow_live_task_universe_b::*;

#[path = "workflow_live_task_universe_parsing.rs"]
pub(crate) mod parsing;
use parsing::{merge_project_capabilities, parse_task_file};
