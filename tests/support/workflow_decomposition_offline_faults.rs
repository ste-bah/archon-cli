//! The fault matrix (#257): one table of fault x boundary cells over the
//! fixed decomposition, each re-run against the fixed golden result.
//!
//! Every cell asserts that:
//! - the final artifacts and normalised result-store records equal golden;
//! - each call made exactly as many provider requests as in golden, except
//!   the request the fault interrupted, which is made once more;
//! - every author call accepted before the fault is reused, not re-executed,
//!   and an interrupted author call is accepted exactly once after it;
//! - the run ends completed with at most one `workflow resume` and no other
//!   manual step.
use std::fs;
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

use crate::deadline::{
    NO_PROGRESS_LIMIT, await_provider_boundary, progress, run, tail, wait_for_child,
};
use crate::project::{
    FinalSnapshot, accepted_call_counts, assert_no_unknown_prompts,
    assert_only_call_accepted_again, assert_same_json, final_snapshot, project_with,
    read_golden_requests, resume, run_id, spawn_decompose,
};
use crate::server::{Plan, Variant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// `workflow pause` while the run waits at the boundary before the call
    /// named by `target` (call id prefix or host `commandId`), then resume.
    PauseAtBoundary,
    /// SIGKILL of the run process at that boundary, then resume.
    KillAtBoundary,
    /// `workflow pause` while the provider holds the `nth` request of role
    /// `target`, inside an author call or a provider-backed host command.
    PauseMidCall,
    /// SIGKILL of the run process while the provider holds that request.
    KillMidCall,
    /// The first attempt of host command `target` reaches the supervisor's
    /// no-progress timeout; the run must recover without a resume.
    TimeoutOnce,
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub fault: Fault,
    pub target: &'static str,
    pub nth: usize,
}

