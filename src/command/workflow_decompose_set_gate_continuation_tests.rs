//! P6 unattributed routing and frozen-chain regression coverage.
use super::*;

#[test]
#[allow(non_snake_case)]
fn P6_16_unattributed_body_finding_is_logged_and_sent_to_skeleton() {
    let stray = r#"{ "text": "obligation AC-X-001 is hollow — task TASK-Z-999: \"x\"", "subject": "AC-X-001", "source_path": "/elsewhere/TASK-Z-999.md", "remediation_scope": "body" }"#;
    let out = run(&driver("{}", &format!("[[{stray}], []]"), ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(count(&out, "hostCalls", "freeze-skeleton"), 2, "{out}");
    assert!(
        out["prompts"]["skeleton-author-2"]
            .to_string()
            .contains("TASK-Z-999"),
        "{out}"
    );
    assert!(
        out["checkpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["id"]
                .as_str()
                .unwrap()
                .starts_with("finding_unattributed-")
                && event["task"].as_str().unwrap().contains("TASK-Z-999")),
        "{out}"
    );
}

const FROZEN_SUBJECTS: &str = r#"[{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }, { taskId: "TASK-X-020", fileName: "TASK-X-020.md" }]"#;

#[test]
fn a_fully_frozen_chain_is_verified_not_authored_before_the_set_gate() {
    let args = format!(
        r#"{{ frozenChain: {{ acceptance: true, skeleton: true, subjects: {FROZEN_SUBJECTS}, bodies: ["TASK-X-010.md", "TASK-X-020.md"] }} }}"#
    );
    let out = run(&driver(&args, "[[]]", ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!([]),
        "no author call at all: {out}"
    );
    assert_eq!(
        out["hostCalls"],
        serde_json::json!([
            "verify-frozen-acceptance",
            "verify-frozen-skeleton",
            "task-set-lint",
            "requirements-trace"
        ]),
        "{out}"
    );
    assert_eq!(
        out["evidence"], 4,
        "acceptance + skeleton + the two gates; frozen bodies carry no author outcome: {out}"
    );
}

#[test]
fn a_frozen_chain_with_one_body_missing_authors_only_that_body() {
    let args = format!(
        r#"{{ frozenChain: {{ acceptance: true, skeleton: true, subjects: {FROZEN_SUBJECTS}, bodies: ["TASK-X-010.md"] }} }}"#
    );
    let out = run(&driver(&args, "[[]]", ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!(["body-TASK-X-020-author-1"]),
        "{out}"
    );
    assert_eq!(
        out["hostCalls"],
        serde_json::json!([
            "verify-frozen-acceptance",
            "verify-frozen-skeleton",
            "land-task-body",
            "task-set-lint",
            "requirements-trace"
        ]),
        "{out}"
    );
    assert_eq!(out["evidence"], 5, "{out}");
}

#[test]
fn a_frozen_body_the_set_gate_sends_back_is_re_authored_like_any_other() {
    let args = format!(
        r#"{{ frozenChain: {{ acceptance: true, skeleton: true, subjects: {FROZEN_SUBJECTS}, bodies: ["TASK-X-010.md", "TASK-X-020.md"] }} }}"#
    );
    let finding = body_finding("TASK-X-010", &format!("\"{TASK_ROOT}/TASK-X-010.md\""));
    let out = run(&driver(&args, &format!("[[{finding}], []]"), ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!(["body-TASK-X-010-author-1"]),
        "{out}"
    );
    assert_eq!(count(&out, "hostCalls", "task-set-lint"), 2);
    assert_eq!(
        out["evidence"], 5,
        "the re-authored frozen body now carries an author outcome: {out}"
    );
}

#[test]
fn a_frozen_acceptance_contract_alone_skips_only_the_acceptance_author() {
    let out = run(&driver(
        r#"{ frozenChain: { acceptance: true, skeleton: false, subjects: [], bodies: [] } }"#,
        "[[]]",
        "",
    ));
    assert!(out.get("error").is_none(), "{out}");
    let agent_calls = out["agentCalls"].as_array().unwrap();
    assert!(
        agent_calls
            .iter()
            .all(|id| !id.as_str().unwrap().starts_with("acceptance-author-")),
        "{out}"
    );
    assert_eq!(out["hostCalls"][0], "verify-frozen-acceptance", "{out}");
    assert_eq!(out["hostCalls"][1], "freeze-skeleton", "{out}");
    assert_eq!(out["evidence"], 6, "{out}");
}

#[test]
fn a_frozen_chain_whose_subjects_differ_at_verification_stops_the_run() {
    let args = r#"{ frozenChain: { acceptance: true, skeleton: true, subjects: [{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }], bodies: ["TASK-X-010.md"] } }"#;
    let out = run(&driver(args, "[[]]", ""));
    let error = out["error"].as_str().expect("the run must stop");
    assert!(error.contains("changed underneath the run"), "{error}");
    assert_eq!(out["agentCalls"], serde_json::json!([]), "{out}");
}
