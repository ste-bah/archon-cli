use super::*;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(project.join("inputs/nested")).unwrap();
    let project = archon_shell::paths::plain(project.canonicalize().unwrap());
    let root = project.join(".archon/workflows/fixture");
    super::super::super::project_inputs::write_test_policy(&root, &project, &["inputs"]);
    (temp, project, root)
}

async fn arm_regular(case: u8) {
    let (_temp, project, root) = fixture();
    let store = crate::WorkflowStore::new(&root);
    let mut run = store
        .create_run(crate::WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "watch".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    run.status = crate::RunStatus::Running;
    store.save_state(&run).unwrap();
    let root = store.run_dir(&run.id);
    super::super::super::project_inputs::write_test_policy(&root, &project, &["inputs"]);
    let names = match case {
        0 => vec!["a"],
        1 => vec!["a", "b"], // deduplicated content objects
        _ => vec!["nested/a"],
    };
    for name in names {
        std::fs::write(project.join("inputs").join(name), b"before").unwrap();
    }
    crate::durable_io::take_synced();
    let owned = arm(&root, "regular input").unwrap().unwrap();
    let object = objects_dir(&root).join(blake3::hash(b"before").to_hex().as_str());
    assert!(
        crate::durable_io::take_synced().contains(&object),
        "retained object bypassed platform-aware sync"
    );
    owned.check().unwrap();
    // On Windows this must reach the watched work, not pause at FlushFileBuffers.
    let called = std::cell::Cell::new(false);
    let writer = crate::stage_write::StageWriter {
        store,
        run_id: run.id,
        owner: crate::control_pause::PauseOwner::Generation(run.generation),
    };
    super::super::watch_owned(writer, Some(&root), "regular work", async {
        called.set(true);
    })
    .await
    .unwrap();
    assert!(called.get());
}
#[tokio::test]
async fn r3_regular_input_arms_and_reaches_work() {
    arm_regular(0).await;
}
#[tokio::test]
async fn r3_duplicate_objects_arm_and_reach_work() {
    arm_regular(1).await;
}
#[tokio::test]
async fn r3_nested_regular_input_arms_and_reaches_work() {
    arm_regular(2).await;
}

fn durable_repair(case: u8) {
    let (_temp, project, root) = fixture();
    let input = project.join("inputs/nested/a");
    if case != 1 {
        std::fs::write(&input, b"before").unwrap();
    }
    let owned = arm(&root, "repair").unwrap().unwrap();
    if case == 2 {
        std::fs::remove_dir_all(input.parent().unwrap()).unwrap();
    } else {
        std::fs::write(&input, b"changed").unwrap();
    }
    crate::durable_io::take_synced();
    let violation = owned.check().unwrap().unwrap();
    assert!(violation.restored());
    let synced = crate::durable_io::take_synced();
    let repair = synced
        .iter()
        .position(|p| p == input.parent().unwrap())
        .expect("input repair directory was not synced");
    let cleanup = synced.iter().rposition(|p| p == &directory(&root)).unwrap();
    assert!(repair < cleanup);
    if case != 2 {
        let backup = violation.backup_dir.join("inputs/nested/a");
        let kept = synced
            .iter()
            .position(|p| p == &backup)
            .expect("changed copy was not synced");
        assert!(kept < repair, "backup must survive before repair");
    }
    if case != 1 {
        assert!(
            synced[..repair].iter().any(|p| p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with("archon-input.tmp"))),
            "repair bytes were not synced before rename"
        );
        assert_eq!(std::fs::read(&input).unwrap(), b"before");
    }
    if case == 2 {
        assert!(
            synced[..cleanup].contains(&project.join("inputs")),
            "new directory entry was not synced"
        );
    }
}
#[test]
fn r3_replacement_and_backup_durable_before_cleanup() {
    durable_repair(0);
}
#[test]
fn r3_deletion_and_backup_durable_before_cleanup() {
    durable_repair(1);
}
#[test]
fn r3_recreated_directories_durable_before_cleanup() {
    durable_repair(2);
}

#[test]
fn r3_matching_repaired_input_synced_before_cleanup() {
    let (_temp, project, root) = fixture();
    for name in ["a", "b"] {
        std::fs::write(project.join("inputs").join(name), b"before").unwrap();
    }
    let mut owned = arm(&root, "matching repair").unwrap().unwrap();
    for name in ["a", "b"] {
        std::fs::write(project.join("inputs").join(name), b"changed").unwrap();
    }
    let report = report_path(&owned.path);
    assert!(
        owned
            .tripwire
            .take()
            .unwrap()
            .check_recording("matching repair", true, |v| {
                if v.changed.len() > 1 {
                    return Err(pause("interrupted after A repair"));
                }
                save(&report, &serde_json::to_vec(v)?)
            })
            .is_err()
    );
    drop(owned);
    std::fs::write(project.join("inputs/b"), b"before").unwrap();
    crate::durable_io::take_synced();
    assert!(reconcile(&root).is_err());
    let synced = crate::durable_io::take_synced();
    let repair = synced
        .iter()
        .position(|p| p == &project.join("inputs/a"))
        .expect("matching repair was not made durable before acknowledgement");
    let cleanup = synced.iter().rposition(|p| p == &directory(&root)).unwrap();
    assert!(repair < cleanup);
    assert!(reconcile(&root).is_ok());
}

