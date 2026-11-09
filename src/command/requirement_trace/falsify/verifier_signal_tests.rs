use super::*;

#[test]
fn interrupted_host_child() {
    let Some(root) = std::env::var_os("VERIFIER_SIGNAL_ROOT") else {
        return;
    };
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        // SAFETY: this isolated child explicitly selects the standalone disposition;
        // a background launcher may otherwise pass down ignored signals.
        assert_ne!(
            unsafe { libc::signal(signal, libc::SIG_DFL) },
            libc::SIG_ERR
        );
    }
    if std::env::var_os("VERIFIER_TWO_PLANS").is_some() {
        if std::env::var_os("VERIFIER_SESSION").is_some() {
            use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
            static RECEIVED: AtomicI32 = AtomicI32::new(0);
            static DELIVERIES: AtomicUsize = AtomicUsize::new(0);
            extern "C" fn session_handler(signal: i32) {
                RECEIVED.store(signal, Ordering::SeqCst);
                DELIVERIES.fetch_add(1, Ordering::SeqCst);
            }
            let default_disposition = std::env::var_os("VERIFIER_RETURN_ONLY").is_some();
            for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
                // SAFETY: this isolated child simulates the session's handlers.
                assert_ne!(
                    unsafe {
                        libc::signal(
                            signal,
                            if default_disposition {
                                libc::SIG_DFL
                            } else {
                                session_handler as *const () as usize
                            },
                        )
                    },
                    libc::SIG_ERR
                );
            }
            let error = interrupted_plans(Path::new(&root)).unwrap_err();
            assert_eq!(
                error.signal,
                std::env::var("VERIFIER_EXPECT_SIGNAL")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap()
            );
            assert_eq!(
                DELIVERIES.load(Ordering::SeqCst),
                usize::from(!default_disposition)
            );
            assert_eq!(
                RECEIVED.load(Ordering::SeqCst),
                if default_disposition { 0 } else { error.signal }
            );
            for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
                // SAFETY: query this isolated child's restored disposition.
                let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
                assert_eq!(
                    unsafe { libc::sigaction(signal, std::ptr::null(), &mut action) },
                    0
                );
                assert_eq!(
                    action.sa_sigaction,
                    if default_disposition {
                        libc::SIG_DFL
                    } else {
                        session_handler as *const () as usize
                    }
                );
            }
            return;
        }
        interrupted_plans(Path::new(&root)).unwrap();
        panic!("interrupted plans returned");
    }
    if std::env::var_os("VERIFIER_IGNORE").is_some() {
        // SAFETY: only this child changes its own inherited disposition.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        let ran = run(
            Path::new(&root),
            &[
                "/bin/sh".into(),
                "-c".into(),
                "kill -HUP $PPID; sleep 1; exit 0".into(),
            ],
            Duration::from_secs(5),
        );
        assert!(
            matches!(ran, Ran::Finished { success: true, .. }),
            "{ran:?}"
        );
        // SAFETY: query only; ignored disposition must survive teardown.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut action) },
            0
        );
        assert_eq!(action.sa_sigaction, libc::SIG_IGN);
        return;
    }
    if std::env::var_os("VERIFIER_RESTORE_RACE").is_some() {
        let mut monitor = super::super::signals::Monitor::new().unwrap();
        // SAFETY: this isolated child has the scoped handler installed.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        let mut nested = super::super::signals::Monitor::new().unwrap();
        assert_eq!(nested.pending(), Some(libc::SIGINT));
        nested.restore().unwrap();
        monitor.restore().unwrap();
        assert_eq!(monitor.pending(), Some(libc::SIGINT));
        super::super::signals::reraise(monitor.pending().unwrap());
    }
    if let Ok(mode) = std::env::var("VERIFIER_SIGNAL_MODE") {
        static OBSERVED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        extern "C" fn observed(signal: i32) {
            OBSERVED.fetch_or(
                if signal == libc::SIGINT { 1 } else { 2 },
                std::sync::atomic::Ordering::SeqCst,
            );
        }
        {
            for signal in [libc::SIGINT, libc::SIGTERM] {
                // SAFETY: this is an isolated child with a static callback.
                assert_ne!(
                    unsafe {
                        libc::signal(
                            signal,
                            if mode == "custom" {
                                observed as *const () as usize
                            } else {
                                libc::SIG_DFL
                            },
                        )
                    },
                    libc::SIG_ERR
                );
            }
        }
        let ran = run(
            Path::new(&root),
            &["/bin/sh".into(), "-c".into(), "exit 0".into()],
            Duration::from_secs(5),
        );
        assert!(matches!(ran, Ran::Finished { success: true, .. }));
        for signal in [libc::SIGINT, libc::SIGTERM] {
            // SAFETY: signal only this isolated test child.
            assert_eq!(unsafe { libc::raise(signal) }, 0);
        }
        assert_eq!(OBSERVED.load(std::sync::atomic::Ordering::SeqCst), 3);
        return;
    }
    let ran = run(
        Path::new(&root),
        &[
            "/bin/sh".into(),
            "-c".into(),
            "echo $$ > pids; sleep 60 & echo $! >> pids; touch ready; wait".into(),
        ],
        Duration::from_secs(60),
    );
    let Ran::Interrupted { signal } = ran else {
        panic!("{ran:?}")
    };
    super::super::signals::reraise(signal);
}

