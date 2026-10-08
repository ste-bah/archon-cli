#[path = "support/branch_revocation.rs"]
mod branch;
#[path = "support/restart_run.rs"]
mod restart_run;

use archon_workflow::{
    LifecycleAction, LifecycleController, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2ResultStore, WorkflowV2Status,
};
use branch::{CALL, item, outcome};
use restart_run::{accepted, generated_run, generated_run_with_stages, v2_store};

#[test]
fn branch_archive_is_partitioned_by_item_and_migrates_flat_entries_on_save() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    for task in ["T-A", "T-B"] {
        v2.save_branch_outcome(
            CALL,
            &outcome(task, WorkflowV2Status::Accepted, "one", true),
        )
        .unwrap();
        v2.save_branch_outcome(CALL, &outcome(task, WorkflowV2Status::Noop, "two", false))
            .unwrap();
    }
    let call = v2
        .branch_outcome_path(CALL, &item("T-A").id)
        .parent()
        .unwrap()
        .to_path_buf();
    let archive = call.join("superseded");
    let item_archives = std::fs::read_dir(&archive)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(item_archives.len(), 2);
    assert!(item_archives.iter().all(|path| path.is_dir()));
    let legacy = archive.join("legacy.json");
    std::fs::write(
        &legacy,
        serde_json::to_vec(&outcome("T-A", WorkflowV2Status::Accepted, "legacy", true)).unwrap(),
    )
    .unwrap();
    v2.save_branch_outcome(
        CALL,
        &outcome("T-A", WorkflowV2Status::Noop, "three", false),
    )
    .unwrap();
    assert!(!legacy.exists());
    assert_eq!(v2.load_superseded_branch_outcomes().len(), 4);
}

#[test]
fn hashed_branch_paths_do_not_alias_and_legacy_current_paths_load() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = WorkflowV2ResultStore::new(temp.path());
    let mut slash = outcome("T-A", WorkflowV2Status::Accepted, "slash", true);
    slash.item_id = "item:x".into();
    let mut underscore = outcome("T-B", WorkflowV2Status::Noop, "underscore", false);
    underscore.item_id = "item?x".into();
    let slash_path = v2.branch_outcome_path("a/b", "item:x");
    let underscore_path = v2.branch_outcome_path("a_b", "item?x");
    assert_ne!(slash_path, underscore_path);
    v2.save_branch_outcome("a/b", &slash).unwrap();
    v2.save_branch_outcome("a_b", &underscore).unwrap();
    assert_eq!(
        v2.load_branch_outcome("a/b", "item:x").unwrap().unwrap(),
        slash
    );
    assert_eq!(
        v2.load_branch_outcome("a_b", "item?x").unwrap().unwrap(),
        underscore
    );

    let legacy = temp.path().join("branches/plain/item.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let mut legacy_outcome = slash;
    legacy_outcome.item_id = "item".into();
    std::fs::write(&legacy, serde_json::to_vec(&legacy_outcome).unwrap()).unwrap();
    assert!(v2.load_branch_outcome("plain", "item").unwrap().is_some());
    assert_eq!(v2.delete_branch_outcomes_for_call("plain").unwrap(), 1);
    assert!(!legacy.exists());
    assert!(legacy.parent().unwrap().join("revoked").is_dir());
}

#[test]
fn whole_call_branch_restart_preserves_current_superseded_and_prior_revoked_records() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    for task in ["T-A", "T-B", "T-C"] {
        v2.save_branch_outcome(
            CALL,
            &outcome(task, WorkflowV2Status::Accepted, "one", true),
        )
        .unwrap();
        if task != "T-C" {
            v2.save_branch_outcome(CALL, &outcome(task, WorkflowV2Status::Noop, "two", false))
                .unwrap();
        }
    }
    let call = v2
        .branch_outcome_path(CALL, &item("T-A").id)
        .parent()
        .unwrap()
        .to_path_buf();
    let prior = call.join("revoked/prior.json");
    std::fs::create_dir_all(prior.parent().unwrap()).unwrap();
    std::fs::write(&prior, b"audit").unwrap();
    assert_eq!(v2.delete_branch_outcomes_for_call(CALL).unwrap(), 5);
    assert!(!v2.branch_outcome_path(CALL, &item("T-A").id).exists());
    assert!(!v2.branch_outcome_path(CALL, &item("T-B").id).exists());
    assert!(!v2.branch_outcome_path(CALL, &item("T-C").id).exists());
    assert!(prior.exists());
    assert!(v2.load_superseded_branch_outcomes().is_empty());
}

#[test]
fn restart_stage_revokes_branch_history_and_keeps_audit_files() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run_with_stages(&temp, &["call-a"], &["call-a"]);
    let v2 = v2_store(&store, &run);
    let current = outcome("T-A", WorkflowV2Status::Noop, "two", false);
    v2.save_branch_outcome(
        "call-a",
        &outcome("T-A", WorkflowV2Status::Accepted, "one", true),
    )
    .unwrap();
    v2.save_branch_outcome("call-a", &current).unwrap();
    let current_path = v2.branch_outcome_path("call-a", &current.item_id);
    let archive_path = current_path
        .parent()
        .unwrap()
        .join("superseded")
        .join(branch::branch_component(&current.item_id));
    LifecycleController::new(store.clone())
        .apply_restart(&run.id, LifecycleAction::RestartStage("call-a".into()))
        .unwrap();
    assert!(!current_path.exists());
    assert_eq!(std::fs::read_dir(archive_path).unwrap().count(), 0);
    let revoked = current_path.parent().unwrap().join("revoked");
    assert_eq!(std::fs::read_dir(revoked).unwrap().count(), 2);
    assert!(v2.load_superseded_branch_outcomes().is_empty());
}

#[test]
fn whole_call_restart_clears_downstream_call_branches_too() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run_with_stages(&temp, &["call-a", "call-b"], &[]);
    let metadata_path = store.run_dir(&run.id).join("v2/generated-metadata.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    let call = |id: &str, source: Option<&str>| {
        let mut call = WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        };
        call.options.source = source.map(str::to_string);
        call
    };
    metadata["generated_scaffold"]["host_call_manifest"] = serde_json::to_value(vec![
        call("call-a", None),
        call("call-b", Some("call-a.output")),
    ])
    .unwrap();
    std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let v2 = v2_store(&store, &run);
    for id in ["call-a", "call-b"] {
        v2.save_call_record(&accepted(id, "T-A")).unwrap();
        let mut branch = outcome("T-A", WorkflowV2Status::Accepted, "branch", true);
        branch.item_id = format!("{id}-item");
        v2.save_branch_outcome(id, &branch).unwrap();
    }
    let invalidated =
        archon_workflow::v2::restart::invalidate_generated_v2_call(&store, &run, "call-a").unwrap();
    assert!(invalidated.contains(&"call-b".to_string()));
    for id in ["call-a", "call-b"] {
        assert!(!v2.branch_outcome_path(id, &format!("{id}-item")).exists());
        assert!(
            v2.branch_outcome_path(id, &format!("{id}-item"))
                .parent()
                .unwrap()
                .join("revoked")
                .is_dir()
        );
    }
}
