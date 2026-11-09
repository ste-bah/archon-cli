//! ACC-A7 / M4: an extension's gate holds the coverage a whole-set freeze
//! holds -- every requirement of the PRD as it is now covered by a check.

use super::*;
use crate::command::workflow_task_set::republish::test_fixture::{
    NO_SEEDS, criterion, frozen_set_in,
};

#[test]
fn an_extension_cannot_match_when_logic_identity_is_unavailable() {
    let stored: Option<&serde_json::Value> = None;
    let current: Option<serde_json::Value> = None;
    assert!(
        !logic_identity_matches(stored, current.as_ref()),
        "identity comparison must require a current identity"
    );
}

#[test]
fn an_extension_that_leaves_a_requirement_uncovered_is_refused_by_its_gate() {
    let set = frozen_set_in(
        &[("AC-F-001", "test -f present", true)],
        FreezeGateMode::Enforce,
        "- REQ-F-001: the store keeps raw responses.\n- REQ-F-002: the store is append-only.\n",
    );
    let before = set.chain_bytes();
    let frozen = set.contract();
    let mut sup = criterion("SUP-REQ-F-001", "test -f present", true);
    sup.covers = vec!["REQ-F-001".into()];
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut sup,
        crate::command::workflow_task_set::republish::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        &super::judge::JudgeRoots {
            covered_texts: Default::default(),
            project: Some(set.project.path().to_path_buf()),
            repository: None,
            task_root: Some(set.tasks.clone()),
            repository_commit: None,
        },
    );
    let extension = Extension {
        entries: vec![sup],
        prd_digest: frozen.prd.digest.clone(),
        frozen_prd_digest: frozen.prd.digest.clone(),
        // Both additions still require covering checks.
        new_obligations: ["REQ-F-001".to_string(), "REQ-F-002".to_string()].into(),
    };
    let ids: BTreeSet<String> = ["SUP-REQ-F-001".to_string()].into();
    let attempt = |seeds| {
        extend_and_republish(
            ReauthorRequest {
                project_root: set.project.path(),
                tasks_root: &set.tasks,
                prd_path: &set.prd,
                ids: &ids,
                gate: ReauthorGate {
                    probe: &set.probe,
                    seeds,
                },
                trigger: "test extension",
            },
            &extension,
        )
        .expect_err("REQ-F-002 is still covered by no check")
    };
    let error = attempt(&NO_SEEDS);
    let unused = BTreeMap::from([("SUP-REQ-F-001".into(), "must never reach an author".into())]);
    assert_eq!(
        format!("{error:#}"),
        format!("{:#}", attempt(&unused)),
        "the extension publisher does not consume author seeds"
    );
    assert!(format!("{error:#}").contains("REQ-F-002"), "{error:#}");
    assert_eq!(set.chain_bytes(), before, "nothing published");
}

#[test]
fn new_obligations_are_claimed_only_by_covering_check_or_artifact_owners() {
    let extension = Extension {
        entries: vec![],
        prd_digest: "d".into(),
        frozen_prd_digest: "d".into(),
        new_obligations: ["new".into()].into(),
    };
    let mut check = criterion("AC-F-001", "test -f present", true);
    check.covers = vec!["new".into()];
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"input","digest":"d"},"gap_policy":{},"acceptance":[check],"supplementary":[]})).unwrap();
    let mut s: TaskSkeleton = serde_json::from_value(serde_json::json!({"schema_version":1,"acceptance_digest":"d","tasks":[
        {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["AC-F-001"]},
        {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["old"]},
        {"task_id":"TASK-X-003","file_name":"TASK-X-003.md","implements":["old"],"deliverable_contracts":[{"kind":"output","artifact_path":"item","min_instances":1,"implements":["new"]}]}
    ]})).unwrap();
    let other = s.tasks[1].clone();
    extension.claim_new_obligations(&mut s, &contract);
    assert!(s.tasks[0].implements.contains(&"new".into()));
    assert_eq!(s.tasks[1], other);
    assert!(s.tasks[2].implements.contains(&"new".into()));
}

#[test]
fn republish_new_unowned_obligation_succeeds_and_routes_skeleton_repair() {
    let set = frozen_set_in(
        &[("AC-F-001", "test -f present", true)],
        FreezeGateMode::Enforce,
        "",
    );
    let frozen = set.contract();
    let mut text = std::fs::read_to_string(&set.prd).unwrap();
    text.push_str("\n- REQ-F-003: the store keeps raw responses.\n");
    std::fs::write(&set.prd, &text).unwrap();
    let mut sup = criterion("SUP-REQ-F-003", "test -f present", true);
    sup.covers = vec!["REQ-F-003".into()];
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut sup,
        crate::command::workflow_task_set::republish::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        &super::judge::JudgeRoots {
            covered_texts: Default::default(),
            project: Some(set.project.path().to_path_buf()),
            repository: None,
            task_root: Some(set.tasks.clone()),
            repository_commit: None,
        },
    );
    let extension = Extension {
        entries: vec![sup],
        prd_digest: content_digest(text.as_bytes()),
        frozen_prd_digest: frozen.prd.digest.clone(),
        new_obligations: ["REQ-F-003".into()].into(),
    };
    let ids = ["SUP-REQ-F-003".into()].into();
    let result = extend_and_republish(
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &ids,
            gate: set.gate(),
            trigger: "test extension",
        },
        &extension,
    )
    .unwrap();
    assert!(result.published);
    let skeleton = validate_full_chain(&set.tasks, &set.pin()).unwrap();
    assert!(
        skeleton
            .tasks
            .iter()
            .all(|t| !t.implements.contains(&"REQ-F-003".into()))
    );
    let mut findings = findings::skeleton_findings(
        &set.tasks,
        &set.prd,
        &text,
        &set.pin(),
        &set.pin_path(),
        &skeleton,
    )
    .unwrap();
    let routed = extension.take_unowned_findings(&mut findings);
    assert_eq!(routed.len(), 1);
    assert_eq!(
        routed[0].deterministic_defect.as_ref().unwrap().code,
        "unowned_obligation"
    );
    assert_eq!(
        routed[0].remediation_scope,
        archon_workflow::RemediationScope::Skeleton
    );
    assert!(result.diagnostics.iter().any(|d| d.contains("REQ-F-003")));
}
