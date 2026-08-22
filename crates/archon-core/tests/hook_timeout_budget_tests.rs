//! TASK-HOOK-031: the aggregate timeout budget — its default, its accounting,
//! the failure policy it applies to what it skips, and the per-hook clamp.
//!
//! ## What was wrong with this file, and how it hid
//!
//! Every test here that touched a hook process used `sleep` as the command and
//! `PathBuf::from("/tmp")` as the working directory. Neither exists on Windows —
//! `/tmp` resolves to `\tmp` on the current drive, and a working directory that
//! is not there makes `Command::spawn` fail outright — so the hooks failed in
//! milliseconds and every assertion was satisfied by the failure path.
//!
//! Measured: with the working directory pointed at a path that does not exist,
//! so that no hook process can be created at all, **all nine tests still
//! passed**, and the file ran in 0.23s instead of 10.02s. A suite that reports
//! success against a subsystem it never reached is worse than no suite.
//!
//! The fix follows from what each test is actually about.
//!
//! A budget-*skipped* hook is never spawned: `HookRegistry::execute_hooks`
//! decides eligibility, then compares elapsed time against the budget, and on
//! exhaustion applies the hook's failure policy without going near a process.
//! Every test about that branch is process-free by nature, and the `sleep` was
//! only ever scaffolding to push the clock past a 1ms budget — which a *failed*
//! spawn did just as well, which is why they passed. Those tests now set the
//! budget to zero, reaching the same branch deterministically with no process,
//! no shell and no dependence on how loaded the machine is, and they assert
//! exact skip counts, which the old `> 0` could not.
//!
//! Two claims genuinely need a process.
//! `test_fast_hooks_all_complete_within_budget` stays cross-platform, because
//! `echo ok` runs under both `sh -c` and `cmd /C`, and it now asserts
//! `!is_blocked()` — on `PreToolUse`, the one gating event, a hook that cannot
//! spawn blocks under the default failure policy, so that is exactly the
//! discriminator the old assertion lacked. The clamp test needs a command that
//! outlives its budget, which has no portable spelling, so it is a
//! `cfg(unix)`/`cfg(windows)` pair following the split
//! `hooks::executor_tests.rs` already uses.

use archon_core::hooks::{
    AggregatedHookResult, HookCommandType, HookConfig, HookEvent, HookExecutionConfig,
    HookFailurePolicy, HookMatcher, HookRegistry,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Helper: build a HookConfig for a shell command
// ---------------------------------------------------------------------------

fn cmd_hook(command: &str, timeout: Option<u32>) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Command,
        command: command.to_string(),
        if_condition: None,
        timeout,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: HashMap::new(),
        allowed_env_vars: Vec::new(),
        on_failure: None,
        enabled: true,
    }
}

fn allowing_hook(command: &str) -> HookConfig {
    let mut hook = cmd_hook(command, Some(5));
    hook.on_failure = Some(HookFailurePolicy::Allow);
    hook
}

fn matcher_with_hooks(hooks: Vec<HookConfig>) -> HookMatcher {
    HookMatcher {
        matcher: None,
        hooks,
    }
}

/// A budget with nothing in it.
///
/// `execute_hooks` skips a hook when `budget_start.elapsed() >= budget`, so a
/// zero budget puts every eligible hook on the exhausted branch from the first
/// one, without a process and without a race. The previous spelling was 1ms and
/// a leading `sleep`, which reached the same branch only because *something*
/// took longer than a millisecond — including, on Windows, a spawn that failed.
fn exhausted() -> HookRegistry {
    HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 0,
    })
}

/// A directory that exists on the machine running the test.
fn cwd() -> std::path::PathBuf {
    std::env::temp_dir()
}

// ---------------------------------------------------------------------------
// test_aggregate_timeout_budget_default_is_30s
// ---------------------------------------------------------------------------

#[test]
fn test_aggregate_timeout_budget_default_is_30s() {
    let config = HookExecutionConfig::default();
    assert_eq!(config.aggregate_timeout_ms, 30_000);
}

