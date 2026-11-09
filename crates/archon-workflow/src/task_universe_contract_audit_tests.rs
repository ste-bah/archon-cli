//! The audit's two claims, tested on the shapes that motivated it.
//!
//! The discriminating pair matters most: the task that writes the validator and
//! the task that runs the ingest declare the SAME templated contract, and the
//! audit must fire on one and stay silent on the other. A rule that fires on
//! both is a rule that tells an author to move a contract to a task it would
//! also reject.

use super::*;
use crate::task_universe::WorkflowV2DeliverableContract;

const INSTANCE_PATH: &str =
    ".archon/trading-lab/data/datasets/<DATASET_ID>/<VERSION>/validation.json";

fn concrete(path: &str) -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "create".to_string(),
        artifact_path: path.to_string(),
        typed_verifier_command: Some("grep -q required {artifact_path}".into()),
        ..Default::default()
    }
}

fn instances(path: &str, min: usize) -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "per_dataset_version_validation_report".to_string(),
        artifact_path: path.to_string(),
        min_instances: min,
        ..Default::default()
    }
}

fn task(id: &str, contracts: Vec<WorkflowV2DeliverableContract>) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        requirement_states: Default::default(),
        canonical_task_id: id.to_string(),
        // A path that does not exist: `shell_token_paths` returns empty rather
        // than failing, which is what keeps these tests about the contracts.
        source_path: format!("/nonexistent/{id}.md"),
        deliverable_contracts: contracts,
        ..Default::default()
    }
}

fn universe(tasks: Vec<WorkflowV2TaskUniverseTask>) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks,
    }
}

fn kinds(findings: &[ContractFinding], id: &str) -> Vec<ContractFindingKind> {
    findings
        .iter()
        .filter(|finding| finding.task_id == id)
        .map(|finding| finding.kind)
        .collect()
}

/// The live defect: a task that writes source files and nothing else, claiming
/// instances in a data tree it never touches.
#[test]
fn a_task_with_no_footing_in_the_instance_tree_is_flagged() {
    let validator = task(
        "TASK-VAL",
        vec![
            concrete("crates/pkg/src/validation.rs"),
            concrete("crates/pkg/src/validation_tests.rs"),
            instances(INSTANCE_PATH, 1),
        ],
    );
    let findings = audit_contracts(&universe(vec![validator]));
    assert_eq!(
        kinds(&findings, "TASK-VAL"),
        vec![ContractFindingKind::Misallocated]
    );
    assert!(
        findings[0]
            .message
            .contains(".archon/trading-lab/data/datasets/"),
        "the finding must name the tree it has no footing in: {}",
        findings[0].message
    );
}

/// The other half of the pair, and the one that would make the rule useless if
/// it fired: the ingest task declares a concrete artifact in the same data
/// tree, so it plausibly runs there.
#[test]
fn a_task_with_a_concrete_artifact_in_the_same_tree_is_not_flagged() {
    let ingest = task(
        "TASK-INGEST",
        vec![
            concrete("crates/pkg/src/ingest.rs"),
            concrete(".archon/trading-lab/data/registry.json"),
            instances(INSTANCE_PATH, 1),
        ],
    );
    let findings = audit_contracts(&universe(vec![ingest]));
    assert!(
        kinds(&findings, "TASK-INGEST").is_empty(),
        "the task that runs against the data tree must not be flagged: {findings:?}"
    );
}

#[test]
fn a_declared_instance_source_outranks_the_heuristic() {
    let mut contract = instances(INSTANCE_PATH, 1);
    contract.instance_source_path = Some(".archon/trading-lab/data/registry.json".to_string());
    contract.instance_artifact_field = Some("validation_path".to_string());
    contract.instance_source_records_field = Some("datasets".to_string());
    let findings = audit_contracts(&universe(vec![task("TASK-BOUND", vec![contract])]));
    assert!(
        kinds(&findings, "TASK-BOUND").is_empty(),
        "a task that named its instance source has made a stronger claim: {findings:?}"
    );
}

