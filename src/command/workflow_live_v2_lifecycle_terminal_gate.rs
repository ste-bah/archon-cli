use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support::{
    self as support, LifecycleContract,
};

include!("workflow_live_v2_lifecycle_terminal_gate_a.rs");
include!("workflow_live_v2_lifecycle_terminal_gate_b.rs");
