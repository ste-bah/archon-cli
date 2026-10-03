//! Transcript history is model input, never resume authority.
use super::*;

#[test]
fn history_paths_follow_the_same_write_boundary_as_other_files() {
    let root = crate::agent_records::sessions_root().expect("home");
    let history = root.join("fixture/subagents/agent-child.jsonl");
    let ctx = ToolContext {
        subagent_id: Some("child".into()),
        ..Default::default()
    };
    ensure_write_allowed(&history, &history, &ctx)
        .expect("an unconfined agent needs no special history-write restriction");
    let temp = tempfile::tempdir().unwrap();
    let confined = ToolContext {
        write_roots: vec![temp.path().to_path_buf()],
        ..ctx
    };
    assert!(ensure_write_allowed(&history, &history, &confined).is_err());
}