fn interrupted_repairs(case: u8) {
    let (_temp, project, root) = fixture();
    for name in ["a", "b"] {
        std::fs::write(project.join("inputs").join(name), b"before").unwrap();
    }
    let mut owned = arm(&root, "interrupted").unwrap().unwrap();
    for name in ["a", "b"] {
        std::fs::write(project.join("inputs").join(name), b"changed").unwrap();
    }
    if case == 1 {
        std::fs::remove_file(project.join("inputs/b")).unwrap();
    }
    if case == 2 {
        std::fs::write(project.join("inputs/nested/c"), b"new").unwrap();
    }
    let report = report_path(&owned.path);
    let result = owned
        .tripwire
        .take()
        .unwrap()
        .check_recording("interrupted", true, |v| {
            if v.changed.len() > 1 {
                return Err(pause("interruption between A and B repairs"));
            }
            save(&report, &serde_json::to_vec(v)?)
        });
    assert!(result.is_err());
    assert_eq!(std::fs::read(project.join("inputs/a")).unwrap(), b"before");
    let first: EnvironmentViolation =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    drop(owned);
    assert!(reconcile(&root).is_err(), "recovered violation must pause");
    let text =
        std::fs::read_to_string(root.join("write-coordination/environment-violations.jsonl"))
            .unwrap();
    let logged: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let recovered: EnvironmentViolation =
        serde_json::from_value(logged["violation"].clone()).unwrap();
    assert!(
        recovered.changed.iter().any(|c| c.path == "inputs/a"),
        "A receipt was overwritten by B"
    );
    assert!(recovered.changed.iter().any(|c| c.path == "inputs/b"));
    assert_eq!(
        recovered.backup_dir, first.backup_dir,
        "A backup reference disappeared"
    );
    assert_eq!(
        std::fs::read(recovered.backup_dir.join("inputs/a")).unwrap(),
        b"changed"
    );
    assert!(recovered.restored());
    assert!(reconcile(&root).is_ok());
}
#[test]
fn r3_interrupted_two_replacements_keep_both_receipts() {
    interrupted_repairs(0);
}
#[test]
fn r3_interrupted_replacement_and_deletion_keep_receipts() {
    interrupted_repairs(1);
}
#[test]
fn r3_interrupted_repairs_and_new_file_keep_receipts() {
    interrupted_repairs(2);
}

#[test]
fn r3_process_exit_between_two_repairs_keeps_both_receipts() {
    if let Ok(project) = std::env::var("ARCHON_R3_REPAIR_CHILD_PROJECT") {
        let project = PathBuf::from(project);
        let root = PathBuf::from(std::env::var("ARCHON_R3_REPAIR_CHILD_ROOT").unwrap());
        let mut owned = arm(&root, "exited process").unwrap().unwrap();
        for name in ["a", "b"] {
            std::fs::write(project.join("inputs").join(name), b"changed").unwrap();
        }
        let report = report_path(&owned.path);
        let result = owned
            .tripwire
            .take()
            .unwrap()
            .check_recording("exited process", true, |v| {
                if v.changed.len() > 1 {
                    return Err(pause("exit before B repair"));
                }
                save(&report, &serde_json::to_vec(v)?)
            });
        assert!(result.is_err());
        return; // The process exits with A repaired, B changed, no final log.
    }
    let (_temp, project, root) = fixture();
    for name in ["a", "b"] {
        std::fs::write(project.join("inputs").join(name), b"before").unwrap();
    }
    let name = std::thread::current().name().unwrap().to_string();
    assert!(
        archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args(["--exact", &name])
            .env("ARCHON_R3_REPAIR_CHILD_PROJECT", &project)
            .env("ARCHON_R3_REPAIR_CHILD_ROOT", &root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(std::fs::read(project.join("inputs/a")).unwrap(), b"before");
    assert_eq!(std::fs::read(project.join("inputs/b")).unwrap(), b"changed");
    assert!(
        !root
            .join("write-coordination/environment-violations.jsonl")
            .exists()
    );
    assert!(reconcile(&root).is_err());
    let text =
        std::fs::read_to_string(root.join("write-coordination/environment-violations.jsonl"))
            .unwrap();
    let logged: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let recovered: EnvironmentViolation =
        serde_json::from_value(logged["violation"].clone()).unwrap();
    assert_eq!(
        recovered.changed.len(),
        2,
        "process restart lost A's receipt"
    );
    assert_eq!(
        std::fs::read(recovered.backup_dir.join("inputs/a")).unwrap(),
        b"changed"
    );
    assert!(
        recovered
            .changed
            .iter()
            .any(|c| c.path == "inputs/a" && c.restored)
    );
    assert!(
        recovered
            .changed
            .iter()
            .any(|c| c.path == "inputs/b" && !c.restored)
    );
    // A different process cannot safely restore using the lost host-write history.
    std::fs::write(project.join("inputs/b"), b"before").unwrap();
    assert!(reconcile(&root).is_err());
    assert!(reconcile(&root).is_ok());
}
