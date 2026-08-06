use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::generated_lifecycle_support::{self as support, LifecycleContract};

use super::noop_routing;

#[path = "terminal_gate_a.rs"]
mod terminal_gate_a;
pub use terminal_gate_a::*;
#[path = "terminal_gate_b.rs"]
mod terminal_gate_b;
pub(crate) use terminal_gate_b::*;
