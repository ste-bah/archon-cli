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
    let saved = store.load(&key, false).unwrap();
    assert!(!String::from_utf8_lossy(&saved.stdout).contains("no tests collected"));
    assert!(
        !super::super::baseline::passed(&saved),
        "redaction changed verdict"
    );
}

#[test]
fn round2_unparseable_cache_is_retained_for_investigation() {
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
    let provisional = path.with_extension("provisional");
    let provisional_bytes = br#"{"schema":3,"key":"evidence"}"#;
    std::fs::write(&provisional, provisional_bytes).unwrap();
    assert!(store.load(&key, false).is_none());
    assert!(path.exists(), "unparseable evidence was deleted");
    assert_eq!(std::fs::read(provisional).unwrap(), provisional_bytes);
}

#[test]
fn layoutless_current_schema_cache_is_a_miss_without_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            Vec::new(),
            &[],
        ),
    );
    let key = "e".repeat(64);
    let path = store.path(&key).unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema": 4,
            "key": key,
            "result": {
                "acceptance_id": "AC-Z-004", "exit_code": 1,
                "stdout": [], "stderr": [], "operational_error": null,
                "classification": {"passed": false, "zero_work": false,
                    "crash": "Failed"}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        store.load(&key, false).is_some(),
        "schema-4 evidence remains readable before site-specific layout checks"
    );
    assert!(path.exists(), "layoutless evidence must be retained");
}

#[test]
fn scratch_cache_without_site_repository_mapping_is_a_miss_without_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            Vec::new(),
            &[],
        ),
    );
    let key = "a".repeat(64);
    let path = store.path(&key).unwrap();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema": 4,
        "key": key,
        "result": {
            "acceptance_id": "AC-X-001", "exit_code": 1,
            "stdout": [], "stderr": [], "operational_error": null,
            "classification": {"passed": false, "zero_work": false, "crash": "Failed"}
        },
        "copy_layout": {
            "root": "/scratch", "repository": "/scratch/repo", "project": "/scratch/project",
            "siblings": [], "check_siblings": {"AC-X-001": [{"path": "../repo-alias", "commit": null}]}
        }
    })).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    assert!(
        store.load(&key, true).is_none(),
        "Scratch evidence from before site repository mappings must be reobserved"
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn schema3_scratch_result_is_reused_without_changing_its_file_or_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            Vec::new(),
            &[],
        ),
    );
    let key = "f".repeat(64);
    let result: archon_workflow::acceptance_scratch::CheckResult =
        serde_json::from_value(serde_json::json!({
            "acceptance_id": "AC-SCHEMA-003", "exit_code": 1,
            "stdout": [], "stderr": [], "operational_error": null,
            "classification": {"passed": false, "zero_work": false, "crash": "Failed"}
        }))
        .unwrap();
    assert!(store.save(&key, &result));
    let path = store.path(&key).unwrap();
    std::fs::rename(path.with_extension("provisional"), &path).unwrap();
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved["schema"] = serde_json::json!(3);
    saved.as_object_mut().unwrap().remove("copy_layout");
    let legacy_bytes = serde_json::to_vec(&saved).unwrap();
    std::fs::write(&path, &legacy_bytes).unwrap();

    assert!(
        store.load(&key, false).is_none(),
        "Hermetic rejects schema 3"
    );
    assert_eq!(std::fs::read(&path).unwrap(), legacy_bytes);
    let loaded = store
        .load(&key, true)
        .expect("scratch schema-3 verdict is reusable");
    assert!(loaded.copy_layout.is_none());
    assert_eq!(
        super::super::baseline::passed(&loaded),
        super::super::baseline::passed(&result),
        "scratch context must preserve the prior verdict"
    );
    assert_eq!(std::fs::read(path).unwrap(), legacy_bytes);
}

#[test]
fn replacing_a_schema3_provisional_record_keeps_its_original_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = super::ResultStore::new(
        dir.path().into(),
        crate::command::workflow_task_set::passability::evidence::Redactor::from_vars(
            Vec::new(),
            &[],
        ),
    );
    let key = "9".repeat(64);
    let path = store.path(&key).unwrap();
    let provisional = path.with_extension("provisional");
    let previous = br#"{"schema":3,"key":"legacy"}"#;
    std::fs::write(&provisional, previous).unwrap();
    let result: archon_workflow::acceptance_scratch::CheckResult =
        serde_json::from_value(serde_json::json!({
            "acceptance_id": "AC-SCHEMA-REPLACE", "exit_code": 0,
            "stdout": [], "stderr": [], "operational_error": null,
            "classification": {"passed": true, "zero_work": false, "crash": "Passed"}
        }))
        .unwrap();

    assert!(store.save(&key, &result));
    assert_eq!(
        std::fs::read(path.with_extension("schema3")).unwrap(),
        previous
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(provisional).unwrap()).unwrap()
            ["schema"],
        5
    );
}

async fn hermetic_old_record_is_a_miss_and_survives_rerun(schema: u32, layout: bool) {
    use super::super::HostProbe;
    use super::super::probe_tests::trees;
    use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};
    let runs = tempfile::tempdir().unwrap();
    let check = format!(
        "echo run >> {}; test -f feature.txt",
        runs.path().join("count").display()
    );
    let trees = trees(&[(
        "AC-SCHEMA-REUSE",
        &check,
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
    assert_eq!(
        std::fs::read_to_string(runs.path().join("count"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    let dir = trees
        .set
        .project
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("probe-results");
    let mut originals = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if value["result"]["acceptance_id"] == "AC-SCHEMA-REUSE" {
                value["schema"] = serde_json::json!(schema);
                if !layout {
                    value.as_object_mut().unwrap().remove("copy_layout");
                }
                let bytes = serde_json::to_vec(&value).unwrap();
                std::fs::write(&path, &bytes).unwrap();
                originals.push((path, bytes));
            }
        }
    }
    assert_eq!(originals.len(), 1, "the result was persisted");

    let retry_probe = probe();
    let retry = retry_probe.run(&contract, &digest, &refs).await;
    assert_eq!(retry.len(), 1);
    assert!(
        retry_probe
            .copies_made
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
    );
    assert_eq!(
        std::fs::read_to_string(runs.path().join("count"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    for (path, bytes) in originals {
        let suffix = if schema == 3 {
            "schema3"
        } else {
            "schema4-layoutless"
        };
        let archived = path.with_extension(suffix);
        assert_eq!(std::fs::read(archived).unwrap(), bytes);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()["schema"],
            5
        );
    }
}

#[tokio::test]
async fn hermetic_schema3_records_miss_rerun_and_keep_old_bytes() {
    hermetic_old_record_is_a_miss_and_survives_rerun(3, false).await;
}

#[tokio::test]
async fn hermetic_schema4_layoutless_records_miss_rerun_and_keep_old_bytes() {
    hermetic_old_record_is_a_miss_and_survives_rerun(4, false).await;
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
    let saved = store.load(&key, false).unwrap();
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
#[test]
fn archive_fallback_keeps_different_bytes_when_writers_interleave() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("result.previous");
    let first = b"first prior result";
    let second = b"second prior result";

    assert!(super::write_archive(&archive, first));
    assert!(super::write_archive(&archive, second));

    assert_eq!(std::fs::read(&archive).unwrap(), first);
    assert_eq!(
        std::fs::read(dir.path().join("result.previous.1")).unwrap(),
        second
    );
}
