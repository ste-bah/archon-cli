use super::tests::{TEXT, fixture, request};
use super::*;
use crate::command::workflow::workflow_freeze_cli::skeleton_proof::decisions;
use archon_workflow::requirement_state::{RequirementState, task_status};
use test_client::Client;
#[tokio::test]
async fn corpus_proof_records_done_and_preserves_missing_work() {
    let mut f = fixture("test \"$(sh v*)\" = good");
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_value(serde_json::json!({
        "schema_version":1,"acceptance_digest":f.lock.digest,"tasks":[
            {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-VALUE-001"],"deliverable_contracts":[{"kind":"output","artifact_path":"value","min_instances":1,"implements":["REQ-VALUE-001"]}]},
            {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["missing-id"],"deliverable_contracts":[{"kind":"output","artifact_path":"absent","min_instances":1,"implements":["missing-id"]}],"depends_on":[{"task_id":"TASK-X-001","ordering_only":true}]}
        ]})).unwrap();
    let mut missing = f.contract.acceptance[0].clone();
    missing.id = "missing-check".into();
    missing.covers = vec!["missing-id".into()];
    missing.check = serde_json::from_value(
        serde_json::json!({"kind":"command","command":"test -f absent","cwd":"repo_root"}),
    )
    .unwrap();
    let observed = std::process::Command::new("sh")
        .args(["-c", "test -f absent"])
        .current_dir(f.source.path())
        .status()
        .unwrap();
    assert!(!observed.success());
    f.contract.acceptance.push(missing);
    f.lock.baseline_results = archon_workflow::baseline_results::BaselineResults::observed(
        f.lock.baseline_commit.clone(),
        &[
            (
                "check".into(),
                archon_workflow::baseline_results::BaselineOutcome::Passed,
            ),
            (
                "missing-check".into(),
                archon_workflow::baseline_results::BaselineOutcome::Failed,
            ),
        ]
        .into(),
        &f.contract,
    )
    .outcomes;
    f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
    f.classes.entries = archon_workflow::v2::requirement_classes::classify(
        &skeleton,
        &f.contract,
        &f.lock.baseline_results,
        |path| {
            (
                matches!(
                    f.snapshot().entries.get(path),
                    Some(archon_workflow::base_snapshot::SnapshotEntry::Tracked { .. })
                ),
                false,
            )
        },
    );
    let report = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    assert_eq!(report.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        Some(&report),
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    let key = ("TASK-X-001".into(), "REQ-VALUE-001".into());
    assert_eq!(result.decisions[&key].state, RequirementState::Done);
    let proof = &result.proofs[&key];
    assert!(proof.body_gut_mutant_id.is_some());
    assert!(proof.patch.as_ref().is_some_and(|p| p.contains("value")));
    assert!(proof.output_digest.is_some());
    assert_eq!(
        proof.check_digest,
        f.lock.baseline_results["check"].check_digest
    );
    assert_eq!(
        result.decisions[&("TASK-X-002".into(), "missing-id".into())].state,
        RequirementState::Missing
    );
    result.apply_to(&mut skeleton);
    let cache = report.store.clone().unwrap();
    drop(report);
    std::fs::remove_dir_all(cache).unwrap();
    let tasks = f.source.path().join("tasks");
    std::fs::create_dir(&tasks).unwrap();
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(
        tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        &bytes,
    )
    .unwrap();
    let frozen: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&bytes).unwrap();
    for task in &frozen.tasks {
        let candidate = format!(
            "```yaml\ntask_id: {}\ntitle: item\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
            task.task_id
        );
        let (body, _) = crate::command::workflow_host_command_binding::stamp_frozen_frontmatter(
            &candidate, task,
        )
        .unwrap();
        if task.task_id == "TASK-X-002" {
            assert!(body.contains("to implement: missing-id"));
        }
        std::fs::write(tasks.join(&task.file_name), body).unwrap();
    }
    let universe = archon_workflow::task_universe::extract_task_universe_for_generated_run(
        &format!("Implement the decomposed PRD at {}", tasks.display()),
    )
    .unwrap()
    .unwrap();
    let done = universe
        .tasks
        .iter()
        .find(|t| t.canonical_task_id == "TASK-X-001")
        .unwrap();
    let missing = universe
        .tasks
        .iter()
        .find(|t| t.canonical_task_id == "TASK-X-002")
        .unwrap();
    assert!(done.declared_status_is_complete());
    assert!(!missing.declared_status_is_complete());
    assert_eq!(missing.requirement_states["missing-id"], "missing");
    assert!(missing.dependency_ids.contains(&done.canonical_task_id));
}
#[tokio::test]
async fn corpus_survivors_are_kept_with_literal_patches() {
    let f = fixture("test -d .");
    let report = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        Some(&report),
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    assert_eq!(
        result.decisions.values().next().unwrap().reason,
        "weak_test"
    );
    assert!(!result.undecided[0].survivors.is_empty());
    assert!(result.undecided[0].survivors[0].contains("--- a/value"));
    let states = result
        .decisions
        .into_iter()
        .map(|((_, id), d)| (id, d))
        .collect();
    assert_eq!(task_status(&states).as_deref(), Some("kept"));
}
#[tokio::test]
async fn resumable_budget_uses_the_same_corpus_mutants() {
    let f = fixture("test \"$(sh v*)\" = good");
    let client = Client::default();
    let first = prove_requirements(&f.probe(true), &client, request(&f, 3)).await;
    assert_eq!(first.verdicts.entries[0].verdict, Verdict::BudgetExhausted);
    let ids: Vec<_> = first.verdicts.entries[0]
        .mutants
        .iter()
        .map(|m| m.mutant_id.clone())
        .collect();
    let calls = client.calls.lock().unwrap().len();
    let mut last = first;
    for _ in 0..12 {
        last = prove_requirements(&f.probe(true), &client, request(&f, 3)).await;
        assert_eq!(
            last.verdicts.entries[0]
                .mutants
                .iter()
                .map(|m| m.mutant_id.clone())
                .collect::<Vec<_>>(),
            ids
        );
        if last.verdicts.entries[0].verdict == Verdict::DoneCandidate {
            break;
        }
    }
    assert_eq!(last.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    assert_eq!(client.calls.lock().unwrap().len(), calls);
}
#[test]
fn disabled_proof_and_unobserved_results_never_mark_done() {
    let mut f = fixture("test -d .");
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        true,
        None,
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    assert_eq!(
        result.decisions.values().next().unwrap().reason,
        "proof_disabled"
    );
    f.classes.entries[0].class =
        archon_workflow::v2::requirement_classes::RequirementClass::NoBaseResult;
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        None,
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    assert_eq!(
        result.decisions.values().next().unwrap().reason,
        "no_base_result"
    );
}

#[test]
fn text_evidence_requires_all_quoted_literals_and_declared_text() {
    for (command, text_item, expected) in [
        (
            "test 'It keeps the good' = 'It keeps the good'",
            true,
            false,
        ),
        (
            "test 'unrelated literal' = 'unrelated literal'",
            true,
            false,
        ),
        ("test good = good", true, false),
        ("grep -F 'good' value", true, true),
        (
            "test 'It keeps the good' = 'It keeps the good'",
            false,
            false,
        ),
    ] {
        let mut f = fixture(command);
        assert!(
            std::process::Command::new("sh")
                .args(["-c", command])
                .current_dir(f.source.path())
                .status()
                .unwrap()
                .success()
        );
        f.classes.entries[0].text_item = text_item;
        let result = decisions(
            &f.classes,
            &f.contract,
            &f.lock,
            TEXT,
            false,
            None,
            (f.source.path(), f.source.path()),
        )
        .unwrap();
        assert_eq!(
            result.decisions.values().next().unwrap().state == RequirementState::Done,
            expected,
            "{command}"
        );
        if expected {
            assert!(!result.proofs.values().next().unwrap().literals.is_empty());
        }
        if command == "test 'It keeps the good' = 'It keeps the good'" && text_item {
            assert_eq!(
                result.decisions.values().next().unwrap().reason,
                "check_dependency_unproven"
            );
            assert!(result.proofs.is_empty());
        }
    }
}

#[tokio::test]
async fn a_base_failing_pair_with_lost_done_evidence_remains_missing() {
    use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
    let mut f = fixture("test \"$(sh v*)\" = good");
    let mut failing = f.contract.acceptance[0].clone();
    failing.id = "failing-check".into();
    failing.check = serde_json::from_value(
        serde_json::json!({"kind":"command","command":"test -f absent","cwd":"repo_root"}),
    )
    .unwrap();
    let status = std::process::Command::new("sh")
        .args(["-c", "test -f absent"])
        .current_dir(f.source.path())
        .status()
        .unwrap();
    assert!(!status.success());
    f.contract.acceptance.push(failing);
    f.lock.baseline_results = BaselineResults::observed(
        f.lock.baseline_commit.clone(),
        &[
            ("check".into(), BaselineOutcome::Passed),
            ("failing-check".into(), BaselineOutcome::Failed),
        ]
        .into(),
        &f.contract,
    )
    .outcomes;
    f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
    f.classes.entries[0]
        .checks
        .insert("failing-check".into(), Some(BaselineOutcome::Failed));
    f.classes.entries[0].fallback =
        Some(archon_workflow::v2::requirement_classes::Fallback::Missing);
    let mut report = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    assert_eq!(report.verdicts.entries[0].verdict, Verdict::DoneCandidate);
    let proven = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        Some(&report),
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    assert_eq!(
        proven.decisions.values().next().unwrap().state,
        RequirementState::Done
    );
    report.runs.clear();
    let lost = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        Some(&report),
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    let decision = lost.decisions.values().next().unwrap();
    assert_eq!(decision.state, RequirementState::Missing);
    assert_eq!(decision.reason, "base_failing_check");
    assert!(lost.proofs.is_empty());
    assert_eq!(task_status(&[("id".into(), decision.clone())].into()), None);
}

#[tokio::test]
async fn text_read_with_unconditional_success_never_overrides_surviving_mutants() {
    let mut f = fixture("grep -q 'good' value || test -d .");
    f.classes.entries[0].text_item = true;
    let report = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    let pair = &report.verdicts.entries[0];
    assert_eq!(
        pair.verdict,
        Verdict::WeakTest,
        "pair={pair:?}; runs={:?}; refusal={:?}",
        report.runs,
        report.refusal
    );
    assert!(!pair.survivors.is_empty());
    assert!(
        report.runs.len() >= 3,
        "two behaviour mutants and a body gut must run"
    );
    assert!(report.runs.iter().all(|run| !run.counted
        && matches!(
            run.outcome,
            archon_workflow::breakage_proof::RunOutcome::Survived { .. }
        )));
    assert!(
        report
            .runs
            .iter()
            .any(|run| run.kind == MutantKind::BodyGut)
    );
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        Some(&report),
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    let decision = result.decisions.values().next().unwrap();
    assert_eq!(decision.state, RequirementState::Undecided);
    assert_eq!(decision.reason, "weak_test");
    assert!(result.proofs.is_empty());
    // The literal fixed-success fallback is rejected before execution.
    let mut exact = fixture("grep -q 'good' value || true");
    exact.classes.entries[0].text_item = true;
    let blocked =
        prove_requirements(&exact.probe(true), &Client::default(), request(&exact, 50)).await;
    assert_eq!(blocked.verdicts.entries[0].verdict, Verdict::HostUnrun);
    assert!(blocked.runs.iter().all(|r| matches!(&r.outcome, archon_workflow::breakage_proof::RunOutcome::Unrun { detail, .. } if detail.contains("host policy defect"))));
    let result = decisions(
        &exact.classes,
        &exact.contract,
        &exact.lock,
        TEXT,
        false,
        Some(&blocked),
        (exact.source.path(), exact.source.path()),
    )
    .unwrap();
    assert_eq!(
        result.decisions.values().next().unwrap().state,
        RequirementState::Undecided
    );
    assert!(result.proofs.is_empty());
}

#[tokio::test]
async fn classification_faults_are_listed_without_reclassifying_other_pairs() {
    let mut f = fixture("test \"$(sh v*)\" = good");
    let mut broken = f.classes.entries[0].clone();
    broken.id = "unavailable".into();
    broken.deliverables[0].error = Some("snapshot entry missing: unavailable".into());
    broken.class = archon_workflow::v2::requirement_classes::RequirementClass::NoBaseResult;
    f.classes.entries.push(broken);
    let report = prove_requirements(&f.probe(true), &Client::default(), request(&f, 50)).await;
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        Some(&report),
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    assert_eq!(
        result.decisions[&("task".into(), "REQ-VALUE-001".into())].reason,
        "breakage_kill"
    );
    let d = &result.decisions[&("task".into(), "unavailable".into())];
    assert_eq!(d.reason, "classification_unavailable");
    assert_eq!(d.notes, ["snapshot entry missing: unavailable"]);
    f.classes.error = Some("repository_record_absent".into());
    let result = decisions(
        &f.classes,
        &f.contract,
        &f.lock,
        TEXT,
        false,
        None,
        (f.source.path(), f.source.path()),
    )
    .unwrap();
    assert_eq!(result.undecided.len(), 2);
    assert!(
        result
            .decisions
            .values()
            .all(|d| d.state == RequirementState::Undecided)
    );
    assert!(
        result
            .undecided
            .iter()
            .any(|d| d.reason == "classification_unavailable: repository_record_absent")
    );
}
