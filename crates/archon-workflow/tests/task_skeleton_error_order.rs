//! A tampered frozen chain names its first decisive cause: content digest,
//! then acceptance link, then gate provenance (#532). Every state keeps its
//! accept/refuse verdict; only the reported cause changes.
use std::fs;
use std::path::Path;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin, FreezeGateMode,
    FreezeGateStamp, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::{
    FrozenDependency, FrozenTask, TaskSkeleton, TaskSkeletonLock, validate_full_chain,
};

const ACCEPTANCE: &[u8] = br#"{
  "schema_version":1,
  "prd":{"path":"prds/PRD-X.md","digest":"prd"},
  "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
  "acceptance":[{"id":"AC-X-001","criterion":"done","check":{"kind":"command","command":"sh -c 'exit 0'","cwd":"project_root"},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"attempted","reason":"rejects","host_call_id":"judge-1"}}],
  "supplementary":[]
}"#;

fn stamp(commit: &str) -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Enforce,
        finding_count: 0,
        findings_digest: archon_workflow::task_set_contract::empty_gate_findings_digest(),
        binary_commit: commit.into(),
        evaluated_at: "2026-08-26T18:30:00Z".into(),
    }
}

fn skeleton(acceptance_digest: &str) -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.into(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-010".into(),
            file_name: "TASK-X-010-body.md".into(),
            depends_on: vec![FrozenDependency {
                task_id: "TASK-X-001".into(),
                consumes: Vec::new(),
                ordering_only: true,
            }],
            blocks: Vec::new(),
            implements: vec!["AC-X-001".into()],
            deliverable_contracts: Vec::new(),
        }],
    }
}

fn write_acceptance_lock(root: &Path, digest: &str, gate: FreezeGateStamp) {
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: digest.into(),
        gate,
        baseline_commit: None,
    };
    fs::write(
        root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
}

/// Writes the skeleton and a lock that binds its bytes; returns the digest.
fn write_skeleton(root: &Path, skeleton: &TaskSkeleton, lock_acceptance: &str) -> String {
    let bytes = serde_json::to_vec_pretty(skeleton).unwrap();
    fs::write(root.join(TASK_SKELETON_FILE), &bytes).unwrap();
    let digest = content_digest(&bytes);
    let lock = TaskSkeletonLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        acceptance_digest: lock_acceptance.into(),
        gate: stamp("test-revision"),
    };
    fs::write(
        root.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    digest
}

/// A complete, valid frozen chain and its pin.
fn frozen_chain(root: &Path) -> AcceptancePin {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), ACCEPTANCE).unwrap();
    let acceptance = content_digest(ACCEPTANCE);
    write_acceptance_lock(root, &acceptance, stamp("test-revision"));
    let skeleton_digest = write_skeleton(root, &skeleton(&acceptance), &acceptance);
    AcceptancePin {
        check_sources_digest: None,
        task_root: root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .display()
            .to_string(),
        acceptance_digest: acceptance,
        freeze_event_id: "freeze-1".into(),
        acceptance_gate: stamp("test-revision"),
        skeleton_digest: Some(skeleton_digest),
        skeleton_gate: Some(stamp("test-revision")),
        fidelity_waivers: Vec::new(),
        lineage: Vec::new(),
        lineage_recording: None,
    }
}

fn error_of(root: &Path, pin: &AcceptancePin) -> String {
    validate_full_chain(root, pin)
        .expect_err("a tampered chain is refused")
        .to_string()
}

fn tamper_skeleton_bytes(root: &Path) {
    let path = root.join(TASK_SKELETON_FILE);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\n");
    fs::write(path, bytes).unwrap();
}

#[test]
fn full_chain_digest_failure_precedes_missing_skeleton_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    tamper_skeleton_bytes(&root);
    pin.skeleton_gate = None;
    let error = error_of(&root, &pin);
    assert!(
        error.contains("task skeleton differs from freeze event 'freeze-1'"),
        "{error}"
    );
    assert!(!error.contains("gate provenance"), "{error}");
}

#[test]
fn full_chain_digest_failure_precedes_skeleton_gate_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    tamper_skeleton_bytes(&root);
    pin.skeleton_gate = Some(stamp("other-revision"));
    let error = error_of(&root, &pin);
    assert!(
        error.contains("task skeleton differs from freeze event 'freeze-1'"),
        "{error}"
    );
    assert!(!error.contains("gate provenance"), "{error}");
}

