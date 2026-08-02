use super::*;

#[test]
fn a_tool_attempt_records_one_line() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome("Bash", serde_json::json!({"command": "ls"})));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    assert_eq!(readout.records.len(), 1);
    assert_eq!(readout.records[0].kind, TraceKind::ToolAttempt);
    assert_eq!(readout.records[0].tool.as_deref(), Some("Bash"));
    assert_eq!(readout.records[0].node_id, ROOT_NODE_ID);
}

#[test]
fn tool_input_is_never_recorded_verbatim() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Bash",
        serde_json::json!({"command": "curl -H 'Authorization: Bearer sk-secret-value'"}),
    ));

    let raw = std::fs::read_to_string(trace.paths().trace_jsonl("g1")).unwrap();
    assert!(
        !raw.contains("sk-secret-value"),
        "the trace must not become a secret sink: {raw}"
    );
}

#[test]
fn a_subagent_spawn_becomes_a_node_with_an_edge_to_the_turn_root() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Agent",
        serde_json::json!({"subagent_type": "Explore", "prompt": "look"}),
    ));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    let spawn = readout
        .records
        .iter()
        .find(|record| record.kind == TraceKind::AgentSpawned)
        .expect("a subagent tool call must record a spawn");
    assert_eq!(spawn.parent_node_id.as_deref(), Some(ROOT_NODE_ID));
    assert_eq!(spawn.agent.as_deref(), Some("Explore"));
    assert!(spawn.node_id.starts_with("spawn-tu-1"));
}

#[test]
fn a_blocked_subagent_call_records_no_spawn() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    let mut blocked = outcome("Agent", serde_json::json!({"subagent_type": "Explore"}));
    blocked.blocked = true;
    trace.record_tool_outcome(&blocked);

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    assert!(
        !readout
            .records
            .iter()
            .any(|record| record.kind == TraceKind::AgentSpawned),
        "a blocked call spawned nothing"
    );
}

#[test]
fn a_write_and_a_read_are_recorded_as_separate_kinds() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Write",
        serde_json::json!({"file_path": "C:\\repo\\src\\a.rs", "content": "x"}),
    ));
    trace.record_tool_outcome(&outcome(
        "Read",
        serde_json::json!({"file_path": "src/b.rs"}),
    ));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    let written: Vec<&TraceRecord> = readout
        .records
        .iter()
        .filter(|record| record.kind == TraceKind::FileWritten)
        .collect();
    assert_eq!(written.len(), 1, "only the Write tool writes");
    assert_eq!(
        written[0].writes,
        vec![WriteTarget::Path("C:/repo/src/a.rs".into())],
        "separators must be normalized so exact-match conflict detection works"
    );

    let read: Vec<&TraceRecord> = readout
        .records
        .iter()
        .filter(|record| record.kind == TraceKind::FileRead)
        .collect();
    assert_eq!(read.len(), 1, "the Read tool reads and does not write");
    assert_eq!(read[0].reads, vec![WriteTarget::Path("src/b.rs".into())]);
    assert!(read[0].writes.is_empty(), "a read is not a write");
}

/// `Grep` and `Glob` read, but their `path` is a search root rather than a
/// file. The coupling check compares read targets to write targets by exact
/// string, so a directory can never match — recording one would add rows that
/// cannot ever produce a finding.
#[test]
fn a_search_root_is_not_recorded_as_a_read_target() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Grep",
        serde_json::json!({"pattern": "fn main", "path": "src"}),
    ));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    assert!(
        !readout
            .records
            .iter()
            .any(|record| record.kind == TraceKind::FileRead),
        "a search over a directory names no file"
    );
}

/// An `Edit` is both halves of a dataflow: it cannot rewrite a file it did not
/// first read. Recording only the write would make a genuine coupling — one
/// branch editing what another branch produced — invisible.
#[test]
fn an_edit_records_both_a_read_and_a_write_of_the_same_file() {
    let temp = tempfile::tempdir().unwrap();
    let trace = trace_in(temp.path());

    trace.record_tool_outcome(&outcome(
        "Edit",
        serde_json::json!({"file_path": "src/a.rs", "old_string": "a", "new_string": "b"}),
    ));

    let readout = read_trace(&trace.paths().trace_jsonl("g1")).unwrap();
    let target = WriteTarget::Path("src/a.rs".into());
    assert!(
        readout.records.iter().any(
            |record| record.kind == TraceKind::FileWritten && record.writes == [target.clone()]
        )
    );
    assert!(
        readout
            .records
            .iter()
            .any(|record| record.kind == TraceKind::FileRead && record.reads == [target.clone()])
    );
}

#[test]
fn permission_levels_map_onto_the_ir_classes() {
    assert_eq!(
        permission_class(PermissionLevel::Safe),
        PermissionClass::Safe
    );
    assert_eq!(
        permission_class(PermissionLevel::Risky),
        PermissionClass::Risky
    );
    assert_eq!(
        permission_class(PermissionLevel::Dangerous),
        PermissionClass::Irreversible,
        "under-classifying irreversibility is the failure that matters"
    );
}

