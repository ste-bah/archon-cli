//! REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D5.
//!
//! Locks in the pre-refactor behaviour of AGT-025 "auto-background at
//! 120s" so that any phase-1 REQ-FOR-D1/D2/D3 refactor that drifts from
//! the current semantics is caught immediately.
//!
//! Invariants asserted (from 00-prd-analysis.md REQ-FOR-PRESERVE-D5):
//!   (a) `AUTO_BACKGROUND_MS == 120_000` constant is unchanged
//!   (b) `ARCHON_AUTO_BACKGROUND_TASKS` env gate still flips
//!   (c) timeout-elapsed branch abandons the join_handle without re-awaiting
//!   (d) `SubagentManager` tracks a Running → Completed state transition
//!
//! Spec-vs-reality divergence (approved phase-0 2026-04-11):
//!   Test (b) in the spec says "spawn a mock subagent past the
//!   threshold". The threshold is hard-coded at 120_000 ms (2 minutes)
//!   and there is no in-process override hook, so waiting it out in CI
//!   is impossible. The existing unit tests at subagent.rs:1371-1429
//!   assert the invariant by flipping `ARCHON_AUTO_BACKGROUND_TASKS`
//!   and calling `is_auto_background_enabled()` directly. This
//!   regression guard mirrors that approach — it asserts the *gate
//!   function* still honours the env var. No MockProvider is needed,
//!   so `archon-test-support` is deliberately NOT opted into by this
//!   test. Later phase-specific tasks are free to add a slower
//!   integration test on top that DOES spawn a real subagent once
//!   AUTO_BACKGROUND_MS becomes injectable.

use archon_core::subagent::{
    is_auto_background_enabled, SubagentManager, SubagentStatus, AUTO_BACKGROUND_MS,
};
use archon_tools::agent_tool::SubagentRequest;

// --------------------------------------------------------------------
// Test (a): constant unchanged
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D5 (a).
#[test]
fn test_auto_background_ms_constant_is_120000() {
    // Runtime assertion (cheap, fast).
    assert_eq!(
        AUTO_BACKGROUND_MS, 120_000,
        "AGT-025 auto-background threshold drifted from 120_000 ms — REQ-FOR-PRESERVE-D5 (a) violated"
    );

    // Static assertion: the literal must still appear in source so
    // refactors that move the constant under a different name or into
    // a config struct are also caught.
    let source = include_str!("../src/subagent.rs");
    assert!(
        source.contains("AUTO_BACKGROUND_MS: u64 = 120_000")
            || source.contains("AUTO_BACKGROUND_MS = 120_000"),
        "AUTO_BACKGROUND_MS literal 120_000 missing from subagent.rs — REQ-FOR-PRESERVE-D5 (a)"
    );
}

// --------------------------------------------------------------------
// Test (b): env gate still works
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D5 (b).
///
/// Serialised via `#[serial]` because it mutates the
/// `ARCHON_AUTO_BACKGROUND_TASKS` env var which is process-global.
#[test]
#[serial_test::serial]
fn test_env_gate_archon_auto_background_tasks() {
    // Baseline: unset → disabled.
    // SAFETY: env mutation is serialised via #[serial] — no other test
    // is touching ARCHON_AUTO_BACKGROUND_TASKS concurrently.
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
    assert!(
        !is_auto_background_enabled(),
        "gate should be OFF when env unset"
    );

    for on_val in ["1", "true", "TRUE", "True"] {
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", on_val);
        }
        assert!(
            is_auto_background_enabled(),
            "gate should be ON for env value {on_val:?}"
        );
    }

    for off_val in ["0", "false", "no", ""] {
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", off_val);
        }
        assert!(
            !is_auto_background_enabled(),
            "gate should be OFF for env value {off_val:?}"
        );
    }

    // Leave the env clean for other serial tests.
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
}

