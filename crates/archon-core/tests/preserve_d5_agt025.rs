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

    // The sleep arm's BODY must NOT re-await join_handle. We isolate
    // the arm by splitting on the sleep pattern and inspecting the
    // next ~50 lines until the arm's closing `}` terminator.
    let after_sleep = source
        .split("_ = tokio::time::sleep(timeout_duration) =>")
        .nth(1)
        .expect("sleep arm pattern not found");
    // Grab a conservative window — enough to cover the entire arm but
    // stop before the next select arm or enclosing block.
    let window: String = after_sleep.lines().take(20).collect::<Vec<_>>().join("\n");
    assert!(
        !window.contains("join_handle.await"),
        "REQ-FOR-PRESERVE-D5 (c) violated: timeout arm re-awaits join_handle, creating a \
         double-await race. The whole point of auto-background is to ABANDON the handle \
         and let the spawned task keep running in the background."
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
