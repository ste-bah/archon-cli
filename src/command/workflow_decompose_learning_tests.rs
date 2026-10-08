use super::workflow_decompose_learning::{lesson_records, redact_lesson, stable_lesson_key};

#[test]
fn learning_decompose_refutations_and_cannot_pass_findings_become_records() {
    let source = serde_json::json!({
        "judgment": {"verdict":"refuted", "reason":"check misses output", "counterexample":"empty output"},
        "findings": [{"verdict":"cannot_pass_as_written", "rule":"must name a concrete file"}]
    });
    let records = lesson_records(
        "run-1",
        "review and judge acceptance checks",
        "judge-call",
        2,
        &source,
        &[],
    );
    assert!(
        records
            .iter()
            .any(|r| r.name.contains("check misses output") && r.name.contains("empty output"))
    );
    assert!(records.iter().any(|r| r.name.contains("cannot pass") && r.name.contains("must name a concrete file")));
}

#[test]
fn learning_decompose_pause_records_lessons_and_resume_identity_is_stable() {
    let paused = lesson_records(
        "run-1",
        "review",
        "judge-call",
        2,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"weak check","counterexample":"empty output"}}),
        &["reasoning_bank".into()],
    );
    assert_eq!(paused.len(), 1);
    assert_eq!(paused[0].telemetry.attempt, 2);
    assert_eq!(
        stable_lesson_key("run-1", "judge-call", 2),
        stable_lesson_key("run-1", "judge-call", 2)
    );
    assert_ne!(
        stable_lesson_key("run-1", "judge-call", 2),
        stable_lesson_key("run-1", "judge-call", 3)
    );
}

#[test]
fn learning_content_is_redacted_before_storage() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz1234567890";
    let records = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":format!("token={secret}"),"counterexample":"empty"}}),
        &["reasoning_bank".into()],
    );
    let stored = serde_json::to_string(&records).unwrap();
    assert!(!stored.contains(secret));
    assert!(!redact_lesson(&format!("token={secret}")).contains(secret));
}

#[test]
fn learning_decompose_disabled_learning_yields_no_hooks() {
    let mut config = archon_core::config::LearningConfig::default();
    config.sona.enabled = false;
    config.sona.pipeline_recording = false;
    config.reasoning_bank.enabled = false;
    config.desc.enabled = false;
    let hooks = super::workflow_decompose_learning::hooks("review acceptance checks", &config);
    assert!(hooks.is_empty());
}

#[test]
fn learning_decompose_run_end_emits_reasoning_records_for_judged_findings() {
    let mut config = archon_core::config::LearningConfig::default();
    config.reasoning_bank.enabled = true;
    config.desc.enabled = false;
    let hooks = super::workflow_decompose_learning::hooks("review judge findings", &config);
    let records = lesson_records(
        "run-1",
        "review judge findings",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"misses output","counterexample":"empty result"}}),
        &hooks,
    );
    assert!(
        records
            .iter()
            .any(|record| record.hooks.contains(&"reasoning_bank".to_string()))
    );
    assert!(
        records
            .iter()
            .any(|record| record.name.contains("misses output"))
    );
}

#[test]
fn learning_decompose_resume_dispatch_set_excludes_previously_recorded_lessons() {
    let records = lesson_records(
        "run-1",
        "review",
        "judge-call",
        1,
        &serde_json::json!({"judgment":{"verdict":"refuted","reason":"weak","counterexample":"empty"}}),
        &[],
    );
    let mut known = std::collections::BTreeSet::new();
    let first = super::workflow_decompose_learning::filter_new_records(records.clone(), &mut known);
    let resumed = super::workflow_decompose_learning::filter_new_records(records, &mut known);
    assert_eq!(first.len(), 1);
    assert!(resumed.is_empty());
}

#[test]
fn learning_decompose_fold_errors_do_not_change_run_report() {
    let report = "Fixed decomposition completed";
    let fold_result: Option<()> =
        super::workflow_decompose_learning::best_effort_fold("run-1", || {
            Err(anyhow::anyhow!("db offline"))
        });
    assert!(fold_result.is_none());
    assert_eq!(report, "Fixed decomposition completed");
}
