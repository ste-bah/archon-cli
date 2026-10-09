use super::*;

#[test]
fn pure_document_has_no_discard_diagnostic() {
    let raw = br#"{"kind":"x"}"#;
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.document, raw);
    assert_eq!(extracted.discarded_before, 0);
    assert_eq!(extracted.discarded_after, 0);
    assert!(!extracted.unwrapped_fence);
    assert_eq!(extraction_diagnostic(&extracted), None);
}

#[test]
fn one_json_fence_is_marked_without_counting_packaging_as_discarded_text() {
    let raw = b"```json\n{\"kind\":\"x\"}\n```";
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.document, b"\n{\"kind\":\"x\"}");
    assert_eq!(extracted.discarded_before, 0);
    assert_eq!(extracted.discarded_after, 1);
    assert!(extracted.unwrapped_fence);
    let diagnostic = extraction_diagnostic(&extracted).expect("whitespace discard note");
    assert!(diagnostic.contains("whitespace only"), "{diagnostic}");
    assert!(!diagnostic.contains("excerpt="), "{diagnostic}");
}

#[test]
fn chat_before_a_fence_is_recorded_in_host_result_stderr() {
    let raw = b"I could not verify X.\n```json\n{\"kind\":\"x\"}\n```";
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.discarded_before, b"I could not verify X.\n".len());
    let mut stderr = Vec::new();
    record_candidate_extraction(raw, &mut stderr).expect("write diagnostic");
    let stderr = String::from_utf8(stderr).expect("UTF-8 diagnostic");
    let stored_result = archon_workflow::HostCommandResult {
        exit_code: Some(0),
        stdout: String::new(),
        stderr_bytes: stderr.len() as u64,
        stderr,
        stdout_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: None,
        publication_receipt: None,
        subjects: Vec::new(),
        postcondition: None,
    };
    assert!(stored_result.stderr.contains("I could not verify X."));
    assert!(stored_result.stderr.contains("discarded_before="));
}

#[test]
fn commentary_after_the_first_complete_value_is_counted() {
    let raw = b"{\"kind\":\"x\"}\nI could not verify Y.";
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.document, b"{\"kind\":\"x\"}");
    assert_eq!(extracted.discarded_after, b"\nI could not verify Y.".len());
}

#[test]
fn whitespace_after_a_complete_value_is_counted_and_described_without_preview() {
    let raw = b"{\"kind\":\"x\"} \n\t";
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.document, b"{\"kind\":\"x\"}");
    assert_eq!(extracted.discarded_after, b" \n\t".len());
    let diagnostic = extraction_diagnostic(&extracted).expect("discard note");
    assert!(
        diagnostic.contains("discarded_after=3 bytes"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("whitespace only"), "{diagnostic}");
    assert!(!diagnostic.contains("excerpt="), "{diagnostic}");
}

#[test]
fn whitespace_after_a_fenced_value_is_counted_and_described_without_preview() {
    let raw = b"```json\n{\"kind\":\"x\"} \n\t```";
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.document, b"\n{\"kind\":\"x\"}");
    assert_eq!(extracted.discarded_after, b" \n\t".len());
    let diagnostic = extraction_diagnostic(&extracted).expect("discard note");
    assert!(
        diagnostic.contains("discarded_after=3 bytes"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("whitespace only"), "{diagnostic}");
    assert!(!diagnostic.contains("excerpt="), "{diagnostic}");
}

#[test]
fn two_fenced_blocks_remain_whole_and_fail_as_before() {
    let raw = b"one\n```json\n{\"a\":1}\n```\ntwo\n```json\n{\"b\":2}\n```\n";
    let extracted = candidate_extraction(raw);
    assert_eq!(extracted.document, raw);
    assert_eq!(extracted.discarded_before, 0);
    assert_eq!(extracted.discarded_after, 0);
    assert!(!extracted.unwrapped_fence);
    assert!(candidate_parse_error::<serde_json::Value>(raw).is_some());
}

#[test]
fn secret_shaped_discarded_text_is_redacted_in_the_diagnostic() {
    let raw = b"token=sk-12345678901234567890\n```json\n{\"kind\":\"x\"}\n```";
    let extracted = candidate_extraction(raw);
    let diagnostic = extraction_diagnostic(&extracted).expect("discard note");
    assert!(
        !diagnostic.contains("sk-12345678901234567890"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("***REDACTED***"), "{diagnostic}");
}
