use super::tool_tap::permission_class;
use super::workflow_tap::workflow_trace_record;
use super::*;

use archon_core::orchestrator::events::{Subtask, SubtaskStatus};
use archon_tools::tool::PermissionLevel;
use archon_topology::ir::{GraphOrigin, PermissionClass, WriteTarget};
use archon_topology::reconstruct::ROOT_NODE_ID;
use archon_topology::trace::read_trace;

fn outcome(tool: &str, input: serde_json::Value) -> ToolRunAttemptOutcome {
    ToolRunAttemptOutcome {
        session_id: "s1".into(),
        parent_action_id: "parent-1".into(),
        tool_use_id: "tu-1".into(),
        attempt: 0,
        tool_name: tool.into(),
        input,
        permission_level: PermissionLevel::Safe,
        blocked: false,
        is_error: false,
        admission_evaluated: false,
    }
}

fn trace_in(dir: &std::path::Path) -> AmbientTrace {
    AmbientTrace::open(dir, "g1", "s1").unwrap()
}


mod orchestrator_tap;
mod tool_tap;
mod workflow_tap;

#[test]
fn the_ambient_slot_installs_and_clears() {
    // The slot is process-wide, so this must not interleave with any other test
    // that installs into it — notably the fold's no-database-access test.
    let _guard = test_lock();
    let temp = tempfile::tempdir().unwrap();

    end();
    assert!(active().is_none());

    let trace = begin(temp.path(), "g-ambient", "s1").expect("begin must succeed on a temp dir");
    assert!(active().is_some());
    assert_eq!(active().unwrap().graph_id(), "g-ambient");

    // The free-function taps route to the installed trace.
    on_tool_run_outcome(&outcome("Bash", serde_json::json!({})));
    assert_eq!(
        read_trace(&trace.paths().trace_jsonl("g-ambient"))
            .unwrap()
            .records
            .len(),
        1
    );

    end();
    assert!(active().is_none());
    // With nothing installed the taps are no-ops rather than panics.
    on_tool_run_outcome(&outcome("Bash", serde_json::json!({})));
    on_orchestrator_event(&OrchestratorEvent::TeamCancelled);
    assert_eq!(
        read_trace(&trace.paths().trace_jsonl("g-ambient"))
            .unwrap()
            .records
            .len(),
        1,
        "a cleared slot must record nothing"
    );
}
