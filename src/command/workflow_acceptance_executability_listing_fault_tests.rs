use std::path::Path;
use std::time::Duration;

use super::super::super::{ExecutabilityProbe, HostProbe, HostUnproven, UnprovenCause, silent};
use super::super::no_verdict_cause;
use super::tests::{Bin, result};
use super::tests_333::{STALL, warm};
use super::tests_333b::contract;

#[test]
fn silent_listing_scratch_overrun_names_the_host_cause_without_claiming_site_absence() {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let script = "#!/bin/sh\n[ \"$1\" = --list ] || exit 0\n/usr/bin/head -c 17000000 /dev/zero > fill\nexec /bin/sleep 30\n";
        let bin = Bin::with_host(
            Path::new("/bin/sh"),
            &[("fixture", script)],
            &[("fixture-item", "#!/bin/sh\nexit 0\n")],
        );
        warm(&bin, &["fixture"]);
        let mut run = result(Some(1), b"", b"error: unknown command 'item'\n");
        run.acceptance_id = "AC-X-001".into();
        let (why, cause) = no_verdict_cause("fixture item", &run, &bin.context()).unwrap();
        assert_eq!(cause, UnprovenCause::SiteLacks, "{why}");
        let expected = "the host could not tell whether `item` is built into `fixture`";
        let stopped = "wrote more than 16777216 bytes to its working directory without printing and was stopped";
        let dependency = "the check must not depend on `fixture item`";
        for why in [why, bin.warned("fixture item").remove(0)] {
            assert!(
                why.contains(expected) && why.contains(stopped) && why.contains(dependency),
                "{why}"
            );
            assert!(!why.contains("site lacks"), "{why}");
            assert!(!why.contains("site does not have"), "{why}");
        }
        send.send(()).unwrap();
    });
    receive
        .recv_timeout(Duration::from_secs(28))
        .expect("scratch listing regression completed");
}

#[test]
fn listing_without_output_is_a_retryable_host_fault_while_partial_output_keeps_its_verdict() {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let script = "#!/bin/sh\n[ \"$1\" = --list ] || exit 0\nprintf 'x\\n' >> \"$RECORD\"\ncase \"$MODE\" in\nstdout) printf 'Commands:\\n' ;;\nstderr) printf 'pending' >&2 ;;\nesac\n/bin/sleep 3\nprintf '    item    Built in\\n'\n";
        let bin = Bin::with_host(
            Path::new("/bin/sh"),
            &[("fixture", script)],
            &[("fixture-item", "#!/bin/sh\nexit 0\n")],
        );
        // Warm the exact executable before timing the deliberate listing delay.
        warm(&bin, &["fixture"]);
        let mut run = result(Some(1), b"", b"error: unknown command 'item'\n");
        run.acceptance_id = "AC-X-001".into();
        let mut check = contract("fixture item");
        check.acceptance[0].id = "AC-X-001".into();
        let root = tempfile::tempdir().unwrap();
        let record = tempfile::NamedTempFile::new().unwrap();
        for mode in ["stdout", "stderr", "silent"] {
            let mut at =
                bin.context_with(&[("MODE", mode), ("RECORD", record.path().to_str().unwrap())]);
            at.list_stall = STALL;
            let (why, cause) = no_verdict_cause("fixture item", &run, &at).unwrap();
            if mode != "silent" {
                assert_eq!(cause, UnprovenCause::SiteLacks, "{mode}: {why}");
                assert!(why.contains("must not depend on it"), "{mode}: {why}");
                continue;
            }
            assert_ne!(cause, UnprovenCause::SiteLacks, "{why}");
            assert_ne!(cause, UnprovenCause::ProgramAbsent, "{why}");
            assert!(
                why.contains("host could not tell") && why.contains("no output"),
                "{why}"
            );
            assert!(!why.contains("must not depend on it"), "{why}");
            // Each resume gets a new probe over the same durable state. A
            // listing failure is never evidence against the check's author.
            for _ in 0..2 {
                let probe = HostProbe::at(root.path().into(), root.path().into(), None);
                assert!(
                    silent::settle_as(&probe, &"0".repeat(40), &check, &run, (&why, cause), &at)
                        .is_none()
                );
                assert_eq!(probe.resume.progress.total(), 0, "no verdict was saved");
                let held = probe.take_unproven_causes();
                assert_eq!(held["AC-X-001"].1, cause);
                let error =
                    HostUnproven(held.into_iter().map(|(id, (why, _))| (id, why)).collect());
                let error =
                    crate::command::workflow_task_set::findings::unproven_incomplete(error.into());
                let incomplete =
                    crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
                        .unwrap();
                assert!(incomplete.report().contains("no output"));
                assert!(!incomplete.is_authored_judge_reply());
            }
            let warnings = super::unresolved_on_path(&["fixture item"], &at);
            assert_eq!(warnings[0].len(), 1);
            assert!(warnings[0][0].contains("no output"), "{warnings:?}");
            assert!(
                !warnings[0][0].contains("must not depend on it"),
                "{warnings:?}"
            );
            // No silent stop is cached: a later listing really tries again.
            assert!(
                no_verdict_cause("fixture item", &run, &at)
                    .unwrap()
                    .0
                    .contains("no output")
            );
            assert_eq!(
                std::fs::read_to_string(record.path())
                    .unwrap()
                    .lines()
                    .count(),
                5
            );
        }
        send.send(()).unwrap();
    });
    receive
        .recv_timeout(Duration::from_secs(28))
        .expect("listing regression completed");
}