#[test]
fn a_concrete_contract_is_never_second_guessed() {
    let findings = audit_contracts(&universe(vec![task(
        "TASK-PLAIN",
        vec![concrete("crates/pkg/src/lib.rs")],
    )]));
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn a_templated_path_with_no_instance_floor_is_left_to_the_gate() {
    // `min_instances: 0` is its own defect and the runtime predicate owns it;
    // the heuristic must not pile a second opinion on top.
    let findings = audit_contracts(&universe(vec![task(
        "TASK-ZERO",
        vec![instances(INSTANCE_PATH, 0)],
    )]));
    assert!(
        !kinds(&findings, "TASK-ZERO").contains(&ContractFindingKind::Misallocated),
        "{findings:?}"
    );
}

/// The audit asks the runtime's own predicate, so anything it calls
/// unsatisfiable is something the gate will refuse.
#[test]
fn an_unsatisfiable_contract_is_reported_as_certain() {
    let mut contract = instances(INSTANCE_PATH, 1);
    contract.required_universe = true;
    let findings = audit_contracts(&universe(vec![task("TASK-UNIV", vec![contract])]));
    let unsatisfiable: Vec<_> = findings
        .iter()
        .filter(|finding| finding.kind == ContractFindingKind::Unsatisfiable)
        .collect();
    assert_eq!(unsatisfiable.len(), 1, "{findings:?}");
    assert!(ContractFindingKind::Unsatisfiable.is_certain());
    assert!(!ContractFindingKind::Misallocated.is_certain());
    assert!(!ContractFindingKind::RepairedAtLoad.is_certain());
}

/// One contract yields one defect: the second is usually the first restated.
#[test]
fn an_unsatisfiable_contract_does_not_also_get_the_heuristic() {
    let mut contract = instances(INSTANCE_PATH, 1);
    contract.required_universe = true;
    let findings = audit_contracts(&universe(vec![task("TASK-UNIV", vec![contract])]));
    assert!(
        !kinds(&findings, "TASK-UNIV").contains(&ContractFindingKind::Misallocated),
        "{findings:?}"
    );
}

#[test]
fn ancestors_run_longest_first_and_terminate() {
    assert_eq!(
        ancestor_prefixes(".archon/trading-lab/data/datasets/"),
        vec![
            ".archon/trading-lab/data/datasets/".to_string(),
            ".archon/trading-lab/data/".to_string(),
            ".archon/trading-lab/".to_string(),
            ".archon/".to_string(),
        ]
    );
    assert_eq!(ancestor_prefixes(""), Vec::<String>::new());
    assert_eq!(ancestor_prefixes("/"), Vec::<String>::new());
}

#[test]
fn the_instance_root_is_everything_before_the_first_token() {
    assert_eq!(
        instance_root(INSTANCE_PATH),
        ".archon/trading-lab/data/datasets/"
    );
    assert_eq!(instance_root("a/b.json"), "a/b.json");
}

/// The shell-token check reads the file, because the parser repairs the value
/// on the way in and the parsed contract is already correct by then.
#[test]
fn the_shell_token_check_reads_the_source_file_not_the_parsed_contract() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("TASK-RAW.md");
    std::fs::write(
        &path,
        "```yaml\ndeliverable_contracts:\n  - kind: report\n    artifact_path: '${PROJECT_ROOT}/out/${ID}.json'\n    min_instances: 1\n```\n",
    )
    .unwrap();

    let mut raw_task = task("TASK-RAW", vec![concrete("crates/pkg/src/lib.rs")]);
    raw_task.source_path = path.display().to_string();
    let findings = audit_contracts(&universe(vec![raw_task]));
    let repaired: Vec<_> = findings
        .iter()
        .filter(|finding| finding.kind == ContractFindingKind::RepairedAtLoad)
        .collect();
    assert_eq!(repaired.len(), 1, "{findings:?}");
    assert_eq!(repaired[0].artifact_path, "${PROJECT_ROOT}/out/${ID}.json");
}

#[test]
fn an_unreadable_source_file_costs_the_shell_check_not_the_audit() {
    let findings = audit_contracts(&universe(vec![task(
        "TASK-GONE",
        vec![concrete("crates/pkg/src/lib.rs")],
    )]));
    assert!(findings.is_empty(), "{findings:?}");
}

