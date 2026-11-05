#![allow(non_snake_case)]
use super::super::super::{ExecutabilityProbe, HostProbe, probe_tests::trees};
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[tokio::test]
async fn P1_17_real_same_parent_still_passed() {
    real(&["data/a", "data/b"], "ls data/a data/b; true").await;
}
#[tokio::test]
async fn P1_19_real_nested_still_passed() {
    real(&["data/tree/a"], "ls data/tree data/tree/a; true").await;
}
async fn real(files: &[&str], command: &str) {
    let trees = trees(&[("AC-X-001", command, TrustedCwd::ProjectRoot)]);
    for file in files {
        let target = trees.set.project.path().join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "original").unwrap();
    }
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let digest = super::super::super::contract_digest(&contract).unwrap();
    let findings = super::super::prove(
        &probe,
        &Baseline {
            origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
            commit: trees.base.clone(),
            repository: trees.repo.clone(),
        },
        &contract,
        &digest,
        &[],
        &["AC-X-001".into()],
        &BTreeMap::new(),
    )
    .await;
    assert!(probe.take_unproven().is_empty());
    let text = &findings["AC-X-001"];
    assert!(text.contains("it still passed with"), "{text}");
    assert!(text.contains("so it does not depend on them"), "{text}");
    assert!(!text.contains("original-recreated"), "{text}");
}