/// One `#[test]` per row of the table, so each cell reports and can be
/// ignored on its own.
macro_rules! fault_matrix {
    ($($(#[$meta:meta])* $name:ident: $fault:ident $target:literal $nth:literal;)*) => {
        #[allow(dead_code)]
        pub const MATRIX: &[(&str, Cell)] = &[
            $((stringify!($name), Cell { fault: Fault::$fault, target: $target, nth: $nth }),)*
        ];
        $(
            #[test]
            $(#[$meta])*
            fn $name() {
                run_cell(stringify!($name), Cell { fault: Fault::$fault, target: $target, nth: $nth });
            }
        )*
    };
}

// New cells run on Unix only (verified there); the provider-boundary pause
// cell kept from the first lifecycle suite still runs on every platform.
fault_matrix! {
    #[cfg(unix)]
    pause_before_acceptance_author: PauseAtBoundary "acceptance-author-" 1;
    #[cfg(unix)]
    pause_before_freeze_acceptance: PauseAtBoundary "freeze-acceptance" 1;
    #[cfg(unix)]
    pause_before_skeleton_author: PauseAtBoundary "skeleton-author-" 1;
    #[cfg(unix)]
    pause_before_freeze_skeleton: PauseAtBoundary "freeze-skeleton" 1;
    #[cfg(unix)]
    pause_before_body_author: PauseAtBoundary "body-" 1;
    #[cfg(unix)]
    pause_before_land_task_body: PauseAtBoundary "land-task-body" 1;
    #[cfg(unix)]
    pause_before_task_set_lint: PauseAtBoundary "task-set-lint" 1;
    #[cfg(unix)]
    pause_before_requirements_trace: PauseAtBoundary "requirements-trace" 1;
    #[cfg(unix)]
    pause_before_final_report: PauseAtBoundary "fixed-decomposition-final" 1;
    #[cfg(unix)]
    kill_before_acceptance_author: KillAtBoundary "acceptance-author-" 1;
    #[cfg(unix)]
    kill_before_freeze_acceptance: KillAtBoundary "freeze-acceptance" 1;
    #[cfg(unix)]
    kill_before_skeleton_author: KillAtBoundary "skeleton-author-" 1;
    #[cfg(unix)]
    kill_before_freeze_skeleton: KillAtBoundary "freeze-skeleton" 1;
    #[cfg(unix)]
    kill_before_body_author: KillAtBoundary "body-" 1;
    #[cfg(unix)]
    kill_before_land_task_body: KillAtBoundary "land-task-body" 1;
    #[cfg(unix)]
    kill_before_task_set_lint: KillAtBoundary "task-set-lint" 1;
    #[cfg(unix)]
    kill_before_requirements_trace: KillAtBoundary "requirements-trace" 1;
    #[cfg(unix)]
    kill_before_final_report: KillAtBoundary "fixed-decomposition-final" 1;
    #[cfg(unix)]
    pause_mid_acceptance_author: PauseMidCall "acceptance" 1;
    #[cfg(unix)]
    pause_mid_freeze_acceptance_judge: PauseMidCall "judge" 1;
    pause_at_provider_call_boundary_resumes_only_interrupted_author: PauseMidCall "skeleton" 1;
    #[cfg(unix)]
    pause_mid_body_author: PauseMidCall "body" 1;
    #[cfg(unix)]
    pause_mid_body_gate_fidelity: PauseMidCall "fidelity" 1;
    #[cfg(unix)]
    pause_mid_set_gate_fidelity: PauseMidCall "fidelity" 3;
    #[cfg(unix)]
    kill_mid_acceptance_author: KillMidCall "acceptance" 1;
    #[ignore = "defect #422: SIGKILL of the run during a host command leaves survivors_unknown, so the resume refuses and asks for a manual step"]
    #[cfg(unix)]
    kill_mid_freeze_acceptance_judge: KillMidCall "judge" 1;
    #[cfg(unix)]
    sigkill_mid_provider_call_resumes_only_interrupted_author: KillMidCall "skeleton" 1;
    #[cfg(unix)]
    kill_mid_body_author: KillMidCall "body" 1;
    #[ignore = "defect #422: SIGKILL of the run during a host command leaves survivors_unknown, so the resume refuses and asks for a manual step"]
    #[cfg(unix)]
    kill_mid_body_gate_fidelity: KillMidCall "fidelity" 1;
    #[ignore = "defect #422: SIGKILL of the run during a host command leaves survivors_unknown, so the resume refuses and asks for a manual step"]
    #[cfg(unix)]
    kill_mid_set_gate_fidelity: KillMidCall "fidelity" 3;
    #[cfg(unix)]
    timeout_freeze_acceptance: TimeoutOnce "freeze-acceptance" 1;
    #[cfg(unix)]
    timeout_freeze_skeleton: TimeoutOnce "freeze-skeleton" 1;
    #[cfg(unix)]
    timeout_land_task_body: TimeoutOnce "land-task-body" 1;
    #[cfg(unix)]
    timeout_task_set_lint: TimeoutOnce "task-set-lint" 1;
    #[cfg(unix)]
    timeout_requirements_trace: TimeoutOnce "requirements-trace" 1;
}

fn run_cell(name: &str, cell: Cell) {
    progress(name, "starting");
    let mid_call = matches!(cell.fault, Fault::PauseMidCall | Fault::KillMidCall);
    let project = project_with(Plan {
        variant: Variant::Golden,
        block: mid_call.then(|| (cell.target.to_string(), cell.nth)),
    });
    let root = project.path();
    let hold = root.join(".test-hold");
    fs::create_dir_all(&hold).unwrap();
    let seams = match cell.fault {
        Fault::PauseAtBoundary | Fault::KillAtBoundary => vec![
            ("ARCHON_TEST_HOLD_CALL", cell.target.to_string()),
            ("ARCHON_TEST_HOLD_DIR", hold.display().to_string()),
        ],
        Fault::TimeoutOnce => vec![("ARCHON_TEST_HOST_COMMAND_TIMEOUT_ONCE", cell.target.into())],
        Fault::PauseMidCall | Fault::KillMidCall => Vec::new(),
    };
    let mut child = spawn_decompose(root, &seams);
    let mut expected_requests = read_golden_requests();
    // The call whose attempt number the fault changes.
    let interrupted = match cell.fault {
        Fault::TimeoutOnce => {
            let status = wait_decompose(root, &mut child, name);
            assert!(
                status.success(),
                "{name}: the run must recover from one timeout in place: {status}"
            );
            let events =
                fs::read_to_string(crate::project::one_run(root).join("events.jsonl")).unwrap();
            let retries = events
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .filter(|event| event["detail"]["event"] == "host_command_operational_retry")
                .map(|event| event["detail"].clone())
                .collect::<Vec<_>>();
            assert_eq!(
                retries.len(),
                1,
                "{name}: exactly one in-place retry is recorded: {retries:?}"
            );
            let retry = &retries[0];
            assert_eq!(retry["command_id"], cell.target, "{name}: {retry}");
            assert_eq!(retry["reason"], "timed_out", "{name}: {retry}");
            assert_eq!(retry["attempt"], 1, "{name}: {retry}");
            // The record keeps the catalog's window; only the seam's attempt
            // ran with a zero one.
            assert!(
                retry["limit_secs"].as_u64().is_some_and(|secs| secs > 0),
                "{name}: {retry}"
            );
            None
        }
        Fault::PauseAtBoundary | Fault::KillAtBoundary => {
            let held = await_hold(root, &hold, &mut child, name);
            progress(name, &format!("held before {held}"));
            let author = ["acceptance-author-", "skeleton-author-", "body-"]
                .iter()
                .any(|prefix| held.starts_with(prefix))
                .then(|| held.clone());
            let release = || fs::write(hold.join("release"), b"").unwrap();
            let pause = cell.fault == Fault::PauseAtBoundary;
            interrupt(
                root,
                &mut child,
                pause,
                true,
                release,
                author.as_deref(),
                name,
            );
            // The held call was never admitted, so no attempt number moves.
            None
        }
        Fault::PauseMidCall | Fault::KillMidCall => {
            await_provider_boundary(&project.provider, root, &mut child, name);
            let key = project
                .provider
                .requests
                .snapshot()
                .into_iter()
                .filter(|request| request.role == cell.target)
                .nth(cell.nth - 1)
                .expect("the held request is logged")
                .key;
            progress(name, &format!("provider holds {key}"));
            *expected_requests.entry(key.clone()).or_default() += 1;
            let provider = &project.provider;
            let author = author_call_prefix(&key);
            let pause = cell.fault == Fault::PauseMidCall;
            interrupt(
                root,
                &mut child,
                pause,
                false,
                || provider.release(),
                author.as_deref(),
                name,
            );
            Some(author.unwrap_or_else(|| host_command_of(cell.target, cell.nth)))
        }
    };
    assert_no_unknown_prompts(&project.provider);
    assert_eq!(
        project.provider.requests.per_call(),
        expected_requests,
        "{name}: provider requests per call differ from golden (+1 only for the interrupted request)"
    );
    let interrupted = interrupted.as_deref().into_iter().collect::<Vec<_>>();
    let mut golden: FinalSnapshot =
        serde_json::from_value(crate::snapshot::read_golden_snapshot()).unwrap();
    crate::snapshot::golden_records_for(&mut golden.records, &interrupted);
    assert_same_json(
        &format!("{name}: final artifacts and result-store records against golden"),
        &crate::project::keyed(
            serde_json::to_value(final_snapshot(&project, &interrupted)).unwrap(),
        ),
        &crate::project::keyed(serde_json::to_value(golden).unwrap()),
    );
    progress(name, "passed");
}

/// Pauses (and lets the held call go) or kills the run, then resumes it once.
fn interrupt(
    root: &Path,
    child: &mut Child,
    pause: bool,
    release_first: bool,
    release: impl FnOnce(),
    author: Option<&str>,
    name: &str,
) {
    let run_id = run_id(root);
    if pause {
        let paused = run(root, &["workflow", "pause", &run_id]);
        assert!(
            paused.status.success(),
            "{name}: pause failed: {}",
            String::from_utf8_lossy(&paused.stderr)
        );
        // A provider-held call is released only after the paused process
        // settled, so a host child the pause has not yet torn down never
        // gets (and saves) the interrupted reply. A boundary hold must be
        // released first: the run polls run control only after it.
        let status = if release_first {
            release();
            wait_decompose(root, child, name)
        } else {
            let status = wait_decompose(root, child, name);
            release();
            status
        };
        assert!(
            status.success(),
            "{name}: the paused run process must settle cleanly: {status}"
        );
    } else {
        child.kill().unwrap();
        let _ = wait_decompose(root, child, name);
        release();
    }
    let before = accepted_call_counts(root);
    progress(name, "resuming");
    let resumed = resume(root, &run_id);
    assert!(
        resumed.status.success(),
        "{name}: one resume must finish the run: {}\n{}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_only_call_accepted_again(&before, &accepted_call_counts(root), author);
}

fn wait_decompose(root: &Path, child: &mut Child, name: &str) -> std::process::ExitStatus {
    wait_for_child(
        child,
        &crate::deadline::harness_root(root).join("decompose.stdout"),
        &crate::deadline::harness_root(root).join("decompose.stderr"),
        name,
    )
}

/// Waits until the hold seam reports the held call id.
fn await_hold(root: &Path, hold: &Path, child: &mut Child, name: &str) -> String {
    let started = Instant::now();
    loop {
        if let Ok(held) = fs::read_to_string(hold.join("held")) {
            return held;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "{name}: the run ended ({status}) before reaching the held call; stderr tail: {}",
                tail(&crate::deadline::harness_root(root).join("decompose.stderr"))
            );
        }
        if started.elapsed() > NO_PROGRESS_LIMIT * 4 {
            let _ = child.kill();
            panic!("{name}: no held call within {:?}", NO_PROGRESS_LIMIT * 4);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The author call a provider request key belongs to, as a call id prefix.
fn author_call_prefix(key: &str) -> Option<String> {
    let (role, id) = key.split_once(':').unwrap_or((key, ""));
    match role {
        "acceptance" => Some(format!("acceptance-author-{id}-")),
        "skeleton" => Some("skeleton-author-".into()),
        "body" => Some(format!("body-{id}-author-")),
        _ => None,
    }
}

/// The host command the `nth` request of a provider role is made from
/// (the golden order: tests/fixtures/offline_decomposition_golden_requests.json).
fn host_command_of(role: &str, nth: usize) -> String {
    match (role, nth) {
        ("judge", _) => "freeze-acceptance".into(),
        ("fidelity", 1 | 2) => "land-task-body".into(),
        ("fidelity", 3) => "task-set-lint".into(),
        other => panic!("no host command makes request {other:?}"),
    }
}

#[test]
fn the_matrix_names_each_cell_once_and_maps_every_interrupted_request() {
    let mut names = std::collections::BTreeSet::new();
    for (name, cell) in MATRIX {
        assert!(names.insert(*name), "{name} appears twice");
        if matches!(cell.fault, Fault::PauseMidCall | Fault::KillMidCall)
            && !matches!(cell.target, "acceptance" | "skeleton" | "body")
        {
            let _ = host_command_of(cell.target, cell.nth);
        }
    }
}