#[test]
fn interrupting_host_stops_real_child_and_grandchild() {
    for (signal, foreground_group) in [
        (libc::SIGINT, false),
        (libc::SIGTERM, false),
        (libc::SIGINT, true),
        (libc::SIGHUP, false),
        (libc::SIGQUIT, false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        use std::os::unix::process::CommandExt;
        let mut host = command
            .args(["interrupted_host_child", "--nocapture"])
            .env("VERIFIER_SIGNAL_ROOT", root.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.path().join("ready").exists() {
            if host.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                let _ = host.kill();
                panic!(
                    "host did not launch verifier: {:?}",
                    host.wait_with_output()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: this is our unreaped test child, never the test runner.
        let recipient = if foreground_group {
            -(host.id() as i32)
        } else {
            host.id() as i32
        };
        assert_eq!(unsafe { libc::kill(recipient, signal) }, 0);
        while host.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = host.kill();
                panic!("interrupted verifier host failed to stop");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = host.wait_with_output().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(signal), "{output:?}");
        for pid in std::fs::read_to_string(root.path().join("pids"))
            .unwrap()
            .split_whitespace()
        {
            assert!(
                !archon_shell::process_liveness::process_alive(pid.parse().unwrap()),
                "signal {signal} left process {pid} alive"
            );
        }
    }
}

#[test]
fn prior_custom_handlers_are_restored_after_verifier_teardown() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["interrupted_host_child", "--nocapture"])
        .env("VERIFIER_SIGNAL_ROOT", root.path())
        .env("VERIFIER_SIGNAL_MODE", "custom")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn prior_default_termination_is_restored_after_verifier_teardown() {
    use std::os::unix::process::ExitStatusExt;
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["interrupted_host_child", "--nocapture"])
        .env("VERIFIER_SIGNAL_ROOT", root.path())
        .env("VERIFIER_SIGNAL_MODE", "default")
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGINT), "{output:?}");
}

