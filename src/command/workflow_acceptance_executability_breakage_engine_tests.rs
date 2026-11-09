use super::*;
#[tokio::test]
async fn attribution_shared_outside_and_mixed() {
    for mode in ["shared", "outside", "mixed"] {
        let mut f = Fixture::new("test \"$(cat value)\" = good && test \"$(cat other)\" = base");
        if mode == "shared" {
            f.classes.entries[0].deliverables[0].shared = true;
            f.classes.entries[0].deliverables.push(
                archon_workflow::v2::requirement_classes::DeliverablePresence {
                    wiring_root: false,
                    landing_verified: false,
                    artifact_path: "other".into(),
                    error: None,
                    at_base: true,
                    live_only: false,
                    shared: false,
                    text: false,
                },
            );
            let object = git(f.source.path(), &["rev-parse", "HEAD:other"]);
            let snapshot = f.classes.base_snapshot.as_mut().unwrap();
            snapshot.entries.insert(
                "other".into(),
                archon_workflow::base_snapshot::SnapshotEntry::Tracked {
                    kind: archon_workflow::base_snapshot::SnapshotKind::Blob,
                    object,
                    live: archon_workflow::base_snapshot::LiveState::Same,
                    live_note: None,
                },
            );
            snapshot.digest = snapshot.recomputed_digest();
        } else {
            let outside = f.mutants.mutants[0]
                .patch
                .replace("value", "other")
                .replace("good", "base");
            if mode == "outside" {
                f.mutants.mutants[0].patch = outside;
            } else {
                f.mutants.mutants[0].patch.push_str(&outside);
            }
        }
        let r = f.prove(&f.probe(false), 10).await;
        let run = r
            .proofs
            .runs
            .values()
            .find(|r| r.kind == MutantKind::Behaviour)
            .unwrap();
        assert!(
            matches!(run.outcome, RunOutcome::Killed { .. }),
            "{mode}: {run:?}"
        );
        assert!(!run.counted);
        assert_eq!(
            run.not_counted,
            Some(if mode == "shared" {
                NotCounted::TouchesShared
            } else {
                NotCounted::TouchesOutsideDeliverables
            })
        );
    }
}
#[tokio::test]
async fn refusal_and_host_failures_return_normally() {
    for mode in [
        "commit",
        "stale",
        "baseline",
        "scratch",
        "materialise",
        "work",
    ] {
        let mut f = Fixture::new("test -f value");
        let mut p = f.probe(false);
        let expected = match mode {
            "commit" => {
                f.mutants.base_commit = "different".into();
                "mutant_commit_mismatch"
            }
            "stale" => {
                f.classes.acceptance_digest = "different".into();
                "classification_stale"
            }
            "baseline" => {
                p.baseline = None;
                "probe_baseline_mismatch"
            }
            "scratch" => {
                p.site = Site::Direct;
                "proof_site_unavailable"
            }
            "materialise" => {
                let snapshot = f.classes.base_snapshot.as_mut().unwrap();
                if let archon_workflow::base_snapshot::SnapshotEntry::Tracked { object, .. } =
                    snapshot.entries.get_mut("value").unwrap()
                {
                    *object = "0".repeat(40);
                }
                snapshot.digest = snapshot.recomputed_digest();
                ""
            }
            _ => {
                let file = f.parent.path().join("file");
                std::fs::write(&file, b"file").unwrap();
                p.copy_parent = file;
                ""
            }
        };
        let r = f.prove(&p, 10).await;
        if !expected.is_empty() {
            assert_eq!(
                r.proofs.summary.as_ref().unwrap().entries[0].state,
                ProofState::HostUnrun,
                "{mode}"
            );
            assert!(
                r.proofs
                    .summary
                    .unwrap()
                    .refusals
                    .iter()
                    .any(|c| c == expected),
                "{mode}"
            );
        } else if mode == "work" || mode == "materialise" {
            assert!(!r.proofs.runs.is_empty(), "{mode}");
            assert_eq!(
                r.proofs.summary.as_ref().unwrap().runs_executed,
                0,
                "{mode}"
            );
            assert!(
                r.proofs
                    .runs
                    .values()
                    .all(|r| matches!(r.outcome, RunOutcome::Unrun { .. })),
                "{mode}: {:?}",
                r.proofs
            );
        }
    }
}
#[tokio::test]
async fn deadline_then_next_mutant_and_absent_cleanup() {
    deadline_then_next_mutant_and_absent_cleanup_case().await;
}
async fn deadline_then_next_mutant_and_absent_cleanup_case() {
    let marker = tempfile::tempdir().unwrap();
    let pid_path = marker.path().join("pid");
    let mut f = Fixture::new(&format!(
        "printf present > absent; if test \"$(cat value)\" = evil; then sleep 30 & printf '%s' \"$!\" > '{}'; wait; fi; test \"$(cat value)\" = good",
        pid_path.display()
    ));
    let hanging = f.mutants.mutants[0].mutant_id();
    let next = (0..1000)
        .find_map(|n| {
            let mut mutant = f.mutants.mutants[0].clone();
            mutant.patch = mutant.patch.replace("evil", &format!("next{n}"));
            (mutant.mutant_id() > hanging).then_some(mutant)
        })
        .unwrap();
    let next_id = next.mutant_id();
    f.mutants.mutants.push(next);
    let p = f.probe(false).with_check_cap(2);
    let start = std::time::Instant::now();
    let r = f.prove(&p, 10).await;
    assert!(start.elapsed().as_secs() < 10);
    assert!(
        r.proofs
            .runs
            .values()
            .any(|r| matches!(r.outcome, RunOutcome::TimedOut { .. })),
        "{:?}",
        r.proofs
    );
    assert!(
        r.proofs
            .runs
            .values()
            .any(|r| matches!(r.outcome, RunOutcome::Killed { .. })),
        "{:?}",
        r.proofs
    );
    assert!(r.proofs.controls.values().all(|c| c.stable()));
    assert_eq!(r.proofs.summary.as_ref().unwrap().runs_executed, 5);
    assert!(
        r.proofs
            .runs
            .values()
            .any(|r| r.mutant_id == next_id && matches!(r.outcome, RunOutcome::Killed { .. }))
    );
    let pid: u32 = std::fs::read_to_string(pid_path).unwrap().parse().unwrap();
    for _ in 0..10 {
        if !archon_workflow::process_liveness::process_alive(pid) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!archon_workflow::process_liveness::process_alive(pid));
}
#[tokio::test]
async fn unstable_controls_void_kills_and_unviable_locations() {
    let marker = tempfile::tempdir().unwrap();
    let path = marker.path().join("marker");
    let command = format!(
        "if test -f '{}'; then exit 1; fi; if test \"$(cat value)\" != good; then touch '{}'; exit 1; fi",
        path.display(),
        path.display()
    );
    let f = Fixture::new(&command);
    let r = f.prove(&f.probe(false), 10).await;
    assert!(
        r.proofs
            .runs
            .values()
            .filter(|r| matches!(r.outcome, RunOutcome::Killed { .. }))
            .all(|r| !r.counted && r.not_counted == Some(NotCounted::ControlUnstable))
    );
    assert!(
        r.proofs
            .controls
            .values()
            .all(|c| c.post.is_some() && !c.stable())
    );
    for (path, class) in [
        ("value", LocatedClass::Mutated),
        ("other", LocatedClass::CheckNamed),
        (".gitignore", LocatedClass::Elsewhere),
    ] {
        let command = format!(
            "test -f other; if test \"$(cat value)\" != good; then printf '{path}:3:4: undefined symbol\\n'; exit 1; fi"
        );
        let f = Fixture::new(&command);
        let r = f.prove(&f.probe(false), 10).await;
        for r in r.proofs.runs.values() {
            assert!(
                matches!(&r.outcome,RunOutcome::Unviable{located} if located.iter().any(|l|l.class==class)),
                "{r:?}"
            );
        }
    }
}
#[tokio::test]
async fn changed_mutant_only_and_current_ownership_recomputed() {
    let mut f = Fixture::new("test \"$(cat value)\" = good");
    let p = f.probe(true);
    f.prove(&p, 10).await;
    f.mutants.mutants[0].patch = f.mutants.mutants[0].patch.replace("evil", "nope");
    let r = f.prove(&p, 10).await;
    let s = r.proofs.summary.unwrap();
    assert_eq!((s.runs_executed, s.runs_reused), (3, 1));
    // Same execution evidence, different current ownership: no stale counted flag.
    f.classes.entries[0].deliverables[0].shared = true;
    let skipped = f.prove(&p, 10).await;
    assert_eq!(
        skipped.proofs.summary.unwrap().entries[0].state,
        ProofState::NoCountableDeliverable
    );
    f.classes.entries[0].deliverables.push(
        archon_workflow::v2::requirement_classes::DeliverablePresence {
            wiring_root: false,
            landing_verified: false,
            artifact_path: "other".into(),
            error: None,
            at_base: true,
            live_only: false,
            shared: false,
            text: false,
        },
    );
    let snapshot = f.classes.base_snapshot.as_mut().unwrap();
    snapshot.entries.insert(
        "other".into(),
        archon_workflow::base_snapshot::SnapshotEntry::Tracked {
            kind: archon_workflow::base_snapshot::SnapshotKind::Blob,
            object: git(f.source.path(), &["rev-parse", "HEAD:other"]),
            live: archon_workflow::base_snapshot::LiveState::Same,
            live_note: None,
        },
    );
    snapshot.digest = snapshot.recomputed_digest();
    let r = f.prove(&p, 0).await;
    assert_eq!(r.proofs.summary.unwrap().runs_executed, 0);
    assert!(
        r.proofs
            .runs
            .values()
            .all(|r| !r.counted && r.not_counted == Some(NotCounted::TouchesShared))
    );
}
#[tokio::test]
async fn three_mutants_share_copy_with_sibling_and_absent_key() {
    let mut f = Fixture::new("test -f value");
    let sibling = tempfile::Builder::new()
        .prefix("breakage-sibling-")
        .tempdir_in(f.source.path().parent().unwrap())
        .unwrap();
    std::fs::write(sibling.path().join("input"), b"sibling").unwrap();
    let command = format!(
        "printf made > absent; test -f '../{}/input' && test \"$(cat value)\" = good",
        sibling.path().file_name().unwrap().to_string_lossy()
    );
    if let AcceptanceCheck::Command { command: c, .. } = &mut f.contract.acceptance[0].check {
        *c = command;
    }
    f.lock
        .baseline_results
        .get_mut("check")
        .unwrap()
        .check_digest = check_digest(&f.contract.acceptance[0].check);
    f.classes.baseline_results_digest = results_digest(&f.lock.baseline_results);
    let mut third = f.mutants.mutants[0].clone();
    third.patch = third.patch.replace("evil", "nope");
    f.mutants.mutants.push(third);
    let r = f.prove(&f.probe(false), 10).await;
    assert_eq!(r.proofs.summary.unwrap().runs_executed, 5);
    assert_eq!(r.proofs.runs.len(), 3);
    assert!(
        r.proofs.runs.values().all(|r| r.counted),
        "{:?}",
        r.proofs.runs
    );
    assert!(r.proofs.controls.values().all(|c| c.stable()));
    assert_eq!(
        std::fs::read(sibling.path().join("input")).unwrap(),
        b"sibling"
    );
}
#[tokio::test]
async fn poisoned_copy_is_replaced_and_its_group_is_not_cached() {
    let mut f = Fixture::new(
        "if test \"$(cat value)\" = evil; then printf corrupt > .git/index; exit 1; fi; test \"$(cat value)\" = good",
    );
    let first = f.mutants.mutants[0].mutant_id();
    let third = (0..1000)
        .find_map(|n| {
            let mut m = f.mutants.mutants[0].clone();
            m.patch = m.patch.replace("evil", &format!("next{n}"));
            (m.mutant_id() > first).then_some(m)
        })
        .unwrap();
    f.mutants.mutants.push(third);
    let r = f.prove(&f.probe(true), 10).await;
    assert!(
        r.proofs
            .runs
            .values()
            .any(|r| matches!(&r.outcome,RunOutcome::Unrun{code,..} if code=="copy_poisoned")),
        "{:?}",
        r.proofs.runs
    );
    assert!(
        r.proofs
            .controls
            .values()
            .all(|c| matches!(c.post, Some(RunOutcome::Passed { .. })))
    );
    assert!(!r.store.unwrap().join("runs").exists());
    assert!(!std::fs::read_dir(f.parent.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(COPY_PREFIX)
    }));
}
#[tokio::test]
async fn missing_post_control_status_is_host_unrun() {
    let marker = tempfile::tempdir().unwrap();
    let path = marker.path().join("marker");
    let command = format!(
        "if test -f '{}'; then kill -KILL $$; fi; if test \"$(cat value)\" != good; then touch '{}'; exit 1; fi",
        path.display(),
        path.display()
    );
    let mut f = Fixture::new(&command);
    f.mutants.mutants.truncate(1);
    let r = f.prove(&f.probe(false), 10).await;
    assert_eq!(
        r.proofs.summary.unwrap().entries[0].state,
        ProofState::HostUnrun
    );
    assert!(
        r.proofs
            .controls
            .values()
            .all(|c| matches!(c.post, Some(RunOutcome::NoExitStatus)))
    );
    assert!(
        r.proofs
            .runs
            .values()
            .all(|r| matches!(r.outcome, RunOutcome::Killed { .. }) && !r.counted)
    );
}
#[tokio::test]
async fn unavailable_base_is_not_a_fetch_transport_failure() {
    let f = Fixture::new("test -f value");
    let p = f.probe(false);
    let commit = f.lock.baseline_commit.as_ref().unwrap();
    std::fs::remove_file(
        f.source
            .path()
            .join(".git")
            .join("objects")
            .join(&commit[..2])
            .join(&commit[2..]),
    )
    .unwrap();
    let r = f.prove(&p, 10).await;
    assert_eq!(r.proofs.summary.unwrap().runs_executed, 0);
    assert!(r.proofs.runs.values().all(
        |r| matches!(&r.outcome, RunOutcome::Unrun {code,..} if code == "baseline_unavailable")
    ));
}

