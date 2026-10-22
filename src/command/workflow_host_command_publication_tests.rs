use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use archon_workflow::{
    PREPARED_PUBLICATION_SCHEMA_VERSION, PreparedPublicationEntry, PreparedPublicationV1,
    RemediationScope,
};

use super::workflow_host_command_catalog::ResolvedHostCommand;
use super::workflow_host_command_publish::{
    LiveMutationSentinels, audit_prepared_publication, prepare_staging, publish_audited,
};

fn digest(bytes: &[u8]) -> String {
    archon_workflow::task_set_contract::content_digest(bytes)
}

fn resolved(root: &std::path::Path, relative: &[&str]) -> ResolvedHostCommand {
    ResolvedHostCommand {
        command_id: "test-publish".into(),
        program: PathBuf::from("/trusted/archon"),
        args: Vec::new(),
        cwd: root.to_path_buf(),
        environment: BTreeMap::new(),
        stdin: None,
        timeout_secs: 5,
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
        declared_write_set: relative.iter().map(|path| root.join(path)).collect(),
        remediation_scopes: BTreeSet::from([RemediationScope::Operational]),
        spill_dir: None,
    }
}

fn manifest(call_id: &str, entries: &[(&str, &[u8])]) -> PreparedPublicationV1 {
    PreparedPublicationV1 {
        schema_version: PREPARED_PUBLICATION_SCHEMA_VERSION,
        call_id: call_id.into(),
        command_id: "test-publish".into(),
        entries: entries
            .iter()
            .map(|(path, bytes)| PreparedPublicationEntry {
                relative_path: (*path).into(),
                byte_len: bytes.len() as u64,
                blake3: digest(bytes),
            })
            .collect(),
        expected_acceptance_contract: None,
    }
}

fn acceptance_manifest(
    call_id: &str,
    expected_contract: Option<Option<String>>,
    bytes: &[u8],
) -> PreparedPublicationV1 {
    let mut prepared = manifest(call_id, &[("acceptance-contract.json", bytes)]);
    prepared.command_id = "freeze-acceptance".into();
    prepared.expected_acceptance_contract =
        expected_contract.map(|digest| archon_workflow::v2::ExpectedAcceptanceContract {
            exists: digest.is_some(),
            digest,
        });
    prepared
}

fn audited_acceptance(
    root: &std::path::Path,
    call_id: &str,
    expected_contract: Option<Option<String>>,
    bytes: &[u8],
) -> (
    super::workflow_host_command_publish::AuditedPublication,
    PathBuf,
    PathBuf,
) {
    let staging = prepare_staging(root, call_id).unwrap();
    std::fs::write(staging.root.join("acceptance-contract.json"), bytes).unwrap();
    let command = resolved(&staging.root, &["acceptance-contract.json"]);
    let mut command = command;
    command.command_id = "freeze-acceptance".into();
    let audited = audit_prepared_publication(
        &staging,
        &acceptance_manifest(call_id, expected_contract, bytes),
        &command,
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap();
    let tasks = root.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let destination = tasks.join("acceptance-contract.json");
    let pin = crate::command::workflow_task_set::acceptance_pin_path(root, &tasks);
    (audited, destination, pin)
}

#[test]
fn staged_acceptance_commit_with_unchanged_contract_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let prior = b"current";
    let candidate = b"frozen";
    let tasks = temp.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("acceptance-contract.json"), prior).unwrap();
    let (audited, destination, pin) = audited_acceptance(
        temp.path(),
        "unchanged",
        Some(Some(digest(prior))),
        candidate,
    );
    let receipt = publish_audited(
        audited,
        &BTreeMap::from([("acceptance-contract.json".into(), destination.clone())]),
        &pin,
        &tasks,
    )
    .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), candidate);
    assert_eq!(receipt.entries[0].blake3, digest(candidate));
}

