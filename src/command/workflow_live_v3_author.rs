// v3 script lifecycle: a planner agent AUTHORS workflow.js from the task
// universe using the documented primitive dialect, then the authored script
// executes through the QuickJS runtime — composition is code, judgment is a
// script-spawned agent, and every write flows through the same gauntlet.

use super::*;

include!("workflow_live_v3_author_a.rs");
include!("workflow_live_v3_author_b.rs");
