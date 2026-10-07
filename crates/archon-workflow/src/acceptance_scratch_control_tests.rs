//! #356: observation phases and scratch git calls are bounded by no progress
//! only; a progressing phase never stops, a stall pauses resumably.
use super::*;
use std::path::PathBuf;

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
fn pulses() -> u64 {
    TEST_PULSES.with(|n| n.get())
}
fn paused(error: &WorkflowError) -> bool {
    matches!(error, WorkflowError::ControlPaused(why) if why.contains(OBSERVATION_STALLED))
}

#[test]
fn issue356_a_progressing_phase_outlives_its_window() {
    let started = Instant::now();
    let done = Control::new(1, cancel()).run(|| {
        for _ in 0..6 {
            std::thread::sleep(Duration::from_millis(300));
            check()?;
        }
        Ok(())
    });
    assert!(done.is_ok(), "{done:?}");
    assert!(
        started.elapsed() > Duration::from_secs(1),
        "past the window in total"
    );
}

#[test]
fn issue356_each_finished_phase_reports_activity() {
    // Two silent phases: no inner progress point, each well inside the
    // window. Each start and each end is reported, so a parent window
    // shorter than both together is renewed between them.
    let before = pulses();
    for _ in 0..2 {
        Control::new(60, cancel())
            .run(|| {
                std::thread::sleep(Duration::from_millis(50));
                Ok(())
            })
            .unwrap();
    }
    assert!(pulses() - before >= 4, "start and end of both phases");
    // A failed phase is no progress at its end.
    let before = pulses();
    let failed: WorkflowResult<()> = Control::new(60, cancel()).run(|| Err(invalid("boom")));
    assert!(failed.is_err());
    assert_eq!(pulses() - before, 1, "only its start");
}

#[test]
fn issue356_a_wait_with_no_progress_pauses_resumably() {
    let started = Instant::now();
    let waited: WorkflowResult<()> = Control::new(1, cancel()).run(|| {
        loop {
            poll("a lock another holder keeps")?;
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let error = waited.expect_err("an endless wait stalls");
    assert!(paused(&error), "a stall pauses, never fails: {error}");
    assert!(
        error.to_string().contains("a lock another holder keeps"),
        "{error}"
    );
    assert!(started.elapsed() >= Duration::from_secs(1));
    // Progress during a wait renews it: a holder that releases in time wins.
    let mut left = 4;
    let renewed = Control::new(1, cancel()).run(|| {
        while left > 0 {
            poll("a lock")?;
            std::thread::sleep(Duration::from_millis(400));
            left -= 1;
            check()?;
        }
        Ok(())
    });
    assert!(renewed.is_ok(), "{renewed:?}");
}

#[test]
fn issue356_cancellation_still_stops_a_phase() {
    let flag = cancel();
    flag.store(true, Ordering::SeqCst);
    let error = Control::new(60, flag).run(|| Ok(())).unwrap_err();
    assert!(!paused(&error), "a cancellation is not a stall: {error}");
}

/// A repository whose pre-commit hook runs `hook`.
fn repo_with_hook(hook: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let run = |args: &[&str]| {
        let status = archon_shell::spawn::command("git")
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    run(&["init", "-q", root.to_str().unwrap()]);
    let hooks = dir.path().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let path = hooks.join("pre-commit");
    std::fs::write(&path, format!("#!/bin/sh\n{hook}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (dir, root)
}
fn commit(root: &Path, hooks: &Path) -> WorkflowResult<String> {
    let hooks = format!("core.hooksPath={}", hooks.display());
    git(
        root,
        &[
            "-c",
            &hooks,
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
        &[],
    )
}

#[cfg(unix)]
#[test]
fn issue356_a_git_child_that_keeps_talking_outlives_the_window() {
    let (dir, root) =
        repo_with_hook("for i in 1 2 3 4 5 6 7; do echo working >&2; sleep 0.3; done");
    let started = Instant::now();
    let done = Control::new(1, cancel()).run(|| commit(&root, &dir.path().join("hooks")));
    assert!(done.is_ok(), "{done:?}");
    assert!(
        started.elapsed() > Duration::from_secs(2),
        "past the window in total"
    );
}

#[cfg(unix)]
#[test]
fn issue356_a_silent_git_child_pauses_at_its_window() {
    let (dir, root) = repo_with_hook("sleep 30");
    let started = Instant::now();
    let error = Control::new(1, cancel())
        .run(|| commit(&root, &dir.path().join("hooks")))
        .expect_err("a silent hook stalls the call");
    assert!(paused(&error), "a stall pauses, never fails: {error}");
    assert!(error.to_string().contains("git child"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "at the window, not the sleep"
    );
}

#[test]
fn issue356_a_recorded_stall_survives_as_text_and_is_found() {
    let error = Control::new(1, cancel())
        .run(|| -> WorkflowResult<()> {
            loop {
                poll("x")?;
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .unwrap_err();
    // An observation records its operational errors as text.
    let recorded = vec!["copy failed".to_string(), error.to_string()];
    assert_eq!(observation_stall(&recorded), Some(&recorded[1]));
    let other = vec!["native observation phase deadline exceeded".to_string()];
    assert_eq!(observation_stall(&other), None, "only a stall pauses");
    assert_eq!(observation_stall(&Vec::<String>::new()), None);
}