// --------------------------------------------------------------------
// Test (c): timeout branch abandons join_handle without re-await
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D5 (c).
#[test]
fn test_timeout_branch_does_not_reawait_join_handle() {
    let source = include_str!("../src/agent.rs");

    // Must contain the select! construct.
    assert!(
        source.contains("tokio::select!"),
        "tokio::select! missing from agent.rs — AGT-025 race structure removed"
    );

    // Must contain the timeout-sleep arm.
    assert!(
        source.contains("tokio::time::sleep(timeout_duration)"),
        "timeout sleep arm missing from agent.rs — AGT-025 race structure removed"
    );

    // Isolate the sleep arm's BODY by splitting on the sleep pattern
    // and collecting lines up to the arm's `return` statement. The arm
    // body MUST end with a `return` (that is precisely the invariant:
    // the arm returns without awaiting anything, abandoning the
    // join_handle). Anything after `return` is out-of-arm code that we
    // do not want to inspect.
    let after_sleep = source
        .split("_ = tokio::time::sleep(timeout_duration) =>")
        .nth(1)
        .expect("sleep arm pattern not found");

    let mut arm_body = String::new();
    let mut saw_return = false;
    for (idx, line) in after_sleep.lines().enumerate() {
        arm_body.push_str(line);
        arm_body.push('\n');
        if line.contains("return ") {
            saw_return = true;
            break;
        }
        if idx >= 40 {
            panic!(
                "REQ-FOR-PRESERVE-D5 (c): could not find a `return` within 40 lines of the \
                 sleep arm — arm structure changed, re-validate the guard"
            );
        }
    }
    assert!(
        saw_return,
        "REQ-FOR-PRESERVE-D5 (c): sleep arm body did not contain a `return` statement"
    );

    // Tighter invariant (adversarial-review fix, 2026-04-11): forbid ANY
    // `.await` in the timeout arm body. The original assertion only
    // looked for the literal `join_handle.await` and could be bypassed
    // by a simple rename (`let handle = ...; handle.await`). The
    // auto-background design requires the arm to RETURN without
    // awaiting anything at all — the spawned task is abandoned by
    // construction. Any `.await` in this arm breaks the invariant.
    assert!(
        !arm_body.contains(".await"),
        "REQ-FOR-PRESERVE-D5 (c) violated: timeout arm body contains `.await`. The auto- \
         background arm MUST return without awaiting — awaiting anything here re-blocks on \
         the spawned task and defeats the purpose of backgrounding. Arm body was:\n{arm_body}"
    );
}

// --------------------------------------------------------------------
// Test (d): SubagentManager tracks running→completed
// --------------------------------------------------------------------

fn sample_request() -> SubagentRequest {
    SubagentRequest {
        prompt: "regression-guard-d5".into(),
        model: Some("claude-sonnet-4-6".into()),
        allowed_tools: vec!["Read".into()],
        max_turns: 1,
        timeout_secs: 60,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
    }
}

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D5 (d).
#[test]
fn test_subagent_manager_tracks_running_and_completed() {
    let mut mgr = SubagentManager::new(SubagentManager::DEFAULT_MAX_CONCURRENT);

    // Register → Running.
    let id = mgr.register(sample_request()).expect("register");
    assert!(
        mgr.is_running(&id),
        "freshly registered subagent must be Running"
    );
    assert_eq!(
        mgr.get_status(&id).expect("status").status,
        SubagentStatus::Running,
        "status query must report Running immediately after register"
    );
    assert_eq!(
        mgr.list_active().len(),
        1,
        "list_active must include the one Running agent"
    );

    // Complete → Completed.
    mgr.complete(&id, "ok".into()).expect("complete");
    assert!(
        !mgr.is_running(&id),
        "completed subagent must not report Running"
    );
    assert_eq!(
        mgr.get_status(&id).expect("status").status,
        SubagentStatus::Completed,
        "status query must report Completed after complete()"
    );
    assert_eq!(
        mgr.list_active().len(),
        0,
        "list_active must be empty once the single agent has completed"
    );
}