// ---------------------------------------------------------------------------
// test_skipped_count_starts_at_zero
// ---------------------------------------------------------------------------

#[test]
fn test_skipped_count_starts_at_zero() {
    let result = AggregatedHookResult::new();
    assert_eq!(result.skipped_count, 0);
}

// ---------------------------------------------------------------------------
// test_skipped_count_incremented_on_budget_exhaustion
// ---------------------------------------------------------------------------

/// Every eligible hook reached with no budget left is counted as skipped.
///
/// The count is asserted exactly. `> 0` was true of any outcome in which
/// anything at all went wrong, including the Windows one where the hooks could
/// not be created; `== 3` says that all three were reached, judged ineligible
/// for execution by the budget, and accounted for — and it fails if the loop
/// ever stops short or double-counts.
#[tokio::test]
async fn test_skipped_count_incremented_on_budget_exhaustion() {
    let registry = exhausted();

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            allowing_hook("exit 0"),
            allowing_hook("exit 0"),
            allowing_hook("exit 0"),
        ])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &cwd(),
            "test-session",
        )
        .await;

    assert_eq!(
        result.skipped_count, 3,
        "every hook reached with an exhausted budget is a skip"
    );
}

// ---------------------------------------------------------------------------
// test_fast_hooks_all_complete_within_budget
// ---------------------------------------------------------------------------

/// Three quick hooks under the default 30s budget: none is skipped, and all
/// three actually ran.
///
/// The second half is the point. `skipped_count == 0` is the field's default
/// value — `test_skipped_count_starts_at_zero` above asserts exactly that — so
/// on its own it is satisfied by a run in which nothing happened at all, which
/// is what it was doing on Windows.
///
/// `!is_blocked()` is what a hook that never ran cannot satisfy. `PreToolUse` is
/// the only gating event, so a hook with no explicit `on_failure` blocks when it
/// cannot spawn, times out, or fails I/O. If the shell is missing, the working
/// directory is not there, or the command is unknown, this test now says so.
///
/// Deliberately not gated: `echo ok` is a valid single command under both
/// `sh -c` and `cmd /C`, so the claim is meaningful on every platform archon
/// runs on, and this is the one test in the file that exercises the hook
/// process path end to end.
#[tokio::test]
async fn test_fast_hooks_all_complete_within_budget() {
    let registry = HookRegistry::new();

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            cmd_hook("echo ok", Some(5)),
            cmd_hook("echo ok", Some(5)),
            cmd_hook("echo ok", Some(5)),
        ])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &cwd(),
            "test-session",
        )
        .await;

    assert_eq!(
        result.skipped_count, 0,
        "fast hooks should all complete within budget"
    );
    assert!(
        !result.is_blocked(),
        "the hooks must have run and exited cleanly; a hook that could not be \
         started blocks a PreToolUse under the default failure policy, which is \
         how this test tells 'they all succeeded' from 'none of them ran'. \
         Reported: {:?}",
        result.block_reason()
    );
}

// ---------------------------------------------------------------------------
// test_per_hook_timeout_clamped_to_remaining_budget
// ---------------------------------------------------------------------------

