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

fn skeleton() -> TaskSkeleton {
    use crate::task_skeleton::{ConsumedArtifact, FrozenDependency, FrozenTask};
    use crate::task_universe::WorkflowV2DeliverableContract;
    let task = |id: &str| FrozenTask {
        task_id: id.to_string(),
        file_name: format!("{id}.md"),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        implements: Vec::new(),
        deliverable_contracts: Vec::new(),
    };
    let mut first = task("TASK-WS-005");
    first.blocks = vec!["TASK-WS-009".into()];
    first.implements = vec!["AC-WS-003".into()];
    first.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "artifact".into(),
        artifact_path: "out/registry.json".into(),
        ..WorkflowV2DeliverableContract::default()
    }];
    let mut second = task("TASK-WS-009");
    second.depends_on = vec![FrozenDependency {
        task_id: "TASK-WS-005".into(),
        consumes: vec![ConsumedArtifact {
            artifact_path: "out/registry.json".into(),
            ..ConsumedArtifact::default()
        }],
        ordering_only: false,
    }];
    let mut third = task("TASK-WS-012");
    third.depends_on = vec![FrozenDependency {
        task_id: "TASK-WS-009".into(),
        consumes: Vec::new(),
        ordering_only: true,
    }];
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "d".into(),
        tasks: vec![first, second, third],
    }
}

/// Every skeleton task is one line with its id, file, depends_on (with the
/// consumed paths and the ordering-only flag), blocks, implements and
/// deliverable paths — including a task whose body is not in the cluster.
#[test]
fn skeleton_summary_lists_every_task_with_its_frozen_edges() {
    let summary = SkeletonSummary::from_skeleton(&skeleton());
    let lines: Vec<&str> = summary.as_str().lines().collect();
    assert_eq!(lines.len(), 4, "{}", summary.as_str());
    assert!(lines[0].starts_with("FROZEN SKELETON (every task in the set"));
    assert_eq!(
        lines[1],
        r#"{"task_id":"TASK-WS-005","file_name":"TASK-WS-005.md","depends_on":[],"blocks":["TASK-WS-009"],"implements":["AC-WS-003"],"deliverable_contracts":[{"kind":"artifact","artifact_path":"out/registry.json"}]}"#
    );
    assert_eq!(
        lines[2],
        r#"{"task_id":"TASK-WS-009","file_name":"TASK-WS-009.md","depends_on":[{"task_id":"TASK-WS-005","consumes":["out/registry.json"],"ordering_only":false}],"blocks":[],"implements":[],"deliverable_contracts":[]}"#
    );
    assert_eq!(
        lines[3],
        r#"{"task_id":"TASK-WS-012","file_name":"TASK-WS-012.md","depends_on":[{"task_id":"TASK-WS-009","consumes":[],"ordering_only":true}],"blocks":[],"implements":[],"deliverable_contracts":[]}"#
    );
    assert!(
        SkeletonSummary::absent()
            .as_str()
            .contains("has no frozen skeleton")
    );
}

#[test]
fn prompt_carries_the_typed_question_every_obligation_and_every_task_text() {
    let prompt = fidelity_prompt(&obligations(), &tasks(), &SkeletonSummary::absent());
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
    assert!(prompt.contains("this task set has no frozen skeleton"));
}

/// The skeleton section sits after the obligations and before the task
/// texts, and the instruction says what it establishes.
#[test]
fn prompt_places_the_skeleton_between_obligations_and_task_texts() {
    let summary = SkeletonSummary::from_skeleton(&skeleton());
    let prompt = fidelity_prompt(&obligations(), &tasks(), &summary);
    let obligations_at = prompt.find("Obligations: [").expect("obligations");
    let skeleton_at = prompt
        .find("FROZEN SKELETON (every task in the set")
        .expect("skeleton section");
    let first_task_at = prompt
        .find("===== BEGIN TASK TASK-WS-005 =====")
        .expect("first task");
    assert!(obligations_at < skeleton_at && skeleton_at < first_task_at);
    assert!(
        prompt.contains(r#"{"task_id":"TASK-WS-012","#),
        "a task with no text included is still listed"
    );
    assert!(prompt.contains(
        "Inter-task ordering and result ownership are FACTS established by the frozen skeleton"
    ));
    assert!(prompt.contains("depends_on is transitive"));
    assert!(prompt.contains("its absence is never by itself a ground to refute an obligation"));
    assert!(!prompt.contains("this task set has no frozen skeleton"));
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
            "not in the audited cluster",
        ),
        (
            r#"{"verdicts":[{"obligation_id":"AC-WS-003","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}]}"#,
            "missing=[\"DONE-9\"]",
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
}

/// Issue-42: a verbose critic is cut, not refused. Live, two true verdicts
/// failed the gate operationally because one reason ran past 400 characters.
#[test]
fn an_over_long_reason_is_cut_to_the_limit_and_accepted() {
    let reason = (0..45)
        .map(|n| format!("clause {n:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(reason.chars().count(), 449);
    let reply = format!(
        r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true,"weakest_task_id":"","reason":"{reason}","quoted_task_text":""}},{{"obligation_id":"DONE-9","necessarily_true":true,"weakest_task_id":"","reason":"r","quoted_task_text":""}}]}}"#
    );
    let verdicts = parse_fidelity_response(&reply, &obligations(), &tasks()).expect("parses");
    let kept = &verdicts[0].reason;
    assert_eq!(kept.chars().count(), MAX_REASON_CHARS + 1);
    assert!(kept.ends_with('…'), "{kept}");
    let prefix: String = reason.chars().take(MAX_REASON_CHARS).collect();
    assert_eq!(kept.trim_end_matches('…'), prefix);
    assert_eq!(
        verdicts[1].reason, "r",
        "a reason within the limit is untouched"
    );
}

