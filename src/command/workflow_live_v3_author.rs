// v3 script lifecycle: a planner agent AUTHORS workflow.js from the task
// universe using the documented primitive dialect, then the authored script
// executes through the QuickJS runtime — composition is code, judgment is a
// script-spawned agent, and every write flows through the same gauntlet.

use super::*;

#[path = "workflow_live_v3_author_a.rs"]
mod workflow_live_v3_author_a;
pub(crate) use workflow_live_v3_author_a::*;
#[path = "workflow_live_v3_author_b.rs"]
mod workflow_live_v3_author_b;
pub(crate) use workflow_live_v3_author_b::*;
