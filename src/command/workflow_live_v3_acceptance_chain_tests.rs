//! The write path end to end: a sanctioned per-check repair records its
//! lineage and files the chain it replaced, and every round adopts the
//! republished chain through the same check the run-end observer makes. A
//! repair an older binary made (no lineage, nothing filed) refuses the round,
//! naming the import that files the surviving launch chain; the import files
//! only digests the launch pin names and changes nothing under the task root.

use std::collections::BTreeSet;

use archon_workflow::task_set_contract::TASK_SKELETON_FILE;
use archon_workflow::task_set_lineage::ChainHistory;

use super::*;
use crate::command::acceptance_chain::import_history;
use crate::command::workflow_task_set::reauthor::AuthorScope;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::{FrozenSet, frozen_set};
use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};

const RUN: &str = "wf-chain-e2e";

fn context(
    set: &FrozenSet,
    launch: &archon_workflow::PortableAcceptanceIdentityV1,
) -> StageContext {
    StageContext {
        project: set.project.path().to_path_buf(),
        task_root: set.tasks.clone(),
        repository: set.project.path().to_path_buf(),
        binding: None,
        launch: Some(launch.clone()),
        run_id: RUN.to_string(),
    }
}

async fn reauthor(set: &FrozenSet, id: &str, command: &'static str) {
    let ids: BTreeSet<String> = [id.to_string()].into();
    let client =
        ScriptedAuthorJudge::new(move |entry, _| command_entry(entry, command), |_, _| true);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let request = ReauthorRequest {
        project_root: set.project.path(),
        tasks_root: &set.tasks,
        prd_path: &set.prd,
        ids: &ids,
        gate: set.gate(),
        trigger: "test",
    };
    reauthor_and_republish(&client, request, &scope)
        .await
        .expect("the repair publishes");
}

fn write_launch_snapshot(set: &FrozenSet, launch: &archon_workflow::PortableAcceptanceIdentityV1) {
    let store = WorkflowStore::project(set.project.path());
    let path = store.run_dir(RUN).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let snapshot = serde_json::json!({
        "observer_snapshot": {
            "schema_version": 1,
            "canonical_task_root_identity": set.tasks.canonicalize().unwrap().display().to_string(),
            "expected_artifact_paths": archon_workflow::RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS,
            "portable_acceptance_identity": launch,
        }
    });
    std::fs::write(path, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
}

#[tokio::test]
async fn repairs_are_adopted_through_the_chain_check_and_an_unrecorded_one_names_the_import() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", true),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
    ]);
    let launch = set.pin().identity();
    let launch_contract = set.contract_bytes();
    let launch_skeleton = std::fs::read(set.tasks.join(TASK_SKELETON_FILE)).unwrap();
    load_contract(&context(&set, &launch)).expect("the launch chain loads");

    reauthor(&set, "AC-F-002", "jq -e '.b == true and .d == 1' out.json").await;
    let pin = set.pin();
    assert_eq!(pin.lineage.len(), 1);
    assert_eq!(pin.lineage[0].from, launch);
    assert_eq!(pin.lineage[0].to, pin.identity());
    assert_eq!(
        pin.lineage[0].reauthored_ids,
        BTreeSet::from(["AC-F-002".to_string()])
    );
    let history = ChainHistory::for_pin(&set.pin_path());
    assert!(history.get(&launch.acceptance_digest).unwrap().is_some());
    assert!(
        history
            .get(launch.skeleton_digest.as_deref().unwrap())
            .unwrap()
            .is_some()
    );
    load_contract(&context(&set, &launch)).expect("a recorded repair is adopted");

    reauthor(&set, "AC-F-002", "jq -e '.b == true and .d == 2' out.json").await;
    assert_eq!(set.pin().lineage.len(), 2);
    load_contract(&context(&set, &launch)).expect("a recorded two-hop repair is adopted");

    // What an older binary left: the same chain, no lineage, nothing filed.
    let mut stripped = set.pin();
    stripped.lineage.clear();
    std::fs::write(
        set.pin_path(),
        serde_json::to_vec_pretty(&stripped).unwrap(),
    )
    .unwrap();
    std::fs::remove_dir_all(history.dir()).unwrap();
    let refused = load_contract(&context(&set, &launch))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("chain check unrecorded_change failed"),
        "{refused}"
    );
    assert!(
        refused.contains(&format!(
            "archon workflow import-chain-history {RUN} --from"
        )),
        "{refused}"
    );

    write_launch_snapshot(&set, &launch);
    let chain_before = set.chain_bytes();
    let saved = tempfile::tempdir().unwrap();
    let files = [
        ("contract", &launch_contract),
        ("skeleton", &launch_skeleton),
        ("current", &set.contract_bytes()),
    ]
    .map(|(name, bytes)| {
        let path = saved.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    });
    let unnamed = import_history(set.project.path(), RUN, &files)
        .unwrap_err()
        .to_string();
    assert!(
        unnamed.contains("chain check unnamed_digest failed"),
        "{unnamed}"
    );
    assert!(
        !history.dir().exists(),
        "one unnamed file refuses the whole import"
    );
    let imported = import_history(set.project.path(), RUN, &files[..2]).unwrap();
    assert_eq!(imported.len(), 2);
    assert_eq!(
        set.chain_bytes(),
        chain_before,
        "the import writes nothing under the task root"
    );
    let proven = load_contract(&context(&set, &launch));
    assert!(proven.is_ok(), "{:?}", proven.err());
}
