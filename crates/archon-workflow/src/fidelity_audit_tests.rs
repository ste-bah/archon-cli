//! Prompt, schema and strict parse of the fidelity verdict.

use super::*;

fn obligations() -> Vec<ClaimedObligation> {
    vec![
        ClaimedObligation {
            id: "AC-WS-003".into(),
            text: "Native widget ingestion stores a registry entry.".into(),
        },
        ClaimedObligation {
            id: "DONE-9".into(),
            text: "A WidgetSpec exists and references registered widgets.".into(),
        },
    ]
}

fn tasks() -> Vec<ClaimingTask> {
    vec![
        ClaimingTask {
            task_id: "TASK-WS-005".into(),
            text: "# TASK-WS-005\n\nScope: library-level focused tests against a temporary\ntarget root; the shared project root is untouched.\n".into(),
        },
        ClaimingTask {
            task_id: "TASK-WS-009".into(),
            text: "# TASK-WS-009\n\nBindings may carry status: pending-ingest.\n".into(),
        },
    ]
}

#[test]
fn prompt_carries_the_typed_question_every_obligation_and_every_task_text() {
    let prompt = fidelity_prompt(&obligations(), &tasks());
    assert!(prompt.contains("Assume every listed task passes its own acceptance criteria"));
    assert!(prompt.contains("is the PRD obligation then necessarily true?"));
    assert!(prompt.contains("Answer strictly"));
    assert!(prompt.contains("\"id\":\"AC-WS-003\""));
    assert!(prompt.contains("references registered widgets"));
    assert!(prompt.contains("===== BEGIN TASK TASK-WS-005 ====="));
    assert!(prompt.contains("the shared project root is untouched"));
    assert!(prompt.contains("===== END TASK TASK-WS-009 ====="));
    assert!(prompt.contains(&format!("at most {MAX_REASON_CHARS} characters")));
    assert!(prompt.contains(&format!("at most {MAX_QUOTE_CHARS} characters")));
}

#[test]
fn both_verdicts_parse_and_a_false_verdict_renders_the_finding() {
    let reply = r#"{"verdicts":[
        {"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005",
         "reason":"the task tests against a temporary root so the shared registry may stay empty",
         "quoted_task_text":"library-level focused tests against a temporary target root; the shared project root is untouched."},
        {"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"the spec is required to reference registered ids","quoted_task_text":""}
    ]}"#;
    let verdicts = parse_fidelity_response(reply, &obligations(), &tasks()).expect("parses");
    assert_eq!(verdicts.len(), 2);
    assert!(!verdicts[0].necessarily_true);
    assert!(verdicts[1].necessarily_true);
    let finding = fidelity_finding(
        &verdicts[0],
        &["TASK-WS-005".to_string(), "TASK-WS-009".to_string()],
    );
    assert_eq!(
        finding,
        "obligation AC-WS-003 is claimed by TASK-WS-005, TASK-WS-009 but none is obliged to make it true — the task tests against a temporary root so the shared registry may stay empty — task TASK-WS-005: \"library-level focused tests against a temporary target root; the shared project root is untouched.\""
    );
}

#[test]
fn a_quote_that_re_wraps_lines_still_counts_as_verbatim() {
    let reply = r#"{"verdicts":[
        {"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":"against a temporary target root; the shared"},
        {"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}
    ]}"#;
    assert!(parse_fidelity_response(reply, &obligations(), &tasks()).is_ok());
}

/// Observed live: asked to escape newlines, the critic escaped them twice, so
/// the decoded quote carried literal backslash-n between lines it had copied
/// exactly. That is packaging, and must not fail the verdict as a paraphrase.
#[test]
fn a_quote_with_double_escaped_newlines_still_counts_as_verbatim() {
    let reply = r#"{"verdicts":[
        {"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":"Scope: library-level focused tests against a temporary\\ntarget root; the shared"},
        {"obligation_id":"DONE-9","necessarily_true":true,"reason":"r"}
    ]}"#;
    let verdicts = parse_fidelity_response(reply, &obligations(), &tasks()).expect("parses");
    assert!(
        verdicts[0].quoted_task_text.contains("\\n"),
        "the decoded string carries the literal"
    );
    assert_eq!(
        verdicts[1].weakest_task_id, "",
        "omitted fields default on a true verdict"
    );
}

#[test]
fn malformed_replies_are_errors_not_passes() {
    let cases: &[(&str, &str)] = &[
        ("not json", "not the verdict document"),
        (r#"{"verdicts":[]}"#, "do not match the cluster"),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""},{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""},{"obligation_id":"AC-WS-999","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "extra=[\"AC-WS-999\"]",
        ),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":"paraphrased loophole"},{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "does not appear verbatim",
        ),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-404","reason":"r","quoted_task_text":"temporary"},{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "does not claim it",
        ),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":""},{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "quotes nothing",
        ),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":true,"weakest_task_id":"","reason":"","quoted_task_text":""},{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "empty reason",
        ),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":"","extra":1},{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "not the verdict document",
        ),
    ];
    for (reply, expected) in cases {
        let error = parse_fidelity_response(reply, &obligations(), &tasks())
            .expect_err("malformed reply must not parse");
        assert!(error.contains(expected), "{reply}\n-> {error}");
    }
    let long = format!(
        r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true,"weakest_task_id":"","reason":"{}","quoted_task_text":""}},{{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}}]}}"#,
        "x".repeat(MAX_REASON_CHARS + 1)
    );
    let error = parse_fidelity_response(&long, &obligations(), &tasks()).expect_err("too long");
    assert!(error.contains("longer than"), "{error}");
}

#[test]
fn cluster_digest_changes_with_any_text_and_nothing_else() {
    let base = fidelity_cluster_digest(&obligations(), &tasks());
    assert_eq!(base, fidelity_cluster_digest(&obligations(), &tasks()));
    let mut edited = tasks();
    edited[1].text.push_str("\nOne more allowance.\n");
    assert_ne!(base, fidelity_cluster_digest(&obligations(), &edited));
    let mut reworded = obligations();
    reworded[0].text = "Something else.".into();
    assert_ne!(base, fidelity_cluster_digest(&reworded, &tasks()));
}
