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
    }
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

    let error = publish_audited(audited, &BTreeMap::new()).unwrap_err();
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
    let command = resolved(&staging.root, &["one.txt"]);
    let audited = audit_prepared_publication(
        &staging,
        &manifest("call-1", &[("one.txt", b"new bytes")]),
        &command,
        LiveMutationSentinels::capture(&[]).unwrap(),
    )
    .unwrap();
    let destinations = BTreeMap::from([("one.txt".to_string(), live.clone())]);

    let receipt = publish_audited(audited, &destinations).unwrap();
    assert_eq!(std::fs::read(&live).unwrap(), b"new bytes");
    assert_eq!(receipt.entries.len(), 1);
    assert_eq!(receipt.entries[0].blake3, digest(b"new bytes"));
    let old_digest = digest(b"old bytes");
    assert_eq!(
        receipt.entries[0].prior_blake3.as_deref(),
        Some(old_digest.as_str())
    );
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
