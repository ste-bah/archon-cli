use super::*;

#[test]
fn a_dispatched_session_reads_back_every_subagent_it_minted() {
    let session = "wf-sp-test-stage-call-a-attempt-1";
    let agent = format!("{session}-0-coder-uuid");
    note_turn(&agent, 3);
    note_turn(&agent, 2);
    note_tool_call(
        &agent,
        "Read",
        &serde_json::json!({"file_path": "src/a.rs"}),
    );
    note_tool_call(
        &agent,
        "Edit",
        &serde_json::json!({"file_path": "src/b.rs"}),
    );
    note_touched(&agent, std::path::Path::new("/w/src/b.rs"));
    note_touched(&agent, std::path::Path::new("/w/src/b.rs"));
    // A sibling whose id merely shares a stem is not this session's.
    note_turn(&format!("{session}0-other"), 9);

    let found = snapshot_for(session);
    assert_eq!(found.len(), 1, "{found:?}");
    let progress = &found[0];
    assert_eq!(progress.agent_id, agent);
    assert_eq!(progress.turns, 3, "turns never move backwards");
    assert_eq!(progress.tool_calls, 2);
    assert!(
        progress
            .last_tool_call
            .as_deref()
            .is_some_and(|call| call.starts_with("Edit ") && call.contains("src/b.rs")),
        "{progress:?}"
    );
    assert_eq!(progress.touched_paths, vec!["/w/src/b.rs".to_string()]);
    assert_eq!(progress.writes, 2);
    assert_eq!(writes(&agent), 2);
}

#[test]
fn a_long_argument_is_summarised_not_copied() {
    let agent = "wf-sp-test-long-0-coder";
    let body = "x".repeat(5_000);
    note_tool_call(agent, "Write", &serde_json::json!({"content": body}));
    let found = snapshot_for(agent);
    let last = found[0].last_tool_call.clone().unwrap_or_default();
    assert!(last.len() < 400 && last.ends_with("..."), "{}", last.len());
}

#[test]
fn an_empty_id_records_nothing() {
    note_turn("", 1);
    assert!(snapshot_for("").is_empty());
}
