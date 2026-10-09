use super::*;

#[test]
fn denial_words_add_guidance_to_failed_execution_without_making_it_a_refusal() {
    let boundary = WriteBoundary {
        read_only: false,
        protected: vec![PathBuf::from("/sealed")],
        ancestors: Vec::new(),
        writable: vec![PathBuf::from("/worktree")],
        siblings: Vec::new(),
        new_dirs: Vec::new(),
    };
    let successful = ToolResult::from_authoritative_bash_execution(
        "Permission denied".into(),
        "session".into(),
        "tool-use".into(),
        0,
        "printf 'Permission denied'".into(),
        0,
    );
    let successful = boundary.annotate(successful);
    assert!(!successful.is_error, "{}", successful.content);
    assert!(
        !successful.content.contains(WRITE_BOUNDARY_NOTE_MARKER),
        "{}",
        successful.content
    );
    assert!(!successful.is_guard_refusal());

    let denied = ToolResult::from_authoritative_bash_execution(
        "Permission denied".into(),
        "session".into(),
        "tool-use".into(),
        0,
        "printf 'Permission denied'; exit 1".into(),
        1,
    );
    let denied = boundary.annotate(denied);
    assert!(denied.is_error, "{}", denied.content);
    assert!(!denied.is_guard_refusal());
    assert!(denied.content.contains(WRITE_BOUNDARY_NOTE_MARKER));
}