/// Which deadline wins when a hook asks for more time than the aggregate budget
/// has left: the budget.
///
/// The claim is about the *outcome*, not the duration. This used to assert
/// `elapsed.as_secs() < 4` around a 2s budget, which is a measurement of how
/// loaded the machine is — it was observed failing at 5.7s while other builds
/// ran and passing on a quiet re-run, in both cases against a clamp that worked.
/// Worse, a machine where the shell could not spawn at all would fail in
/// milliseconds and sail through the old assertion.
///
/// A `sleep 5` under a 2s budget and a 60s hook timeout can only end two ways,
/// and they are distinguishable without a clock: the clamp holds and the hook is
/// killed by its deadline (`RunError::Timeout` → gating-event block policy), or
/// the clamp is gone, 60s wins, and the sleep exits 0 into a clean Success. The
/// wall clock never enters into it — on a machine slow enough to take 5.7s, this
/// still reports a timeout, because the sleep is longer than the budget by
/// construction.
///
/// The clamp arithmetic itself is covered without any process at all in
/// `hooks::registry::budget`.
///
/// `cfg(unix)` because the hook command is `sleep`, following the
/// `cfg(unix)`/`cfg(windows)` split the other hook process tests already use.
/// This was previously ungated and ran on Windows, where neither `sleep` nor
/// `/tmp` exists — so the hook failed to spawn in milliseconds and the old
/// `elapsed` assertion passed on a run that proved nothing. Asserting the
/// outcome instead is what surfaced it. The Windows counterpart is below.
#[cfg(unix)]
#[tokio::test]
async fn test_per_hook_timeout_clamped_to_remaining_budget() {
    assert_clamped_hook_is_cut_short("sleep 5").await;
}

/// The same claim on Windows, which is where it was silently untested.
///
/// A counterpart rather than a bare `cfg(unix)` gate because the clamp decides
/// whether a slow hook can outlive the budget it is spending, and that is not a
/// unix-specific behaviour — it is the thing that stops one hook eating a whole
/// turn, on every platform.
///
/// PowerShell by absolute path, quoted for whichever shell `resolve_shell`
/// picked, is the same construction `hooks::executor_tests.rs` uses for its
/// Windows counterparts: on a Windows host with Git for Windows installed the
/// hook shell is `sh`, and without it `cmd`, and this command string is valid
/// under both. Its startup cost can only push the run further past the 2s
/// budget, never under it, so the outcome is fixed by construction exactly as
/// the unix version's is.
#[cfg(windows)]
#[tokio::test]
async fn test_per_hook_timeout_clamped_to_remaining_budget_windows() {
    assert_clamped_hook_is_cut_short(&windows_sleep_command()).await;
}

/// The body both platforms share: a hook that wants 60s, a budget with 2s in
/// it, and a command that runs for 5s. The only question is which deadline the
/// hook was killed by.
#[cfg(any(unix, windows))]
async fn assert_clamped_hook_is_cut_short(command: &str) {
    let registry = HookRegistry::with_config(HookExecutionConfig {
        aggregate_timeout_ms: 2_000,
    });

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![cmd_hook(command, Some(60))])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            // A directory that exists on the machine running the test. The
            // literal `/tmp` resolved to `\tmp` on the current drive under
            // Windows, which is the other half of why this passed while doing
            // nothing.
            &cwd(),
            "test-session",
        )
        .await;

    assert_eq!(
        result.skipped_count, 0,
        "the hook must have been started and then cut short, not skipped before it \
         ran; a skipped hook would prove nothing about the per-hook clamp"
    );
    let reason = result.block_reason().unwrap_or_default();
    assert!(
        reason.contains("timed out"),
        "expected the hook to be killed by the clamped 2s budget rather than run to \
         completion under its own 60s timeout, but the reported outcome was \
         {reason:?} (blocked: {})",
        result.is_blocked()
    );
}

/// A command that sleeps for five seconds and runs under either Windows shell.
#[cfg(windows)]
fn windows_sleep_command() -> String {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let powershell = std::path::Path::new(&system_root)
        .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    let powershell = if powershell.is_file() {
        powershell.display().to_string()
    } else {
        "powershell".to_string()
    };
    if archon_shell::resolve_shell().command_arg == "-c" {
        return format!(
            "'{}' -NoProfile -Command 'Start-Sleep -Seconds 5'",
            powershell.replace('\'', "'\\''")
        );
    }
    format!("\"{powershell}\" -NoProfile -Command \"Start-Sleep -Seconds 5\"")
}

// ---------------------------------------------------------------------------
// test_budget_exhausted_applies_default_failure_policy
// ---------------------------------------------------------------------------

