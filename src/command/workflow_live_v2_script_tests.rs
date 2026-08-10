use super::*;

#[path = "workflow_live_v2_script_tests_a.rs"]
mod workflow_live_v2_script_tests_a;
use workflow_live_v2_script_tests_a::*;
#[path = "workflow_live_v2_script_delivery_tests.rs"]
mod workflow_live_v2_script_delivery_tests;
#[path = "workflow_live_v2_script_tests_b.rs"]
mod workflow_live_v2_script_tests_b;
#[path = "workflow_live_v2_script_tests_c.rs"]
mod workflow_live_v2_script_tests_c;
use workflow_live_v2_script_tests_b::*;
#[path = "workflow_live_v2_script_tests_d.rs"]
mod workflow_live_v2_script_tests_d;
use workflow_live_v2_script_tests_d::*;
#[path = "workflow_live_v2_reuse_content_key_tests.rs"]
mod workflow_live_v2_reuse_content_key_tests;
#[path = "workflow_live_v2_script_tests_e.rs"]
mod workflow_live_v2_script_tests_e;
#[path = "workflow_live_v2_script_tests_f.rs"]
mod workflow_live_v2_script_tests_f;
// Issue #162 — the events.jsonl / v2-results agreement invariant.
#[path = "workflow_live_v2_blocking_gap_tests.rs"]
mod workflow_live_v2_blocking_gap_tests;