fn verified_contract(
    artifact_path: &str,
    verifier: Option<&str>,
    min_instances: usize,
) -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "report".into(),
        artifact_path: artifact_path.into(),
        typed_verifier_command: verifier.map(str::to_string),
        min_instances,
        ..Default::default()
    }
}

#[test]
fn verifier_strength_uses_one_runtime_predicate_for_bad_and_good_pairs() {
    for (bad, good) in [
        (
            verified_contract("out.json", Some("test -f out.json"), 0),
            verified_contract("out.json", Some("grep -q required out.json"), 0),
        ),
        (
            verified_contract("out.json", Some("bash -c 'test -f out.json'"), 0),
            verified_contract("out.json", Some("jq -e '.ready == true' out.json"), 0),
        ),
        (
            verified_contract("out.json", Some("find . | wc -l"), 0),
            verified_contract("out.json", Some("grep -q required out.json"), 0),
        ),
        (
            verified_contract("out.json", Some("true"), 0),
            verified_contract("out.json", Some("grep -q required out.json"), 0),
        ),
    ] {
        let findings = audit_contracts(&universe(vec![task("TASK-X-010", vec![bad])]));
        let certain: Vec<_> = findings
            .iter()
            .filter(|finding| finding.kind == ContractFindingKind::Unsatisfiable)
            .collect();
        assert_eq!(certain.len(), 1, "{findings:?}");
        assert!(certain[0].message.contains("replace"), "{:?}", certain[0]);
        assert!(
            certain[0]
                .message
                .contains("deleting the verifier does not satisfy this contract"),
            "{:?}",
            certain[0]
        );

        assert!(!certain[0].message.contains("add a falsifiable"));
        let good_findings = audit_contracts(&universe(vec![task("TASK-X-010", vec![good])]));
        assert!(
            !good_findings
                .iter()
                .any(|finding| finding.kind == ContractFindingKind::Unsatisfiable),
            "{good_findings:?}"
        );
    }
}

#[test]
fn deleting_a_verifier_without_a_positive_floor_is_still_unsatisfiable() {
    let missing = verified_contract("out.json", None, 0);
    let findings = audit_contracts(&universe(vec![task("TASK-X-010", vec![missing])]));
    let certain = findings
        .iter()
        .find(|finding| finding.kind == ContractFindingKind::Unsatisfiable)
        .expect("missing verifier without floor is certain");
    assert!(
        certain
            .message
            .contains("neither a verifier nor a positive instance obligation")
    );
    assert!(
        certain
            .message
            .contains("set min_instances >= 1 (preferred for one concrete artifact path), add a falsifiable typed_verifier_command")
    );

    assert!(!certain.message.contains("replace"));
    assert!(!certain.message.contains("deleting"));
    for field in [
        "instance_artifact_field",
        "instance_source_path",
        "registry_path",
        "instance_source_records_field",
        "registry_records_field",
    ] {
        assert!(certain.message.contains(field), "{}", certain.message);
    }
    let floor = verified_contract("out.json", None, 1);
    let floor_findings = audit_contracts(&universe(vec![task("TASK-X-010", vec![floor])]));
    assert!(
        !floor_findings
            .iter()
            .any(|finding| finding.kind == ContractFindingKind::Unsatisfiable),
        "{floor_findings:?}"
    );
}

