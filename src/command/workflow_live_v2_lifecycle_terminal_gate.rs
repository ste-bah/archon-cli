use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use archon_workflow::generated_lifecycle_support::{self as support, LifecycleContract};

#[path = "workflow_live_v2_lifecycle_terminal_gate_a.rs"]
mod workflow_live_v2_lifecycle_terminal_gate_a;
pub(crate) use workflow_live_v2_lifecycle_terminal_gate_a::*;
#[path = "workflow_live_v2_lifecycle_terminal_gate_b.rs"]
mod workflow_live_v2_lifecycle_terminal_gate_b;
pub(super) use workflow_live_v2_lifecycle_terminal_gate_b::*;