#[tokio::test]
async fn tracked_control_writes_are_restored_before_each_mutant() {
    for command in [
        "printf changed > other; test \"$(cat value)\" = good",
        "test \"$(cat other)\" = base || exit 1; printf changed > other; test \"$(cat value)\" = good",
        "printf changed > other; printf made > added; test \"$(cat value)\" = good",
    ] {
        let f = Fixture::new(command);
        let r = f.prove(&f.probe(false), 10).await;
        assert!(r.proofs.runs.values().all(|r| r.counted), "{:?}", r.proofs);
        assert!(
            r.proofs
                .controls
                .values()
                .all(|c| c.stable() && c.restored_runs >= 3)
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 5)]
async fn deadline_five_parallel_repetitions() {
    let mut tasks = Vec::new();
    for _ in 0..5 {
        tasks.push(tokio::spawn(
            deadline_then_next_mutant_and_absent_cleanup_case(),
        ));
    }
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::test]
async fn explicit_errors_from_an_inner_directory_resolve_without_kills() {
    for diagnostic in [
        "a.ts:3:5 - error TS2304: missing",
        "[ERROR] a.java:[5,10] cannot find symbol",
        "a:3:5: fatal broken",
    ] {
        let command = format!(
            "mkdir -p sub; printf source > sub/a.ts; printf source > sub/a.java; printf source > sub/a; if test \"$(cat value)\" != good; then cd sub && printf '%s\\n' '{diagnostic}'; exit 1; fi"
        );
        let f = Fixture::new(&command);
        let r = f.prove(&f.probe(false), 10).await;
        assert!(!r.proofs.runs.is_empty());
        assert!(r.proofs.runs.values().all(|r| matches!(&r.outcome, RunOutcome::Unviable { located } if located.iter().all(|l| l.class == LocatedClass::CheckNamed))), "{:?}", r.proofs);
    }
}

#[tokio::test]
async fn zero_work_mutants_are_not_host_failures() {
    let f = Fixture::new(
        "if test \"$(cat value)\" != good; then printf 'running 0 tests\\ntest result: ok. 0 passed; 0 failed\\n'; else test -f value; fi",
    );
    let r = f.prove(&f.probe(false), 10).await;
    assert!(
        r.proofs
            .runs
            .values()
            .all(|r| matches!(&r.outcome, RunOutcome::Unrun { code, .. } if code == "zero_work")),
        "{:?}",
        r.proofs
    );
    assert_eq!(
        r.proofs.summary.unwrap().entries[0].state,
        ProofState::NoViableMutant
    );
}