fn interrupted_plans(root: &Path) -> Result<(), super::super::Interrupted> {
    use super::super::tests::plan_for;
    use archon_knowledge::traceability::anchors::{Anchor, AnchorFreshness};
    use archon_knowledge::traceability::report::{AnchorVerdict, RequirementRow};
    use archon_knowledge::traceability::{CoverageReport, ProofLevel, Severity, TraceReport};
    let plan = plan_for(root, "/bin/sh run");
    let verdict = AnchorVerdict {
        anchor: Anchor {
            requirement_id: plan.requirement_id.clone(),
            task_id: plan.task_id.clone(),
            file_path: plan.file_path.clone(),
            line_start: plan.line_start,
            line_end: plan.line_end,
            file_hash: plan.expected_file_hash.clone(),
            path_scope: "src/".into(),
            relevance_score: 0.9,
        },
        freshness: AnchorFreshness::Fresh,
        level: ProofLevel::Exercised,
        proof: None,
        missing: None,
        falsification: Ok(plan.clone()),
        falsification_outcome: None,
    };
    let mut second = verdict.clone();
    second.falsification = Ok(archon_knowledge::traceability::FalsificationPlan {
        command: "/bin/sh second".into(),
        ..plan
    });
    let mut report = TraceReport {
        prd_path: "input.md".into(),
        task_dir: "tasks".into(),
        coverage: CoverageReport::default(),
        rows: vec![RequirementRow {
            requirement_id: "item".into(),
            prd_line: 1,
            severity: Severity::Error,
            severity_evidence: None,
            claimed_by: vec![],
            anchors: vec![verdict, second],
            anchor_gap: None,
            level: ProofLevel::Exercised,
        }],
        shared_anchors: vec![],
        stale_anchors: 0,
        index_consulted: true,
    };
    let limits = VerifyLimits {
        reraise_default: std::env::var_os("VERIFIER_SESSION").is_none(),
        ..VerifyLimits::for_test()
    };
    super::super::execute_plans_with_policy(root, &mut report, None, &limits)?;
    std::fs::write(root.join("persisted"), serde_json::to_vec(&report).unwrap()).unwrap();
    Ok(())
}

#[test]
fn interruption_restores_mutation_skips_second_plan_and_persistence() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        interrupt_plan(signal, false, false);
    }
}

#[test]
fn session_interruption_returns_typed_error_after_restoring_mutation() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        interrupt_plan(signal, true, false);
    }
}

#[test]
fn session_interruption_forwards_signal_once_after_restoring_mutation() {
    interrupt_plan(libc::SIGTERM, true, false);
}

#[test]
fn session_with_default_dispositions_returns_typed_error_without_exiting() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        interrupt_plan(signal, true, true);
    }
}

fn interrupt_plan(signal: i32, session: bool, default_disposition: bool) {
    use super::super::tests::repo_with_committed_file;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    let (root, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let original = std::fs::read(&path).unwrap();
    // Use the guard's actual backup name to distinguish baseline and mutant.
    let backup = super::super::guard::backup_path(&path);
    std::fs::write(
        root.path().join("run"),
        format!(
            "if test ! -f '{}'; then exit 0; fi\ntouch ready\nsleep 60",
            backup.display()
        ),
    )
    .unwrap();
    std::fs::write(root.path().join("second"), "touch second-started").unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    if session {
        command
            .env("VERIFIER_SESSION", "1")
            .env("VERIFIER_EXPECT_SIGNAL", signal.to_string());
    }
    if session && default_disposition {
        command.env("VERIFIER_RETURN_ONLY", "1");
    }
    let mut host = command
        .args(["interrupted_host_child", "--nocapture"])
        .env("VERIFIER_SIGNAL_ROOT", root.path())
        .env("VERIFIER_TWO_PLANS", "1")
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.path().join("ready").exists() {
        if host.try_wait().unwrap().is_some() || Instant::now() > deadline {
            let _ = host.kill();
            panic!("mutation did not run: {:?}", host.wait_with_output());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // SAFETY: the unreaped isolated child belongs to this test.
    assert_eq!(unsafe { libc::kill(host.id() as i32, signal) }, 0);
    while host.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = host.kill();
            panic!("host did not stop");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = host.wait_with_output().unwrap();
    if session {
        assert!(output.status.success(), "{output:?}");
    } else {
        assert_eq!(output.status.signal(), Some(signal), "{output:?}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!backup.exists());
    assert!(!root.path().join("second-started").exists());
    assert!(!root.path().join("persisted").exists());
}

#[test]
fn pending_signal_survives_handler_restoration() {
    use std::os::unix::process::ExitStatusExt;
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["interrupted_host_child", "--nocapture"])
        .env("VERIFIER_SIGNAL_ROOT", root.path())
        .env("VERIFIER_RESTORE_RACE", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGINT), "{output:?}");
}

#[test]
fn ignored_hangup_stays_ignored_and_verifier_completes() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["interrupted_host_child", "--nocapture"])
        .env("VERIFIER_SIGNAL_ROOT", root.path())
        .env("VERIFIER_IGNORE", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}
