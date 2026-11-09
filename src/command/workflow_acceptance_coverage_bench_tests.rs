//! S0 fixture harness: observe the frozen links without executing checks.

use super::*;
use crate::command::workflow_task_set as task_set;
use archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE;
use archon_workflow::v2::acceptance_stage::coverage::answers_for;

const CONTRACT: &[u8] = include_bytes!("../../tests/fixtures/r18_plan/acceptance-contract.json");
const SKELETON: &[u8] = include_bytes!("../../tests/fixtures/r18_plan/task-skeleton.json");
const FIXTURE_PRD: &str = include_str!("../../tests/fixtures/r18_plan/prd.md");

#[test]
fn fixture_pins_known_572_coverage_gap() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path();
    let tasks = project.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let contract: AcceptanceContract = serde_json::from_slice(CONTRACT).unwrap();
    // Keep the fixture contract and its PRD digest unchanged: materialize
    // the PRD at the relative location the contract already names.
    let relative_prd = Path::new(&contract.prd.path);
    assert!(relative_prd.is_relative());
    assert!(
        relative_prd
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    );
    let prd = project.join(relative_prd);
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(&prd, FIXTURE_PRD).unwrap();
    assert_eq!(content_digest(FIXTURE_PRD.as_bytes()), contract.prd.digest);
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&contract_path, CONTRACT).unwrap();
    let coverage = acceptance_coverage_findings(FIXTURE_PRD, &contract_path, &contract);
    let (lock, pin) = task_set::findings::acceptance_lock_and_pin(
        &tasks,
        FreezeGateMode::Enforce,
        &coverage,
        CONTRACT,
        None.into(),
    );
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    let pin_path = task_set::acceptance_pin_path(project, &tasks);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    let prepared = task_set::prepare_skeleton_freeze_from_candidate(
        project,
        &tasks,
        &prd,
        GateMode::Enforce,
        SKELETON.to_vec(),
    )
    .unwrap();

    // Known #572 gap, pinned by S0: S1 widens coverable ids; S2 adds
    // deliverable-link diagnostics and S4 promotes them to routed findings.
    // Flip these expectations with those steps, never by changing the fixture.
    let task_without_check: Vec<_> = prepared
        .findings
        .iter()
        .chain(&coverage)
        .filter(|finding| {
            finding
                .deterministic_defect
                .as_ref()
                .is_some_and(|defect| defect.code == "task_without_check")
        })
        .collect();
    assert_eq!(task_without_check.len(), 0, "{task_without_check:#?}");
    let non_requirement_covers: Vec<_> = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .flat_map(|entry| {
            entry
                .covers
                .iter()
                .filter(|id| !id.starts_with("REQ-"))
                .map(|id| (&entry.id, id))
        })
        .collect();
    assert_eq!(
        non_requirement_covers.len(),
        0,
        "{non_requirement_covers:#?}"
    );

    let skeleton: TaskSkeleton = serde_json::from_slice(SKELETON).unwrap();
    let unrelated: BTreeSet<_> = skeleton
        .tasks
        .iter()
        .filter(|task| {
            !contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .filter(|entry| answers_for(entry, &task.implements))
                .any(|entry| {
                    let AcceptanceCheck::Command { command, .. } = &entry.check else {
                        panic!("fixture audit requires command checks: {}", entry.id);
                    };
                    // S0's simple read-operand proxy is a literal path match
                    // on executable lines, excluding comment-only mentions.
                    command
                        .lines()
                        .filter(|line| !line.trim_start().starts_with('#'))
                        .any(|line| {
                            task.deliverable_contracts.iter().any(|deliverable| {
                                !deliverable.artifact_path.is_empty()
                                    && line.contains(&deliverable.artifact_path)
                            })
                        })
                })
        })
        .map(|task| task.task_id.as_str())
        .collect();
    println!("#572 S0 tasks with no linked deliverable path: {unrelated:#?}");
    let audited = BTreeSet::from([
        "TASK-TDL-012",
        "TASK-TDL-013",
        "TASK-AHDM-001",
        "TASK-AHDM-002",
        "TASK-AHDM-003",
        "TASK-AHDM-005",
        "TASK-TDL-001",
        "TASK-TDL-011",
        "TASK-TDL-014",
    ]);
    assert!(unrelated.is_superset(&audited), "full set: {unrelated:#?}");
}

#[test]
fn r18_classification_has_no_invented_base_outcomes() {
    use archon_workflow::v2::requirement_classes::{RequirementClass, classify};
    let skeleton: TaskSkeleton = serde_json::from_slice(SKELETON).unwrap();
    let contract: AcceptanceContract = serde_json::from_slice(CONTRACT).unwrap();
    // Classification needs the full base inventory, including non-read deliverables.
    let paths: BTreeSet<_> = include_str!("../../tests/fixtures/r18_plan/base-paths.txt")
        .lines()
        .chain(
            include_str!("../../tests/fixtures/r18_plan/base-non-read-deliverable-paths.txt")
                .lines(),
        )
        .collect();
    let entries = classify(&skeleton, &contract, &Default::default(), |path| {
        (paths.contains(path), false)
    });
    let counts = [
        RequirementClass::NoCheck,
        RequirementClass::Missing,
        RequirementClass::CandidateDone,
        RequirementClass::NoBaseResult,
    ]
    .map(|class| entries.iter().filter(|entry| entry.class == class).count());
    println!("R18 S4a [no_check, missing, candidate_done, no_base_result]={counts:?}");
    assert!(
        entries
            .iter()
            .all(|entry| entry.checks.values().all(Option::is_none))
    );
    // Counts are pinned from the fixture, without synthesizing verdicts.
    assert_eq!(counts, [54, 0, 0, 147]);
}
