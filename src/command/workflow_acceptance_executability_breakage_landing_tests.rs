use super::tests::Fixture;
use super::*;
use archon_workflow::base_snapshot::{SnapshotEntry, UnverifiedReason};
fn fixture() -> Fixture {
    let mut f = Fixture::new("test \"$(cat ignored)\" = good");
    std::fs::write(f.source.path().join("ignored"), b"good\n").unwrap();
    f.classes.entries[0].deliverables[0].artifact_path = "ignored".into();
    f.classes.entries[0].deliverables[0].landing_verified = true;
    let snapshot = f.classes.base_snapshot.as_mut().unwrap();
    let digest = content_digest(b"good\n");
    snapshot.entries.insert(
        "ignored".into(),
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::Ignored,
            live_digest: Some(digest.clone()),
            live_note: None,
        },
    );
    snapshot.landing_verified.insert("ignored".into(), digest);
    snapshot.digest = snapshot.recomputed_digest();
    for mutant in &mut f.mutants.mutants {
        mutant.patch = mutant
            .patch
            .replace("a/value", "a/ignored")
            .replace("b/value", "b/ignored");
    }
    f
}
#[tokio::test]
async fn landing_verified_ignored_bytes_are_counted_and_reverted() {
    let f = fixture();
    let report = f.prove(&f.probe(false), 20).await;
    assert!(
        report.proofs.runs.values().any(|r| r.counted),
        "{:?}",
        report.proofs.summary
    );
    assert!(report.proofs.controls.values().all(|c| c.stable()));
    assert_eq!(
        std::fs::read(f.source.path().join("ignored")).unwrap(),
        b"good\n"
    );
}
#[test]
fn landing_verified_breaker_view_uses_rehashed_live_bytes() {
    let f = fixture();
    let keys = selection::keys(&f.classes.entries[0], f.snapshot(), f.source.path(), false);
    let view = super::super::breaker_view::materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &keys,
        &[],
    )
    .unwrap();
    assert_eq!(view.files["ignored"].bytes, b"good\n");
    std::fs::write(f.source.path().join("ignored"), b"changed\n").unwrap();
    assert!(
        super::super::breaker_view::materialise_view(
            f.parent.path(),
            f.source.path(),
            f.snapshot(),
            &keys,
            &[]
        )
        .is_err()
    );
    assert!(
        materialise_base_copy(
            f.parent.path(),
            f.source.path(),
            f.source.path(),
            None,
            f.snapshot(),
            &f.snapshot().commit,
            &f.parent.path().join("build"),
            None
        )
        .is_err()
    );
}

#[tokio::test]
async fn scratch_site_proves_with_policy_environment_and_parent() {
    let f = Fixture::new("test \"$EXPECTED\" = good && test \"$(cat value)\" = good");
    let mut probe = f.probe(false);
    let policy = archon_workflow::acceptance_scratch::ScratchPolicy {
        task_root: f.source.path().into(),
        repository: f.source.path().into(),
        project: f.source.path().into(),
        scratch_parent: f.parent.path().into(),
        project_inputs: vec![],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: [("EXPECTED".into(), "good".into())].into(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 20,
        output_bytes: 8192,
        scratch_bytes: 16777216,
        build_cache: None,
    };
    probe.site = Site::Scratch(Box::new(NativeBinding {
        policy,
        source_commit: f.snapshot().commit.clone(),
        external_data_roots: vec![],
    }));
    let report = f.prove(&probe, 20).await;
    assert!(
        report.proofs.runs.values().any(|r| r.counted),
        "{:?}",
        report.proofs.summary
    );
    probe.site = Site::Direct;
    assert_eq!(
        breakage_refusal(
            &probe,
            &BreakageRequest {
                contract: &f.contract,
                chain_digest: &f.lock.digest,
                lock: &f.lock,
                classification: &f.classes,
                mutants: &f.mutants,
                max_runs: 20,
                min_deadline_secs: 2
            }
        ),
        Some("proof_site_unavailable")
    );
}
