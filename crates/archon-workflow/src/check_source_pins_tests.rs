use super::tests_support::contract;
use super::*;
use crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE;

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("repo/scripts")).unwrap();
    std::fs::create_dir_all(dir.path().join("project/tasks/set")).unwrap();
    std::fs::write(dir.path().join("repo/scripts/a.sh"), "exit 1\n").unwrap();
    dir
}

#[test]
fn a_contract_is_pinned_with_digests_absent_sources_and_filed_bytes() {
    let dir = tree();
    let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
    let roots = Roots {
        repository: &repo,
        project: &project,
    };
    let blobs = BlobStore::at(dir.path().join("blobs"));
    let c = contract(&[("AC-1", "bash scripts/a.sh"), ("AC-2", "bash scripts/b.sh")]);
    let pins = pin_contract(&c, "digest", &roots, ORIGIN_FREEZE, &blobs);
    let a = &pins.checks["AC-1"].sources[0];
    assert_eq!(
        (a.root, a.path.as_str()),
        (SourceRoot::Repository, "scripts/a.sh")
    );
    assert_eq!(
        a.digest.as_deref(),
        Some(content_digest(b"exit 1\n").as_str())
    );
    assert_eq!(blobs.get(a.digest.as_ref().unwrap()).unwrap(), b"exit 1\n");
    let b = &pins.checks["AC-2"].sources[0];
    assert_eq!(b.digest, None, "absent at pin time");
    assert_eq!(pins.acceptance_digest, "digest");
}

#[test]
fn a_rebind_carries_unchanged_checks_and_repins_reauthored_ones() {
    let dir = tree();
    let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
    let roots = Roots {
        repository: &repo,
        project: &project,
    };
    let blobs = BlobStore::at(dir.path().join("blobs"));
    let c = contract(&[("AC-1", "bash scripts/a.sh"), ("AC-2", "bash scripts/a.sh")]);
    let prior = pin_contract(&c, "one", &roots, ORIGIN_FREEZE, &blobs);
    // Drift outside any landing: an unrelated republish must not bless it.
    std::fs::write(repo.join("scripts/a.sh"), "exit 0\n").unwrap();
    let reauthored = BTreeSet::from(["AC-2".to_string()]);
    let next = rebind(
        &prior,
        &c,
        "two",
        &roots,
        ORIGIN_REPUBLISH,
        &blobs,
        &reauthored,
    );
    assert_eq!(next.checks["AC-1"], prior.checks["AC-1"]);
    assert_ne!(next.checks["AC-2"], prior.checks["AC-2"]);
    assert_eq!(next.acceptance_digest, "two");
}

#[test]
fn a_run_pins_an_old_contract_itself_and_prefers_a_frozen_sidecar_that_binds() {
    let dir = tree();
    let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
    let tasks = project.join("tasks/set");
    let run = project.join(".archon/workflows/run-1");
    let roots = Roots {
        repository: &repo,
        project: &project,
    };
    let c = contract(&[("AC-1", "bash scripts/a.sh")]);
    let bytes = serde_json::to_vec_pretty(&c).unwrap();
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    let (store, pins) = load_for_run(&run, &project, &tasks, &roots)
        .unwrap()
        .unwrap();
    assert!(!store.frozen);
    assert_eq!(pins.origin, ORIGIN_RUN_FIRST_USE);
    assert!(run.join("v2/check-sources/pins.json").is_file());
    // Read back, not re-pinned: the file changed since, the record did not.
    std::fs::write(repo.join("scripts/a.sh"), "exit 0\n").unwrap();
    let (_, again) = load_for_run(&run, &project, &tasks, &roots)
        .unwrap()
        .unwrap();
    assert_eq!(again, pins);
    // A frozen sidecar that binds the contract wins.
    let frozen = PinStore::frozen(&project, &tasks);
    let mut sidecar = pins.clone();
    sidecar.origin = ORIGIN_FREEZE.into();
    frozen.write(&sidecar).unwrap();
    let (store, read) = load_for_run(&run, &project, &tasks, &roots)
        .unwrap()
        .unwrap();
    assert!(store.frozen);
    assert_eq!(read.origin, ORIGIN_FREEZE);
    // No contract: nothing to pin.
    let empty = project.join("tasks/none");
    assert!(
        load_for_run(&run, &project, &empty, &roots)
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_acceptance_pin_records_the_sidecar_and_a_mismatch_is_restored_or_refused() {
    let dir = tree();
    let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
    let tasks = project.join("tasks/set");
    let roots = Roots {
        repository: &repo,
        project: &project,
    };
    let store = PinStore::frozen(&project, &tasks);
    let pin_path = store.pin.clone().unwrap();
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    let pin = serde_json::json!({
        "task_root": tasks.display().to_string(), "acceptance_digest": "d",
        "freeze_event_id": "acceptance-freeze-d",
        "acceptance_gate": {"mode": "enforce", "finding_count": 0, "findings_digest": "x",
            "binary_commit": "c", "evaluated_at": "2026-01-01T00:00:00Z"}
    });
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    let c = contract(&[("AC-1", "bash scripts/a.sh")]);
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &store.blobs);
    store.write(&pins).unwrap();
    let recorded: crate::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    assert_eq!(
        recorded.check_sources_digest.as_deref(),
        Some(content_digest(&PinStore::bytes(&pins)).as_str()),
        "the re-pin goes through the pin"
    );
    // A sidecar that no longer hashes to the pin is read from its filed copy,
    // and a reader never writes it (Issue 338): only the exclusive restore.
    std::fs::write(&store.sidecar, b"{\"tampered\": true}").unwrap();
    assert_eq!(store.verified_read().unwrap().unwrap(), pins);
    assert_eq!(
        std::fs::read(&store.sidecar).unwrap(),
        b"{\"tampered\": true}",
        "a reader wrote the sidecar"
    );
    store.restore_sidecar().unwrap();
    assert_eq!(
        store.read().unwrap().unwrap(),
        pins,
        "restored under the lock"
    );
    // With no filed copy it is refused, never read.
    let mut other = pins.clone();
    other.origin = "other".into();
    let mut pin = recorded.clone();
    pin.check_sources_digest = Some(content_digest(&PinStore::bytes(&other)));
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    assert!(store.verified_read().unwrap_err().contains("does not hash"));
    // A pin frozen before the field existed is not held to it.
    let mut old = recorded;
    old.check_sources_digest = None;
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();
    assert_eq!(store.verified_read().unwrap().unwrap(), pins);
}

/// Review minor 10: a frozen task set whose contract is missing is an
/// error, never "nothing pinned".
#[test]
fn a_frozen_task_set_missing_its_contract_is_an_error() {
    let dir = tree();
    let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
    let tasks = project.join("tasks/set");
    let run = project.join(".archon/workflows/run-1");
    let roots = Roots {
        repository: &repo,
        project: &project,
    };
    assert!(
        load_for_run(&run, &project, &tasks, &roots)
            .unwrap()
            .is_none()
    );
    std::fs::write(
        tasks.join(crate::task_set_contract::ACCEPTANCE_LOCK_FILE),
        "{}",
    )
    .unwrap();
    let error = load_for_run(&run, &project, &tasks, &roots).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("missing although the task set was frozen"),
        "{error}"
    );
}
