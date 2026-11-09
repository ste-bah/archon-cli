use super::super::anchor::{Anchor, AnchoredPair, prove_anchored};
use super::*;

fn tip_snapshot(f: &Fixture) -> BaseSnapshot {
    std::fs::write(f.source.path().join("value"), b"tip\n").unwrap();
    git(f.source.path(), &["add", "value"]);
    git(f.source.path(), &["commit", "-qm", "tip"]);
    let commit = git(f.source.path(), &["rev-parse", "HEAD"]);
    assert_ne!(Some(&commit), f.lock.baseline_commit.as_ref());
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: f.source.path().display().to_string(),
        base_commit: commit,
        decomposition_run_id: "test".into(),
        recorded_at: "test".into(),
    })
    .unwrap();
    snapshot_deliverables(&tree, f.source.path(), ["value", "other", "absent"]).unwrap()
}

fn anchor<'a>(f: &'a Fixture, snapshot: &'a BaseSnapshot) -> Anchor<'a> {
    let mut mutants = f.mutants.mutants.clone();
    for mutant in &mut mutants {
        mutant.patch = mutant.patch.replace("good", "tip");
    }
    Anchor {
        contract: &f.contract,
        chain_digest: &f.lock.digest,
        commit: &snapshot.commit,
        snapshot,
        pairs: vec![AnchoredPair {
            task_id: "task".into(),
            id: "item".into(),
            checks: vec!["check".into()],
            countable: vec![(
                "value".into(),
                archon_workflow::base_snapshot::SnapshotKind::Blob,
            )],
            shared: Vec::new(),
            state: None,
        }],
        mutants,
        max_runs: 10,
        min_deadline_secs: 1,
        store_root: Some(f.parent.path().join("provided-store")),
    }
}

#[tokio::test]
async fn anchor_at_tip_counts_kills_survivors_and_shared_touches() {
    for mode in ["kill", "survivor", "shared"] {
        let f = Fixture::new(if mode == "survivor" {
            "test -f value"
        } else {
            "test \"$(cat value)\" = tip"
        });
        let snapshot = tip_snapshot(&f);
        let mut probe = f.probe(false);
        // A direct caller still executes its observations in hermetic copies.
        probe.site = Site::Direct;
        let mut req = anchor(&f, &snapshot);
        if mode == "shared" {
            req.pairs[0].shared = std::mem::take(&mut req.pairs[0].countable);
            req.pairs[0].countable = vec![(
                "other".into(),
                archon_workflow::base_snapshot::SnapshotKind::Blob,
            )];
        }
        let report = prove_anchored(&probe, req, Vec::new()).await;
        assert!(report.refusals.is_empty(), "{:?}", report.refusals);
        assert_eq!(report.runs_executed, 4);
        assert_eq!(report.runs_reused, 0);
        assert_eq!(report.runs.len(), 2);
        assert!(report.controls.values().all(|c| c.stable()));
        assert_eq!(
            report.entries[0].state,
            match mode {
                "kill" => ProofState::BehaviourKill,
                "survivor" => ProofState::SurvivorsOnly,
                _ => ProofState::NoViableMutant,
            }
        );
        for run in report.runs.values() {
            match mode {
                "kill" => assert!(run.counted && matches!(run.outcome, RunOutcome::Killed { .. })),
                "survivor" => {
                    assert!(!run.counted && matches!(run.outcome, RunOutcome::Survived { .. }))
                }
                _ => {
                    assert!(matches!(run.outcome, RunOutcome::Killed { .. }));
                    assert!(!run.counted);
                    assert_eq!(run.not_counted, Some(NotCounted::TouchesShared));
                }
            }
            assert!(
                f.parent
                    .path()
                    .join("provided-store")
                    .join(&run.evidence.patch)
                    .is_file()
            );
        }
        let mut req = anchor(&f, &snapshot);
        req.max_runs = 0;
        // Current ownership is recomputed even for evidence in the supplied store.
        let reused = prove_anchored(&probe, req, Vec::new()).await;
        assert_eq!((reused.runs_executed, reused.runs_reused), (0, 2));
        assert_eq!(
            reused.entries[0].state,
            if mode == "survivor" {
                ProofState::SurvivorsOnly
            } else {
                ProofState::BehaviourKill
            }
        );
        assert!(
            reused
                .runs
                .values()
                .all(|r| r.counted == (mode != "survivor"))
        );
        assert_eq!(
            std::fs::read(f.source.path().join("value")).unwrap(),
            b"tip\n"
        );
        assert_eq!(f.lock.baseline_commit, f.classes.baseline_commit);
    }
}

