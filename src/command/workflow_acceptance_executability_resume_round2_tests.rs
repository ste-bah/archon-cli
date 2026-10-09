#[test]
fn round2_saved_zero_work_verdict_survives_redaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            vec![("SERVICE_TOKEN".into(), "no tests collected".into())],
            &["SERVICE_TOKEN".into()],
        ),
    );
    let key = "b".repeat(64);
    let raw: archon_workflow::acceptance_scratch::CheckResult =
        serde_json::from_value(serde_json::json!({
            "acceptance_id": "AC-Z-001", "exit_code": 0, "stdout": b"no tests collected".to_vec(),
            "stderr": [], "operational_error": null
        }))
        .unwrap();
    assert!(!super::super::baseline::passed(&raw));
    assert!(store.save(&key, &raw));
    let path = store.path(&key).unwrap();
    std::fs::rename(path.with_extension("provisional"), &path).unwrap();
    let saved = store.load(&key).unwrap();
    assert!(!String::from_utf8_lossy(&saved.stdout).contains("no tests collected"));
    assert!(
        !super::super::baseline::passed(&saved),
        "redaction changed verdict"
    );
}

#[test]
fn round2_old_cache_is_discarded_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            Vec::new(),
            &[],
        ),
    );
    let key = "c".repeat(64);
    let path = store.path(&key).unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema": 2, "key": key, "result": {"acceptance_id":"AC-Z-002", "exit_code":0,
            "stdout": b"old-plaintext-secret".to_vec(), "stderr": [], "operational_error": null}
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(store.load(&key).is_none());
    assert!(!path.exists(), "obsolete raw cache retained");
}

#[test]
fn round2_saved_crash_class_survives_redaction() {
    use archon_workflow::acceptance_check_crash::{CheckRunClass, classify_check_run};
    let command = "python3 - <<'PY'\nprint(missing)\nPY";
    let stderr = "Traceback (most recent call last):\n  File \"<stdin>\", line 1, in <module>\nNameError: name 'missing' is not defined\n";
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            vec![("SERVICE_TOKEN".into(), stderr.into())],
            &[],
        ),
    );
    let key = "d".repeat(64);
    let raw: archon_workflow::acceptance_scratch::CheckResult =
        serde_json::from_value(serde_json::json!({
            "acceptance_id": "AC-C-001", "exit_code": 1, "stdout": [], "stderr": stderr.as_bytes(),
            "operational_error": null,
            "classification": {"passed": false, "zero_work": false, "crash": {"ScriptDefect": {
                "interpreter": "python", "rule": "undefined name in the check's inline python",
                "signal": "see the fenced stderr below"}}}
        }))
        .unwrap();
    assert!(matches!(
        classify_check_run(command, &raw),
        CheckRunClass::ScriptDefect(_)
    ));
    assert!(store.save(&key, &raw));
    let path = store.path(&key).unwrap();
    std::fs::rename(path.with_extension("provisional"), &path).unwrap();
    let saved = store.load(&key).unwrap();
    assert!(!String::from_utf8_lossy(&saved.stderr).contains("NameError"));
    assert!(
        matches!(
            classify_check_run(command, &saved),
            CheckRunClass::ScriptDefect(_)
        ),
        "crash class lost"
    );
}

#[tokio::test]
async fn round2_hermetic_verdict_is_identical_on_disk_reuse() {
    use super::super::HostProbe;
    use super::super::probe_tests::trees;
    use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};
    let trees = trees(&[(
        "AC-Z-003",
        "test -f missing.txt",
        archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
    )]);
    let copies = tempfile::tempdir().unwrap();
    let resume = FreezeResume::saving(FreezeBudget::unlimited(), true);
    let probe = || {
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_copy_parent(copies.path().into())
            .with_resume(&resume)
            .without_process_memo()
    };
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    let first = probe().run(&contract, &digest, &refs).await;
    assert_eq!(first.len(), 1);
    assert!(!super::super::baseline::passed(&first[0]));
    let retry = probe();
    let reused = retry.run(&contract, &digest, &refs).await;
    assert_eq!(
        retry.copies_made.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(
        !super::super::baseline::passed(&reused[0]),
        "disk reuse changed verdict"
    );
}