#[test]
fn mutation_proof_keeps_an_inconclusive_listing_with_the_host() {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use super::super::super::{
            Baseline, contract_digest, hermetic::CheckEvidence, mutation, prove, repairs::TreeRun,
        };
        let script = "#!/bin/sh\nif [ \"$1\" = --list ]; then /bin/sleep 15; printf '    item    Built in\\n'; exit 0; fi\n[ -f \"$2\" ] && exit 0\nprintf \"error: unknown command 'item'\\n\" >&2\nexit 1\n";
        let bin = Bin::new(Path::new("/bin/sh"), &[("fixture", script)]);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("data")).unwrap();
        std::fs::write(root.path().join("data/state.txt"), "ready").unwrap();
        let command = format!(
            "'{}' item data/state.txt",
            bin.dir.path().join("fixture").display()
        );
        let mut check = contract(&command);
        check.acceptance[0].id = "AC-X-001".into();
        let markers = mutation::Markers::new();
        let wrapped = mutation::wrap(&command, &["data/state.txt".into()], &[], &markers);
        let observed = mutation::run_with_host_mutation(root.path(), &wrapped);
        assert_eq!(observed.status.code(), Some(1));
        let mut run = result(observed.status.code(), &observed.stdout, &observed.stderr);
        run.acceptance_id = "AC-X-001".into();
        let probe = HostProbe::at(root.path().into(), root.path().into(), None);
        // Inject only the already observed mutation run; listing and proof
        // classification still execute their normal production paths.
        probe.inject_prove_run(
            TreeRun::known(
                &[CheckEvidence {
                    result: run,
                    copy_layout: None,
                }],
                "observed",
            ),
            markers,
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let findings = runtime.block_on(prove::prove(
            &probe,
            &Baseline {
                commit: "0".repeat(40),
                repository: root.path().into(),
            },
            &check,
            &contract_digest(&check).unwrap(),
            &[],
            &["AC-X-001".into()],
            &Default::default(),
        ));
        assert!(
            findings.is_empty(),
            "a listing failure cannot accuse the author: {findings:?}"
        );
        let held = probe.take_unproven_causes();
        assert!(held["AC-X-001"].0.contains("no output"), "{held:?}");
        assert_ne!(held["AC-X-001"].1, UnprovenCause::SiteLacks);
        assert_ne!(held["AC-X-001"].1, UnprovenCause::ProgramAbsent);
        send.send(()).unwrap();
    });
    receive
        .recv_timeout(Duration::from_secs(28))
        .expect("mutation listing regression completed");
}