#[test]
fn skeleton_and_body_share_every_contract_refusal() {
    use crate::task_skeleton::{FrozenTask, TaskSkeleton, skeleton_defects, validate_skeleton};
    let mut bound = instances("output/<ID>/report.json", 0);
    bound.instance_source_path = Some("input/records.json".into());
    bound.instance_source_records_field = Some("records".into());
    bound.instance_artifact_field = Some("report_path".into());
    let mut floor = concrete("output/report.json");
    floor.typed_verifier_command = None;
    floor.min_instances = 1;
    let mut contracts = vec![concrete("output/report.json"), floor, bound];
    for verifier in [None, Some("test -f output/report.json"), Some("true")] {
        let mut contract = concrete("output/report.json");
        contract.typed_verifier_command = verifier.map(str::to_string);
        contracts.push(contract);
    }
    for (index, contract) in contracts.into_iter().enumerate() {
        let skeleton = TaskSkeleton {
            schema_version: 1,
            acceptance_digest: "digest".into(),
            tasks: vec![FrozenTask {
                requirement_states: Default::default(),
                proven_done: Default::default(),
                status: None,
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010.md".into(),
                depends_on: vec![],
                blocks: vec![],
                implements: vec![],
                source_sections: vec![],
                deliverable_contracts: vec![contract.clone()],
            }],
        };
        let defects = skeleton_defects(&skeleton, "digest");
        let body = audit_contracts(&universe(vec![task("TASK-X-010", vec![contract])]));
        let certain: Vec<_> = body.iter().filter(|f| f.kind.is_certain()).collect();
        assert_eq!(
            defects.len(),
            certain.len(),
            "case {index}: {defects:?} {body:?}"
        );
        assert_eq!(validate_skeleton(&skeleton, "digest").is_ok(), index < 3);
        if index >= 3 {
            assert_eq!(defects.len(), 1);
            assert_eq!(Some(&defects[0].identity), certain[0].identity.as_ref());
            assert_eq!(defects[0].identity.subject, "TASK-X-010");
            assert_eq!(
                defects[0].identity.location,
                "deliverable_contracts/0/typed_verifier_command"
            );
            assert!(
                defects[0]
                    .message
                    .contains("task-skeleton.json task TASK-X-010")
            );
            assert!(defects[0].message.contains("min_instances"));
        }
    }
}

#[test]
fn contract_remedies_follow_verifier_precedence_and_existing_obligations() {
    use crate::verifier_strength::verifier_strength_defect;
    for command in ["test -f out.json", "true", "grep -q ready out.json || true"] {
        for binding in [false, true] {
            for floor in [0, 1] {
                let mut contract = verified_contract("out.json", Some(command), floor);
                if binding {
                    contract.instance_artifact_field = Some("path".into());
                    contract.registry_path = Some("registry.json".into());
                    contract.registry_records_field = Some("rows".into());
                }
                assert!(
                    verifier_strength_defect(Some(command), Some("out.json"), Some(&contract))
                        .is_some()
                );
                let findings = contract_defects("TASK-X-010", 2, &contract);
                let message = &findings.last().unwrap().message;
                assert!(
                    message.contains("replace the verifier with a command that can exit non-zero"),
                    "{message}"
                );
                assert_eq!(
                    message.contains("or remove it"),
                    floor > 0 || binding,
                    "{message}"
                );
                assert_eq!(
                    message.contains("deleting the verifier does not satisfy"),
                    floor == 0 && !binding,
                    "{message}"
                );
                contract.typed_verifier_command = None;
                assert_eq!(
                    contract_defects("TASK-X-010", 2, &contract).is_empty(),
                    floor > 0 || binding
                );
            }
        }
    }
}

#[test]
fn every_optional_string_rejects_blank_and_template_values_at_its_index() {
    let fields: Vec<_> = include_str!("task_universe_deliverable.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub "))
        .filter_map(|line| line.split_once(": Option<String>"))
        .map(|(field, _)| field)
        .collect();
    assert!(fields.contains(&"typed_verifier_command"));
    for field in fields {
        for invalid in ["", "  ", "<optional: concrete value>", "${UNREPLACED}"] {
            let mut value = serde_json::to_value(verified_contract("out.json", None, 1)).unwrap();
            value[field] = serde_json::json!(invalid);
            let contract = serde_json::from_value(value).unwrap();
            let defects = contract_defects("TASK-X-010", 3, &contract);
            assert!(
                defects
                    .iter()
                    .any(|defect| defect.identity.location
                        == format!("deliverable_contracts/3/{field}")),
                "{field}={invalid:?}: {defects:?}"
            );
        }
    }
}

#[test]
fn prompt_keeps_the_exact_contract_remedy_field_grouping() {
    let source = include_str!("../../../src/command/workflow_decompose_v1.js");
    assert!(DELIVERABLE_CONTRACT_RULE.contains(INSTANCE_SOURCE_FIELDS));
    assert!(source.contains("return args.deliverableContractRule;"));
}