/// A hook skipped for want of budget on a gating event blocks, because that is
/// what a `PreToolUse` hook that did not get to answer means.
///
/// No process: the hook is never spawned, so the old leading `sleep` proved
/// nothing about this and only served to advance the clock.
#[tokio::test]
async fn test_budget_exhausted_applies_default_failure_policy() {
    let registry = exhausted();

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![cmd_hook("exit 0", Some(5))])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &cwd(),
            "test-session",
        )
        .await;

    assert_eq!(result.skipped_count, 1);
    assert!(
        result.is_blocked(),
        "budget-exhausted PreToolUse hooks must use the default block policy"
    );
    assert!(
        result
            .block_reason()
            .unwrap_or_default()
            .contains("aggregate timeout exhausted"),
        "the refusal must name the budget as the cause, or an operator cannot \
         tell it from a hook that ran and said no: {:?}",
        result.block_reason()
    );
}

/// The same skip, with the hook's own policy overriding the event default.
#[tokio::test]
async fn test_budget_exhausted_respects_explicit_allow_policy() {
    let registry = exhausted();

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![allowing_hook("exit 0")])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &cwd(),
            "test-session",
        )
        .await;

    assert_eq!(result.skipped_count, 1);
    assert!(!result.is_blocked(), "blocked: {:?}", result.block_reason());
}

/// A hook whose condition does not match is ineligible, not timeout-skipped.
///
/// Eligibility is decided before the budget is consulted, so with no budget at
/// all the matching hook is skipped and the non-matching one is not counted —
/// `skipped_count == 1`, not 2 and not 0. The old version asserted `== 0` with
/// only a non-matching hook under test, which cannot tell "was not skipped"
/// from "was never there"; pairing it with a hook that *is* skipped is what
/// makes the number mean something.
#[tokio::test]
async fn test_budget_exhaustion_does_not_apply_policy_to_non_matching_hook() {
    let registry = exhausted();
    let mut non_matching = allowing_hook("exit 0");
    non_matching.if_condition = Some("Read".to_string());

    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![matcher_with_hooks(vec![
            allowing_hook("exit 0"),
            non_matching,
        ])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &cwd(),
            "test-session",
        )
        .await;

    assert!(!result.is_blocked(), "blocked: {:?}", result.block_reason());
    assert_eq!(
        result.skipped_count, 1,
        "only the matching hook is timeout-skipped; a hook that does not match \
         is ineligible and never reaches the budget check"
    );
}

/// An observational event has nothing to gate, so exhausting its budget cannot
/// block anything.
///
/// The hooks are deliberately left on the event default, because that default
/// is the whole subject: `is_gating_event` is true of `PreToolUse` and nothing
/// else, so the same undeclared `on_failure` that blocks a tool call in
/// `test_budget_exhausted_applies_default_failure_policy` above must allow here.
/// Declaring `Block` explicitly would not test the event default — an explicit
/// policy is honoured on every event, gating or not.
#[tokio::test]
async fn test_observational_budget_exhaustion_remains_non_blocking() {
    let registry = exhausted();

    registry.register_matchers(
        HookEvent::PostToolUse,
        vec![matcher_with_hooks(vec![
            cmd_hook("exit 0", Some(5)),
            cmd_hook("exit 0", Some(5)),
        ])],
        None,
    );

    let result = registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({"tool_name": "Bash"}),
            &cwd(),
            "test-session",
        )
        .await;

    assert_eq!(result.skipped_count, 2);
    assert!(!result.is_blocked(), "blocked: {:?}", result.block_reason());
}

// ---------------------------------------------------------------------------
// test_hook_execution_config_serialization
// ---------------------------------------------------------------------------

#[test]
fn test_hook_execution_config_serialization() {
    let config = HookExecutionConfig {
        aggregate_timeout_ms: 15_000,
    };

    let json = serde_json::to_string(&config).expect("serialize");
    let deserialized: HookExecutionConfig = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(deserialized.aggregate_timeout_ms, 15_000);
}