#[test]
fn full_chain_acceptance_digest_failure_precedes_acceptance_gate_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    let mut bytes = ACCEPTANCE.to_vec();
    bytes.extend_from_slice(b"\n");
    fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
    pin.acceptance_gate = stamp("other-revision");
    let error = error_of(&root, &pin);
    assert!(
        error.contains("acceptance chain differs from freeze event 'freeze-1'"),
        "{error}"
    );
    assert!(!error.contains("gate provenance"), "{error}");
}

#[test]
fn full_chain_lock_acceptance_link_precedes_missing_skeleton_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    let digest = write_skeleton(&root, &skeleton(&pin.acceptance_digest), "blake3:stale");
    pin.skeleton_digest = Some(digest);
    pin.skeleton_gate = None;
    let error = error_of(&root, &pin);
    assert!(
        error.contains(&format!(
            "{TASK_SKELETON_LOCK_FILE} acceptance_digest is blake3:stale"
        )),
        "{error}"
    );
    assert!(!error.contains("gate provenance"), "{error}");
}

#[test]
fn full_chain_body_acceptance_link_precedes_missing_skeleton_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    let digest = write_skeleton(&root, &skeleton("blake3:stale"), &pin.acceptance_digest);
    pin.skeleton_digest = Some(digest);
    pin.skeleton_gate = None;
    let error = error_of(&root, &pin);
    assert!(
        error.contains("task skeleton acceptance_digest is blake3:stale"),
        "{error}"
    );
    assert!(!error.contains("gate provenance"), "{error}");
}

#[test]
fn full_chain_provenance_is_reported_after_integrity_and_links() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    validate_full_chain(&root, &pin).expect("the untouched chain verifies");
    pin.skeleton_gate = None;
    let error = error_of(&root, &pin);
    assert!(error.contains("has no skeleton gate provenance"), "{error}");
    pin.skeleton_gate = Some(stamp("other-revision"));
    let error = error_of(&root, &pin);
    assert!(
        error.contains("task skeleton lock/pin gate provenance mismatch"),
        "{error}"
    );
    let mut pin = frozen_chain(&root);
    pin.acceptance_gate = stamp("other-revision");
    let error = error_of(&root, &pin);
    assert!(
        error.contains("acceptance lock/pin gate provenance mismatch"),
        "{error}"
    );
}

#[test]
fn full_chain_pending_skeleton_names_the_missing_freeze() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks");
    let mut pin = frozen_chain(&root);
    // An intact skeleton whose pin has not been frozen yet.
    pin.skeleton_digest = None;
    pin.skeleton_gate = None;
    let error = error_of(&root, &pin);
    assert!(error.contains("workflow freeze-skeleton"), "{error}");
    assert!(error.contains("freeze-1"), "{error}");
}

#[test]
fn full_chain_error_reordering_preserves_every_verdict() {
    type Fault = fn(&Path, &mut AcceptancePin);
    let faults: Vec<(&str, Fault)> = vec![
        ("skeleton bytes", |root, _| tamper_skeleton_bytes(root)),
        ("pin skeleton digest", |_, pin| {
            pin.skeleton_digest = Some("blake3:other".into())
        }),
        ("pin skeleton digest absent", |_, pin| {
            pin.skeleton_digest = None
        }),
        ("skeleton gate absent", |_, pin| pin.skeleton_gate = None),
        ("skeleton gate mismatch", |_, pin| {
            pin.skeleton_gate = Some(stamp("other-revision"))
        }),
        ("acceptance gate mismatch", |_, pin| {
            pin.acceptance_gate = stamp("other-revision")
        }),
        ("acceptance lock gate invalid", |root, pin| {
            let mut gate = stamp("test-revision");
            gate.finding_count = 1;
            write_acceptance_lock(root, &pin.acceptance_digest, gate)
        }),
        ("lock acceptance link", |root, pin| {
            pin.skeleton_digest = Some(write_skeleton(
                root,
                &skeleton(&pin.acceptance_digest),
                "blake3:stale",
            ))
        }),
        ("body acceptance link", |root, pin| {
            pin.skeleton_digest = Some(write_skeleton(
                root,
                &skeleton("blake3:stale"),
                &pin.acceptance_digest,
            ))
        }),
        ("acceptance bytes", |root, _| {
            fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), b"{}").unwrap()
        }),
    ];
    // Every single fault and every pair of faults is refused.
    for (left, first) in faults.iter().enumerate() {
        for second in faults.iter().skip(left) {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("tasks");
            let mut pin = frozen_chain(&root);
            (first.1)(&root, &mut pin);
            (second.1)(&root, &mut pin);
            assert!(
                validate_full_chain(&root, &pin).is_err(),
                "{} + {} must stay refused",
                first.0,
                second.0
            );
        }
    }
}
