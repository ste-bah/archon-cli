use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
use archon_workflow::repository_record::related_paths;
use archon_workflow::task_set_contract::*;
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock, validate_skeleton_set};
use archon_workflow::v2::requirement_classes::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn contract() -> AcceptanceContract {
    serde_json::from_value(json!({
        "schema_version":1,"prd":{"path":"input","digest":"anchor"},
        "gap_policy":{},"supplementary":[],
        "acceptance":[
            {"id":"AC-1","criterion":"one","check":{"kind":"command","command":"verify one","cwd":"repo_root"},"covers":["REQ-1","REQ-2"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"judge"}},
            {"id":"AC-2","criterion":"two","check":{"kind":"command","command":"verify two","cwd":"repo_root"},"covers":["REQ-1"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"judge"}}
        ]
    })).unwrap()
}

fn skeleton() -> TaskSkeleton {
    serde_json::from_value(json!({"schema_version":1,"acceptance_digest":"anchor","tasks":[
        {"task_id":"TASK-A-001","file_name":"TASK-A-001.md","implements":["REQ-1"],"deliverable_contracts":[{"kind":"item","artifact_path":"work/item","min_instances":1,"implements":["REQ-1"],"artifact_format":"text"}]},
        {"task_id":"TASK-A-002","file_name":"TASK-A-002.md","implements":[],"deliverable_contracts":[{"kind":"item","artifact_path":"work","min_instances":1}]}
    ]})).unwrap()
}

fn results(contract: &AcceptanceContract, outcomes: &[(&str, BaselineOutcome)]) -> BaselineResults {
    BaselineResults::observed(
        Some("base".into()),
        &outcomes
            .iter()
            .map(|(id, outcome)| (id.to_string(), *outcome))
            .collect(),
        contract,
    )
}

#[test]
fn classifier_table_covers_every_branch_and_c2() {
    use BaselineOutcome::{Failed, Passed};
    let contract = contract();
    let skeleton = skeleton();
    let cases = [
        (vec![], true, RequirementClass::NoBaseResult, None, None),
        (
            vec![("AC-1", Failed)],
            true,
            RequirementClass::NoBaseResult,
            None,
            None,
        ),
        (
            vec![("AC-1", Failed), ("AC-2", Failed)],
            true,
            RequirementClass::Missing,
            Some("all_checks_fail_on_base"),
            None,
        ),
        (
            vec![("AC-1", Passed)],
            true,
            RequirementClass::CandidateDone,
            None,
            Some(Fallback::Undecided),
        ),
        (
            vec![("AC-1", Passed), ("AC-2", Failed)],
            true,
            RequirementClass::CandidateDone,
            None,
            Some(Fallback::Missing),
        ),
        (
            vec![("AC-1", Passed), ("AC-2", Passed)],
            true,
            RequirementClass::CandidateDone,
            None,
            Some(Fallback::Undecided),
        ),
        (
            vec![],
            false,
            RequirementClass::Missing,
            Some("deliverables_absent_at_base"),
            None,
        ),
        (
            vec![("AC-1", Passed)],
            false,
            RequirementClass::Missing,
            Some("deliverables_absent_at_base"),
            None,
        ),
    ];
    for (outcomes, at_base, class, reason, fallback) in cases {
        let result = classify(
            &skeleton,
            &contract,
            &results(&contract, &outcomes).outcomes,
            |_| (at_base, true),
        )
        .remove(0);
        assert_eq!(
            (result.class, result.reason.as_deref(), result.fallback),
            (class, reason, fallback)
        );
        assert!(result.text_item && result.deliverables[0].text && result.deliverables[0].shared);
        assert_eq!(result.deliverables[0].live_only, !at_base);
        assert_eq!(
            result.passing_check_without_deliverable,
            !at_base && outcomes.iter().any(|(_, o)| *o == Passed)
        );
    }
    let mut no_checks = skeleton.clone();
    no_checks.tasks[0].implements = vec!["REQ-unknown".into()];
    assert_eq!(
        classify(&no_checks, &contract, &BTreeMap::new(), |_| (false, false))[0].class,
        RequirementClass::NoCheck
    );
    // A failed multi-cover check decides MISSING for every covered id.
    let mut multi = contract.clone();
    multi.acceptance.pop();
    no_checks.tasks[0].implements = vec!["REQ-1".into(), "REQ-2".into()];
    no_checks.tasks[0].deliverable_contracts.clear();
    assert!(
        classify(
            &no_checks,
            &multi,
            &results(&multi, &[("AC-1", Failed)]).outcomes,
            |_| (true, true)
        )
        .iter()
        .all(|e| e.class == RequirementClass::Missing)
    );
}