#[test]
fn staged_acceptance_with_a_changed_contract_returns_stale_and_can_be_reprepared() {
    let temp = tempfile::tempdir().unwrap();
    let prior = b"prepared from this";
    let concurrent = b"new live contract";
    let candidate = b"stale freeze";
    let tasks = temp.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let destination = tasks.join("acceptance-contract.json");
    std::fs::write(&destination, prior).unwrap();
    let (stale, _, pin) =
        audited_acceptance(temp.path(), "stale", Some(Some(digest(prior))), candidate);
    std::fs::write(&destination, concurrent).unwrap();
    let error = publish_audited(
        stale,
        &BTreeMap::from([("acceptance-contract.json".into(), destination.clone())]),
        &pin,
        &tasks,
    )
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::command::workflow_task_set::StaleAcceptanceContract>()
            .is_some(),
        "{error:#}"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), concurrent);

    let (refreshed, _, pin) = audited_acceptance(
        temp.path(),
        "refreshed",
        Some(Some(digest(concurrent))),
        b"freeze from refreshed contract",
    );
    publish_audited(
        refreshed,
        &BTreeMap::from([("acceptance-contract.json".into(), destination.clone())]),
        &pin,
        &tasks,
    )
    .expect("a re-prepared freeze commits against the current contract");
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"freeze from refreshed contract"
    );
}

#[test]
fn first_staged_acceptance_commit_accepts_recorded_contract_absence() {
    let temp = tempfile::tempdir().unwrap();
    let candidate = b"first frozen contract";
    let manifest = acceptance_manifest("round-trip", Some(None), candidate);
    let decoded: PreparedPublicationV1 =
        serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(
        decoded.expected_acceptance_contract,
        manifest.expected_acceptance_contract
    );
    let (audited, destination, pin) =
        audited_acceptance(temp.path(), "first", Some(None), candidate);
    let tasks = temp.path().join("tasks");
    publish_audited(
        audited,
        &BTreeMap::from([("acceptance-contract.json".into(), destination.clone())]),
        &pin,
        &tasks,
    )
    .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), candidate);
}

#[test]
fn parent_commit_requires_exact_declared_staged_tree() {
    let temp = tempfile::tempdir().unwrap();
    let staging = prepare_staging(temp.path(), "call-1").unwrap();
    std::fs::write(staging.root.join("one.txt"), b"one").unwrap();
    std::fs::write(staging.root.join("unexpected.txt"), b"unexpected").unwrap();
    let command = resolved(&staging.root, &["one.txt"]);

    let error = audit_prepared_publication(
        &staging,
        &manifest("call-1", &[("one.txt", b"one")]),
        &command,
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("unexpected.txt"), "{error}");
}

#[test]
fn manifest_digest_mismatch_is_operational() {
    let temp = tempfile::tempdir().unwrap();
    let staging = prepare_staging(temp.path(), "call-1").unwrap();
    std::fs::write(staging.root.join("one.txt"), b"changed").unwrap();
    let command = resolved(&staging.root, &["one.txt"]);

    let error = audit_prepared_publication(
        &staging,
        &manifest("call-1", &[("one.txt", b"changes")]),
        &command,
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("digest mismatch"), "{error}");
}

