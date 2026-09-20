//! Map-reduce review contract shapes: the exact-coverage pass, and the
//! gap/duplicate/unbounded and write/non-critic rejections.

use super::*;

#[test]
fn map_reduce_review_contract_passes_for_exact_coverage() {
    let expected = task_set(["TASK-EX-001", "TASK-EX-002"]);
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-002",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-002",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    validate_map_reduce_review_calls(&details, &expected).expect("complete review passes");
}

#[test]
fn map_reduce_review_rejects_gap_duplicate_and_unbounded_reduce() {
    let expected = task_set(["TASK-EX-001", "TASK-EX-002", "TASK-EX-003"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-002",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-003",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.review_reduce_edges[0].max_input_bytes = None;
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("bad review rejected");
    assert!(error.contains("TASK-EX-002"), "{error}");
    assert!(error.contains("TASK-EX-003"), "{error}");
    assert!(error.contains("more than once"), "{error}");
    assert!(error.contains("maxInputBytes"), "{error}");
}

#[test]
fn map_reduce_review_rejects_write_and_non_critic_reviews() {
    let expected = task_set(["TASK-EX-001"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.calls[1].write_mode = Some(WorkflowV2WriteMode::Worktree);
    details.calls[2].options.role = Some("coder".to_string());
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("unsafe review rejected");
    assert!(error.contains("read-only"), "{error}");
    assert!(error.contains("tier 'critic'"), "{error}");
}

#[test]
fn a_remediation_call_may_repeat_the_task_its_verifier_failed() {
    // The brief demands one initial write call per task AND a bounded
    // remediation loop re-running a write agent for that same task. Counting
    // every repeat as a duplicate claim made both instructions unsatisfiable:
    // a live attempt was rejected for writing exactly what it was told to.
    let claims = vec![
        ("TASK-A".to_string(), "agents-1".to_string()),
        ("TASK-B".to_string(), "agents-2".to_string()),
        ("TASK-A".to_string(), "remediate-task-a-2-5".to_string()),
        ("TASK-B".to_string(), "remediate-task-b-2-9".to_string()),
    ];
    assert!(
        super::v3_author_checks_a::cross_ownership_defects(&claims).is_empty(),
        "{:?}",
        super::v3_author_checks_a::cross_ownership_defects(&claims)
    );
}

#[test]
fn a_second_owner_claiming_other_tasks_too_is_still_a_defect() {
    let claims = vec![
        ("TASK-A".to_string(), "agents-1".to_string()),
        ("TASK-A".to_string(), "agents-2".to_string()),
        ("TASK-C".to_string(), "agents-2".to_string()),
    ];
    let defects = super::v3_author_checks_a::cross_ownership_defects(&claims);
    assert_eq!(defects.len(), 1, "{defects:?}");
    assert!(defects[0].contains("TASK-A"), "{defects:?}");
}

/// Diagnostic harness: validate a real authored draft against the pre-flight.
///
/// Ignored because it needs a script on disk. It exists because every synthetic
/// test in this file builds call structures by hand, so nothing here ran a real
/// script through the rehearsal — which is how a pre-flight that no correct
/// script could satisfy survived: the rehearsal reported zero accepted tasks
/// while the validator demanded review coverage of accepted tasks.
#[tokio::test]
#[ignore = "diagnostic; set ARCHON_DRAFT_PATH to an authored workflow.js"]
async fn a_real_draft_passes_the_preflight() {
    let path = std::env::var("ARCHON_DRAFT_PATH").expect("ARCHON_DRAFT_PATH");
    let source = std::fs::read_to_string(&path).expect("draft readable");
    let expected: std::collections::BTreeSet<String> = std::env::var("ARCHON_DRAFT_TASK_IDS")
        .expect("ARCHON_DRAFT_TASK_IDS")
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();

    match super::super::v3_author_checks_a::validate_authored_plan(&source, &expected).await {
        Ok(()) => println!("PRE-FLIGHT PASSED"),
        Err(reason) => panic!("PRE-FLIGHT REJECTED: {reason}"),
    }
}

#[test]
fn accounting_may_restore_a_finding_the_reducer_dropped() {
    // `preserveMapFindings` is an instruction to a model, and the prelude
    // repairs a model that ignores it by merging the map findings back in.
    // Requiring the raw reduce record to contain them forbade that repair and
    // discarded a completed run over one `severity: none` observation the host
    // had already put back.
    let temp = tempfile::tempdir().expect("tempdir");
    let store = crate::v2::WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = super::v3_author_checks_tests_a::review_details(
        vec![super::v3_author_checks_tests_a::work_call(
            "implement-task-1",
        )],
        vec![
            super::v3_author_checks_tests_a::review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            super::v3_author_checks_tests_a::review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            super::v3_author_checks_tests_a::review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            super::v3_author_checks_tests_a::review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    super::v3_author_checks_tests_a::save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    // the reducer dropped it; the host merged it back
    super::v3_author_checks_tests_a::save_reduce_record(
        &store,
        "adversarial-review-reduce",
        "adversarial_findings",
        ["adversarial-review-map"],
        serde_json::json!(["cross finding"]),
    );
    super::v3_author_checks_tests_a::save_review_record(
        &store,
        "coverage-audit-map",
        serde_json::json!([]),
    );
    super::v3_author_checks_tests_a::save_reduce_record(
        &store,
        "coverage-audit-reduce",
        "uncovered_requirements",
        ["coverage-audit-map"],
        serde_json::json!([]),
    );
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": ["map finding", "cross finding"],
        "uncovered_requirements": [],
    })
    .to_string();

    super::v3_author_checks_b::validate_review_accounting_from_reducers(
        Some(&accounting),
        &details,
        &store,
    )
    .expect("a restored map finding is preservation, not fabrication");
}

/// Diagnostic harness: replay a finished run through every post-run check.
///
/// Ignored because it needs a run directory on disk. It exists because the three
/// checks below run only at the end of a live run, so each blocker they found
/// cost hours to reach and the run that hit it was gone. This replays them from
/// the recorded artifacts in about a second, and reports EVERY verdict rather
/// than stopping at the first, so one pass lists what is left instead of one
/// blocker per run.
///
/// ARCHON_RUN_DIR=<.archon/workflows/wf-...> ARCHON_RUN_TASK_IDS=TASK-A-010,TASK-A-020
#[tokio::test]
#[ignore = "diagnostic; set ARCHON_RUN_DIR to a finished run directory"]
async fn a_finished_run_passes_the_post_run_checks() {
    let dir = std::path::PathBuf::from(std::env::var("ARCHON_RUN_DIR").expect("ARCHON_RUN_DIR"));
    let expected: std::collections::BTreeSet<String> = std::env::var("ARCHON_RUN_TASK_IDS")
        .expect("ARCHON_RUN_TASK_IDS")
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    let source = std::fs::read_to_string(dir.join("authored-workflow.js"))
        .expect("authored-workflow.js in the run directory");
    let store = crate::v2::WorkflowV2ResultStore::new(dir.join("v2"));
    let script_result = std::fs::read_to_string(dir.join("v2/script-result.json")).ok();

    // The live caller derives the plan from the script, then replaces its calls
    // with the ones the run actually executed.
    let mut details = super::super::dry_run_workflow_plan_full_details(&source, None)
        .await
        .expect("the authored script plans");
    let mut executed: Vec<(String, crate::v2::WorkflowV2HostCall)> = store
        .load_call_records()
        .expect("call records")
        .into_iter()
        .map(|record| (record.finished_at.clone(), record.call))
        .collect();
    executed.sort_by(|left, right| left.0.cmp(&right.0));
    details.calls = executed.into_iter().map(|(_, call)| call).collect();

    let mut verdicts = Vec::new();
    verdicts.push(
        match super::v3_author_checks_a::validate_map_reduce_review_calls(&details, &expected) {
            Ok(()) => "review call contract: PASS".to_string(),
            Err(reason) => format!("review call contract: FAIL — {reason}"),
        },
    );
    verdicts.push(
        match super::super::v3_author_b::validate_authored_task_accounting(
            script_result.as_deref(),
            &expected,
        ) {
            Ok(()) => "task accounting: PASS".to_string(),
            Err(error) => format!("task accounting: FAIL — {error}"),
        },
    );
    verdicts.push(
        match super::v3_author_checks_b::validate_review_accounting_from_reducers(
            script_result.as_deref(),
            &details,
            &store,
        ) {
            Ok(()) => "review accounting: PASS".to_string(),
            Err(error) => format!("review accounting: FAIL — {error}"),
        },
    );
    for verdict in &verdicts {
        println!("{verdict}");
    }
    let failures: Vec<&String> = verdicts.iter().filter(|v| v.contains("FAIL")).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The check is host-against-host: what the script reports must be exactly
/// what the host attached to the final reducer. These pin the two refusals the
/// old containment could not express, and the record shape it now requires.
#[cfg(test)]
mod host_against_host_tests {
    use super::v3_author_checks_b::validate_review_accounting_from_reducers;
    use super::v3_author_checks_tests_a::{
        review_details, review_map_claim, review_reduce, save_reduce_record, save_review_record,
        work_call,
    };
    use crate::v2::WorkflowV2ResultStore;

    fn details() -> super::WorkflowDryRunPlanDetails {
        review_details(
            vec![work_call("implement-task-1")],
            vec![
                review_map_claim(
                    "adversarial_findings",
                    "adversarial-review-map",
                    "TASK-EX-001",
                ),
                review_map_claim(
                    "uncovered_requirements",
                    "coverage-audit-map",
                    "TASK-EX-001",
                ),
            ],
            vec![
                review_reduce(
                    "adversarial_findings",
                    "adversarial-review-reduce",
                    "adversarial_findings",
                    ["adversarial-review-map"],
                    [],
                ),
                review_reduce(
                    "uncovered_requirements",
                    "coverage-audit-reduce",
                    "uncovered_requirements",
                    ["coverage-audit-map"],
                    [],
                ),
            ],
        )
    }

    fn accounting(adversarial: serde_json::Value) -> String {
        serde_json::json!({
            "accepted": ["TASK-EX-001"],
            "blocked": [],
            "adversarial_findings": adversarial,
            "uncovered_requirements": [],
        })
        .to_string()
    }

    /// A script that reports a finding the host never attached is refused --
    /// the containment check let an invented finding through as long as some
    /// reviewer had produced it somewhere.
    #[test]
    fn a_finding_the_host_never_attached_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        save_review_record(
            &store,
            "adversarial-review-map",
            serde_json::json!(["map finding"]),
        );
        save_reduce_record(
            &store,
            "adversarial-review-reduce",
            "adversarial_findings",
            ["adversarial-review-map"],
            serde_json::json!([]),
        );
        save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
        save_reduce_record(
            &store,
            "coverage-audit-reduce",
            "uncovered_requirements",
            ["coverage-audit-map"],
            serde_json::json!([]),
        );

        let error = validate_review_accounting_from_reducers(
            Some(&accounting(serde_json::json!(["map finding", "invented"]))),
            &details(),
            &store,
        )
        .expect_err("an invented finding is refused")
        .to_string();
        assert!(
            error.contains("reports 1 finding(s) the host never attached"),
            "{error}"
        );
    }

    /// A final reducer record without the host's attachment cannot back the
    /// accounting: nothing was computed, so nothing can be compared.
    #[test]
    fn a_reducer_record_without_the_host_attachment_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        save_review_record(&store, "adversarial-review-map", serde_json::json!([]));
        save_review_record(&store, "adversarial-review-reduce", serde_json::json!([]));
        save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
        save_review_record(&store, "coverage-audit-reduce", serde_json::json!([]));

        let error = validate_review_accounting_from_reducers(
            Some(&accounting(serde_json::json!([]))),
            &details(),
            &store,
        )
        .expect_err("no attachment, no accounting")
        .to_string();
        assert!(error.contains("carries no host review findings"), "{error}");
    }
}
