use super::*;

#[test]
fn reply_blocks_brace_double_fault_names_the_continued_field() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = r#"{"id":"AC-X-001","check":{"kind":"command","command":"run"}}},"gap_permitted":false,"judgment":{"verdict":"accepted"}}"#;
    let error = candidate_entry_with_markers(reply, frozen, &[]).unwrap_err();
    assert!(error.contains("before \"gap_permitted\""), "{error}");
    assert!(error.contains("check.cwd"), "{error}");
    assert!(!error.contains("different entries"), "{error}");
}

#[test]
fn reply_blocks_brace_double_fault_after_complete_check_is_refused() {
    let set = frozen_set(&[("AC-X-001", "false", false)]);
    let frozen = &set.contract().acceptance[0];
    let valid =
        r#"{"id":"AC-X-001","check":{"kind":"command","command":"true","cwd":"project_root"}}"#;
    assert!(candidate_entry_with_markers(valid, frozen, &[]).is_ok());
    let reply = r#"{"id":"AC-X-001","check":{"kind":"command","command":"true","cwd":"project_root"}}},"gap_permitted":false}"#;
    let result = candidate_entry_with_markers(reply, frozen, &[]);
    assert!(result.is_err(), "malformed reply was accepted: {result:?}");
    assert!(result.unwrap_err().contains("before \"gap_permitted\""));
}

#[test]
fn reply_blocks_brace_unknown_field_does_not_hide_missing_fields() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply =
        r#"{"id":"AC-X-001","check":{"kind":"command","unexpected":true}},"gap_permitted":false}"#;
    let error = candidate_entry_with_markers(reply, frozen, &[]).unwrap_err();
    assert!(error.contains("check.command"), "{error}");
    assert!(error.contains("check.cwd"), "{error}");
}

#[test]
fn reply_blocks_brace_early_close_also_names_the_second_entry() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let first = r#"{"id":"AC-X-001","check":{"kind":"command","command":"run"}},"gap_permitted":false,"judgment":{"verdict":"accepted"}}"#;
    let second =
        r#"{"id":"AC-X-002","check":{"kind":"command","command":"run","cwd":"project_root"}}"#;
    for reply in [
        format!("{first}\n{second}"),
        format!("{first}\n```json\n{second}\n```"),
        format!("```json\n{second}\n```\n{first}"),
    ] {
        let error = candidate_entry_with_markers(&reply, frozen, &[]).unwrap_err();
        assert!(
            error.contains("before \"gap_permitted\""),
            "{error}; reply: {reply}"
        );
        assert!(
            error.contains("additional entry \"AC-X-002\""),
            "{error}; reply: {reply}"
        );
    }
}