/// A long task whose text is one run of numbered clauses, so a quote can
/// exceed the limit while staying verbatim, or drift after the cut.
fn long_task() -> (Vec<ClaimedObligation>, Vec<ClaimingTask>, String) {
    let body = (0..80)
        .map(|n| format!("clause {n:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let obligations = vec![ClaimedObligation {
        id: "OBL-1".into(),
        text: "The result exists.".into(),
    }];
    let tasks = vec![ClaimingTask {
        task_id: "TASK-A-001".into(),
        text: format!("# TASK-A-001\n\n{body}\n"),
    }];
    (obligations, tasks, body)
}

fn false_reply(quote: &str) -> String {
    serde_json::json!({"verdicts": [{"obligation_id": "OBL-1", "necessarily_true": false, "weakest_task_id": "TASK-A-001", "reason": "r", "quoted_task_text": quote}]}).to_string()
}

/// Issue-42: an over-long quote is cut to the limit and then checked verbatim
/// — a cut excerpt that the task still contains is accepted, one that drifts
/// from the task before the cut is refused exactly as before.
#[test]
fn an_over_long_quote_is_cut_then_checked_verbatim() {
    let (obligations, tasks, body) = long_task();
    let verbatim: String = body.chars().take(MAX_QUOTE_CHARS + 50).collect();
    let verdicts = parse_fidelity_response(&false_reply(&verbatim), &obligations, &tasks)
        .expect("a cut verbatim excerpt is still verbatim");
    let kept = &verdicts[0].quoted_task_text;
    assert_eq!(kept.chars().count(), MAX_QUOTE_CHARS);
    assert!(verbatim.starts_with(kept.as_str()));

    let drifted = format!(
        "{} paraphrased here {}",
        body.chars().take(100).collect::<String>(),
        body.chars()
            .skip(120)
            .take(MAX_QUOTE_CHARS)
            .collect::<String>()
    );
    assert!(drifted.chars().count() > MAX_QUOTE_CHARS);
    let error = parse_fidelity_response(&false_reply(&drifted), &obligations, &tasks)
        .expect_err("a paraphrase is refused however long it is");
    assert!(error.contains("does not appear verbatim"), "{error}");
}

#[test]
fn cluster_digest_changes_with_any_text_or_the_skeleton_and_nothing_else() {
    let frozen = SkeletonSummary::from_skeleton(&skeleton());
    let base = fidelity_cluster_digest(&obligations(), &tasks(), &frozen);
    assert_eq!(
        base,
        fidelity_cluster_digest(&obligations(), &tasks(), &frozen)
    );
    let mut edited = tasks();
    edited[1].text.push_str("\nOne more allowance.\n");
    assert_ne!(
        base,
        fidelity_cluster_digest(&obligations(), &edited, &frozen)
    );
    let mut reworded = obligations();
    reworded[0].text = "Something else.".into();
    assert_ne!(base, fidelity_cluster_digest(&reworded, &tasks(), &frozen));
    let mut refrozen = skeleton();
    refrozen.tasks[2].depends_on.clear();
    let refrozen = SkeletonSummary::from_skeleton(&refrozen);
    assert_ne!(
        base,
        fidelity_cluster_digest(&obligations(), &tasks(), &refrozen)
    );
    assert_ne!(
        base,
        fidelity_cluster_digest(&obligations(), &tasks(), &SkeletonSummary::absent())
    );
}