#[test]
fn mutation_sentinel_change_refuses_publication() {
    let temp = tempfile::tempdir().unwrap();
    let staging = prepare_staging(temp.path(), "call-1").unwrap();
    std::fs::write(staging.root.join("one.txt"), b"one").unwrap();
    let protected = temp.path().join("protected.txt");
    std::fs::write(&protected, b"before").unwrap();
    let command = resolved(&staging.root, &["one.txt"]);
    let sentinels = LiveMutationSentinels::capture(std::slice::from_ref(&protected)).unwrap();
    let audited = audit_prepared_publication(
        &staging,
        &manifest("call-1", &[("one.txt", b"one")]),
        &command,
        sentinels,
    )
    .unwrap();
    std::fs::write(&protected, b"after").unwrap();

    let error = publish_audited(
        audited,
        &BTreeMap::new(),
        &temp.path().join("pins/set.json"),
        temp.path(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("mutation sentinel"), "{error}");
}

#[test]
fn receipt_records_exact_live_bytes_after_parent_commit() {
    let temp = tempfile::tempdir().unwrap();
    let staging = prepare_staging(temp.path(), "call-1").unwrap();
    std::fs::write(staging.root.join("one.txt"), b"new bytes").unwrap();
    let live = temp.path().join("live/one.txt");
    std::fs::create_dir_all(live.parent().unwrap()).unwrap();
    std::fs::write(&live, b"old bytes").unwrap();
    let gate = temp
        .path()
        .join(".archon/workflows/run/host-command-results/call/gate-envelope.json");
    std::fs::write(staging.root.join("gate-envelope.json"), b"receipt").unwrap();
    let command = resolved(&staging.root, &["one.txt", "gate-envelope.json"]);
    let audited = audit_prepared_publication(
        &staging,
        &manifest(
            "call-1",
            &[
                ("one.txt", b"new bytes"),
                ("gate-envelope.json", b"receipt"),
            ],
        ),
        &command,
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap();
    let destinations = BTreeMap::from([
        ("one.txt".to_string(), live.clone()),
        ("gate-envelope.json".to_string(), gate.clone()),
    ]);
    let pin = crate::command::workflow_task_set::acceptance_pin_path(
        temp.path(),
        &temp.path().join("live"),
    );

    let receipt = publish_audited(audited, &destinations, &pin, &temp.path().join("live")).unwrap();
    assert_eq!(std::fs::read(&live).unwrap(), b"new bytes");
    assert_eq!(receipt.entries.len(), 2);
    assert_eq!(std::fs::read(&gate).unwrap(), b"receipt");
    let one = receipt
        .entries
        .iter()
        .find(|entry| entry.relative_path == "one.txt")
        .unwrap();
    assert_eq!(one.blake3, digest(b"new bytes"));
    let old_digest = digest(b"old bytes");
    assert_eq!(one.prior_blake3.as_deref(), Some(old_digest.as_str()));
}

#[cfg(unix)]
#[test]
fn staged_symlink_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let staging = prepare_staging(temp.path(), "call-1").unwrap();
    let outside = temp.path().join("outside");
    std::fs::write(&outside, b"outside").unwrap();
    std::os::unix::fs::symlink(&outside, staging.root.join("one.txt")).unwrap();
    let command = resolved(&staging.root, &["one.txt"]);

    let error = audit_prepared_publication(
        &staging,
        &manifest("call-1", &[("one.txt", b"outside")]),
        &command,
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("symlink"), "{error}");
}

/// Issue 271 round 3: a per-check repair may hold the chain lock across model
/// calls; the parent publication waits for it instead of failing the stage.
#[test]
fn parent_publication_waits_for_the_frozen_chain_lock() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("live");
    std::fs::create_dir(&tasks).unwrap();
    let live = tasks.join("one.txt");
    std::fs::write(&live, b"old bytes").unwrap();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), &tasks);
    let chain = crate::command::workflow_task_set::ChainLock::acquire(&pin, &tasks).unwrap();
    let staging = prepare_staging(temp.path(), "call-chain").unwrap();
    std::fs::write(staging.root.join("one.txt"), b"new bytes").unwrap();
    let audited = audit_prepared_publication(
        &staging,
        &manifest("call-chain", &[("one.txt", b"new bytes")]),
        &resolved(&staging.root, &["one.txt"]),
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let destinations = BTreeMap::from([("one.txt".to_string(), live.clone())]);
    let (pin_for_publish, tasks_for_publish) = (pin.clone(), tasks.clone());
    let publisher = std::thread::spawn(move || {
        let result = publish_audited(audited, &destinations, &pin_for_publish, &tasks_for_publish)
            .map(|_| ())
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    assert!(
        receiver
            .recv_timeout(std::time::Duration::from_millis(400))
            .is_err(),
        "publication must wait while a repair holds the chain lock"
    );
    assert_eq!(std::fs::read(&live).unwrap(), b"old bytes");
    drop(chain);
    receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("publication proceeds once the lock is released")
        .unwrap();
    publisher.join().unwrap();
    assert_eq!(std::fs::read(&live).unwrap(), b"new bytes");
}