#[tokio::test]
async fn anchor_refuses_mismatched_baseline_snapshot_and_scratch() {
    for mode in ["baseline", "digest", "commit", "scratch"] {
        let f = Fixture::new("test -f value");
        let mut snapshot = tip_snapshot(&f);
        let mut probe = f.probe(false);
        let expected = match mode {
            "baseline" => {
                probe.baseline = None;
                "probe_baseline_mismatch"
            }
            "digest" => {
                snapshot.digest.clear();
                "snapshot_invalid"
            }
            "commit" => "snapshot_invalid",
            _ => "proof_site_unavailable",
        };
        if mode == "scratch" {
            probe.site = Site::Scratch(Box::new(
                crate::command::acceptance_scratch_policy::NativeBinding {
                    policy: archon_workflow::acceptance_scratch::ScratchPolicy {
                        repository: f.source.path().into(),
                        project: f.source.path().into(),
                        task_root: f.source.path().into(),
                        scratch_parent: f.parent.path().into(),
                        project_inputs: Vec::new(),
                        project_input_excludes: Vec::new(),
                        combined: false,
                        toolchain_path: String::new(),
                        environment: Default::default(),
                        environment_allowlist: Vec::new(),
                        cargo_seed: None,
                        timeout_secs: 60,
                        output_bytes: 8192,
                        scratch_bytes: 16 << 20,
                        build_cache: None,
                    },
                    source_commit: snapshot.commit.clone(),
                    external_data_roots: Vec::new(),
                },
            ));
        }
        let mut req = anchor(&f, &snapshot);
        if mode == "commit" {
            req.commit = f.lock.baseline_commit.as_deref().unwrap();
        }
        let report = prove_anchored(&probe, req, Vec::new()).await;
        assert!(
            report.refusals.iter().any(|r| r == expected),
            "{:?}",
            report.refusals
        );
        assert_eq!((report.runs_executed, report.runs_reused), (0, 0));
        assert!(report.runs.is_empty());
        assert_eq!(report.entries[0].state, ProofState::HostUnrun);
    }
}

#[tokio::test]
async fn anchor_wrapper_keeps_complete_summary_and_refusal_order() {
    for mode in ["missing", "schema", "empty"] {
        let mut f = Fixture::new("test -f value");
        let probe = f.probe(false);
        let refusals = if mode == "empty" {
            f.mutants.mutants.clear();
            Vec::new()
        } else {
            f.lock.baseline_commit = None;
            f.mutants.base_commit.clear();
            if mode == "schema" {
                f.mutants.schema = 0;
            }
            vec![
                if mode == "schema" {
                    "mutant_set_schema"
                } else {
                    "mutant_commit_mismatch"
                }
                .into(),
                "classification_stale".into(),
            ]
        };
        let result = f.prove(&probe, 10).await;
        assert!(result.proofs.controls.is_empty());
        assert!(result.proofs.runs.is_empty());
        assert_eq!(
            result.proofs.base_commit,
            f.lock.baseline_commit.clone().unwrap_or_default()
        );
        assert_eq!(
            result.proofs.summary,
            Some(ProofSummary {
                acceptance_digest: f.lock.digest.clone(),
                baseline_results_digest: results_digest(&f.lock.baseline_results),
                snapshot_digest: f.snapshot().digest.clone(),
                runs_executed: 0,
                runs_reused: 0,
                refusals,
                entries: vec![RequirementProof {
                    task_id: "task".into(),
                    id: "item".into(),
                    checks: vec!["check".into()],
                    state: if mode == "empty" {
                        ProofState::NoMutants
                    } else {
                        ProofState::HostUnrun
                    },
                    counted_kills: Vec::new(),
                }],
            })
        );
    }
}

#[tokio::test]
async fn anchor_wrapper_deduplicates_unreadable_summary_and_run() {
    let f = Fixture::new("test \"$(cat value)\" = good");
    let probe = f.probe(true);
    let first = f.prove(&probe, 10).await;
    assert_eq!(first.proofs.runs.len(), 2);
    let root = first.store.unwrap();
    let summary = store::summary_path(&root, &f.lock.digest).unwrap();
    let run = std::fs::read_dir(root.join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(&summary, b"corrupt").unwrap();
    std::fs::write(run, b"corrupt").unwrap();
    let result = f.prove(&probe, 10).await;
    assert_eq!(
        result.proofs.summary.as_ref().unwrap().refusals,
        ["proofs_unreadable"]
    );
    let saved: BreakageProofs = serde_json::from_slice(&std::fs::read(summary).unwrap()).unwrap();
    assert_eq!(saved.summary, result.proofs.summary);
}