#[test]
fn stale_results_are_ignored_and_text_is_declared_only() {
    let mut contract = contract();
    let mut skeleton = skeleton();
    let results = results(&contract, &[("AC-1", BaselineOutcome::Passed)]);
    contract.acceptance[0].check = AcceptanceCheck::Command {
        command: "changed".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    skeleton.tasks[0].deliverable_contracts[0].artifact_format = None;
    skeleton.tasks[0].deliverable_contracts[0].artifact_path = "report.md".into();
    let entry = classify(&skeleton, &contract, &results.outcomes, |_| (true, true)).remove(0);
    assert_eq!(entry.class, RequirementClass::NoBaseResult);
    assert!(!entry.text_item && !entry.deliverables[0].text && !entry.deliverables[0].shared);
    assert!(related_paths("work", "work/item"));
    assert!(related_paths("work/item", "work"));
    assert!(related_paths("work/item", "work/item"));
    assert!(!related_paths("work/item", "work/items"));
}

#[test]
fn old_lock_fixture_bytes_round_trip_identically() {
    let acceptance = include_bytes!("fixtures/classification/acceptance.lock");
    let skeleton = include_bytes!("fixtures/classification/skeleton.lock");
    let lock: AcceptanceLock = serde_json::from_slice(acceptance).unwrap();
    assert!(lock.baseline_results.is_empty());
    assert_eq!(serde_json::to_vec_pretty(&lock).unwrap(), acceptance);
    let lock: TaskSkeletonLock = serde_json::from_slice(skeleton).unwrap();
    assert!(lock.classification.is_none());
    assert_eq!(serde_json::to_vec_pretty(&lock).unwrap(), skeleton);
}

#[test]
fn carry_forward_requires_same_commit_and_current_check_digest() {
    let mut contract = contract();
    let mut lock: AcceptanceLock =
        serde_json::from_slice(include_bytes!("fixtures/classification/acceptance.lock")).unwrap();
    lock.baseline_results = results(
        &contract,
        &[
            ("AC-1", BaselineOutcome::Passed),
            ("AC-2", BaselineOutcome::Failed),
        ],
    )
    .outcomes;
    assert_eq!(
        BaselineResults::retained(&lock, Some("base".into()), &contract)
            .outcomes
            .len(),
        2
    );
    assert!(
        BaselineResults::retained(&lock, Some("other".into()), &contract)
            .outcomes
            .is_empty()
    );
    assert!(
        BaselineResults::retained(&lock, None, &contract)
            .outcomes
            .is_empty()
    );
    contract.acceptance[0].check = AcceptanceCheck::Command {
        command: "changed".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let retained = BaselineResults::retained(&lock, Some("base".into()), &contract);
    assert_eq!(
        retained.outcomes.keys().cloned().collect::<Vec<_>>(),
        ["AC-2"]
    );
    contract.acceptance.clear();
    assert!(retained.current(&contract).is_empty());
}

#[test]
fn deliverable_claim_rule_uses_task_subject_and_exact_field_and_text() {
    let mut skeleton = skeleton();
    let expected = BTreeSet::from(["REQ-1".into()]);
    assert!(validate_skeleton_set(&skeleton, &expected).is_empty());
    skeleton.tasks[0].deliverable_contracts[0]
        .implements
        .push("REQ-2".into());
    let findings = validate_skeleton_set(&skeleton, &expected);
    let finding = findings
        .iter()
        .find(|f| f.identity.code == "deliverable_implements_unclaimed")
        .unwrap();
    assert_eq!(finding.identity.subject, "TASK-A-001");
    assert_eq!(finding.field, "deliverable_contracts/0/implements/1");
    assert_eq!(finding.identity.location, finding.field);
    assert_eq!(
        finding.message,
        "task 'TASK-A-001' deliverable 'work/item' implements 'REQ-2', which task 'TASK-A-001' does not claim in implements; add the id to the task's implements or remove it from the deliverable"
    );
    skeleton.tasks[0].implements.push("REQ-2".into());
    let findings = validate_skeleton_set(&skeleton, &expected);
    assert!(
        findings
            .iter()
            .any(|f| f.identity.code == "unknown_obligation")
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.identity.code == "deliverable_implements_unclaimed")
    );
}

#[test]
fn populated_deliverable_claims_remain_valid_for_audit_and_floor() {
    use archon_workflow::v2::declarative_floor::{
        DeclarativeFloorFacts, evaluate_declarative_floor,
    };
    let skeleton = skeleton();
    let deliverable = &skeleton.tasks[0].deliverable_contracts[0];
    let value = serde_json::to_value(deliverable).unwrap();
    assert!(archon_workflow::v2::deliverable_contract::contract_defects(&value).is_empty());
    let facts = DeclarativeFloorFacts {
        artifact_present: true,
        artifact_byte_len: 4,
        artifact_json: None,
        registry_json: None,
        instance_count: 1,
        searched_roots: Vec::new(),
    };
    let mut without = deliverable.clone();
    without.implements.clear();
    assert_eq!(
        evaluate_declarative_floor(deliverable, &facts),
        evaluate_declarative_floor(&without, &facts)
    );
}

#[test]
fn classifier_presence_error_does_not_claim_missing_or_candidate_done() {
    let contract = contract();
    for outcome in [BaselineOutcome::Passed, BaselineOutcome::Failed] {
        let entries = classify_checked(
            &skeleton(),
            &contract,
            &results(&contract, &[("AC-1", outcome), ("AC-2", outcome)]).outcomes,
            |_| Err("outside repository".into()),
        );
        assert_eq!(entries[0].class, RequirementClass::NoBaseResult);
        assert_eq!(
            entries[0].reason.as_deref(),
            Some("deliverable_presence_error")
        );
        assert_eq!(
            entries[0].deliverables[0].error.as_deref(),
            Some("deliverable_presence_error")
        );
        assert!(!entries[0].passing_check_without_deliverable);
    }
}

#[test]
fn requirement_classes_and_floor_share_declared_format_normalization() {
    use archon_workflow::v2::declarative_floor::{
        DeclarativeFloorEvaluation, DeclarativeFloorFacts, evaluate_declarative_floor,
    };
    let facts = DeclarativeFloorFacts {
        artifact_present: true,
        artifact_byte_len: 4,
        artifact_json: None,
        registry_json: None,
        instance_count: 1,
        searched_roots: Vec::new(),
    };
    for format in ["text", "Text", " TEXT ", "\tTeXt\n", "json", "", "  "] {
        let mut skeleton = skeleton();
        let deliverable = &mut skeleton.tasks[0].deliverable_contracts[0];
        deliverable.artifact_format = Some(format.into());
        let is_text = format.trim().eq_ignore_ascii_case("text");
        assert_eq!(
            deliverable.normalized_artifact_format().as_deref() == Some("text"),
            is_text
        );
        let floor = evaluate_declarative_floor(deliverable, &facts);
        let entry = classify(&skeleton, &contract(), &BTreeMap::new(), |_| (true, true)).remove(0);
        assert_eq!(entry.text_item, is_text, "{format:?}");
        assert_eq!(entry.deliverables[0].text, is_text, "{format:?}");
        if is_text {
            assert_eq!(floor, DeclarativeFloorEvaluation::Passed);
        }
    }
}
