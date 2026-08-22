# 0002 — a test passed on the one platform where it could not run

- **Date discovered:** 2026-08-22
- **Introduced:** [`1da0df3da`](https://github.com/ste-bah/archon-cli/commit/1da0df3da) — 2026-04-08
- **Fixed:** assertion rewritten in [`bed66e1a0`](https://github.com/ste-bah/archon-cli/commit/bed66e1a0); platform gate and cwd in [`3d2c24b58`](https://github.com/ste-bah/archon-cli/commit/3d2c24b58) — both 2026-08-22
- **Exposure:** 136 days green on Windows without ever executing the code under test
- **Defect class:** [**vacuous check**](../defensive-patterns.md#dp-0--a-check-whose-scan-target-can-vanish-must-fail-not-pass) — here, a test whose subject failed to start
- **Test:** `test_per_hook_timeout_clamped_to_remaining_budget`, `crates/archon-core/tests/hook_timeout_budget_tests.rs`

## What the test was for

Hooks run under an aggregate budget. A single hook may declare a long timeout of
its own, but it must be clamped to whatever remains of the aggregate. The test
registers one hook with `timeout=60` whose command sleeps for 5 seconds, sets
`aggregate_timeout_ms: 2_000`, and checks the hook is killed at the budget rather
than at its own timeout.

That is a real invariant and worth a test.

## What actually happened

```rust
let input = serde_json::json!({"tool_name": "Bash"});
let cwd = PathBuf::from("/tmp");

let start = std::time::Instant::now();
let _result = registry
    .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
    .await;
let elapsed = start.elapsed();

// Should complete in roughly 2s (budget), NOT 5s (sleep) or 60s (hook timeout).
assert!(
    elapsed.as_secs() < 4,
    "Expected hook to be killed within ~2s budget, but took {:.1}s",
    elapsed.as_secs_f64()
);
```

The test was not platform-gated, so it ran on Windows. On Windows **two**
independent things are missing:

1. There is no `sleep` binary.
2. `PathBuf::from("/tmp")` is a rooted path with no drive, so it resolves to
   `\tmp` on the current drive. That directory does not exist, and a process spawn
   with a non-existent working directory fails before the command is even looked
   up — `os error 267`, "The directory name is invalid."

So the hook never started. `execute_hooks` returned a spawn error in well under a
millisecond, the result was discarded into `let _result`, and `elapsed` was
approximately zero.

**Zero is less than four.** The assertion passed.

That is the whole defect, and it is worth stating in its general form because the
shape recurs constantly: **an upper time bound is satisfied by everything that
fails fast.** The faster and more completely the subject breaks, the more
confidently the test reports success. A test whose only assertion is
`elapsed < N` cannot distinguish "the deadline worked" from "nothing ran".

Two further details made it invisible:

- `let _result = ...` discarded the only value that carried the failure. The spawn
  error was fully reported in `result.block_reason()` and nothing read it.
- The comment above the assertion described the intended three-way distinction —
  2s budget vs 5s sleep vs 60s hook timeout — which reads as rigour. The code
  underneath collapsed all three into a single one-sided bound.

## Why nothing caught it

Nothing to catch. The test was green on all three CI platforms and had been since
2026-04-08. It never became slow, never became flaky, and never failed. Its Windows
run was a fast, silent no-op with a passing assertion, which is indistinguishable
from a fast, correct run — from the outside.

## The fix

Assert the **outcome**, not the duration:

```rust
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
```

There is no clock in it. The first assertion rules out "never ran"; the second
rules out "ran to completion". Together they pin the exact behaviour the test
claims to be about.

**It went red on Windows immediately**, in 0.220 s:

```
FAIL [0.220s] archon-core::hook_timeout_budget_tests test_per_hook_timeout_clamped_to_remaining_budget
  expected the hook to be killed by the clamped 2s budget rather than run to
  completion under its own 60s timeout, but the reported outcome was
  "hook 'sleep 5' failed: spawn error: sleep 5: The directory name is invalid. (os error 267)"
```

Which is the point: **the corrected assertion surfaced a four-month-old lie on its
first run.** The subsequent fix — `#[cfg(unix)]` on the test, matching the
`cfg(unix)`/`cfg(windows)` split the other hook process tests already use, and
`std::env::temp_dir()` instead of the `/tmp` literal — was the easy part.

## Related

The same commit removed elapsed-time bounds from two other suites for the same
reason: the subagent runner (`d41_total_wall_clock_times_out_silent_llm_stream`,
`parallel_tool_dispatch_concurrent_and_order_preserved`) and the bash tool
(`bash_clamps_longer_requested_timeout_to_configured_maximum`). Several of those
bounds merely restated an outer `tokio::time::timeout` that would have failed the
test anyway. The clamp arithmetic itself was extracted to a process-free module,
`crates/archon-core/src/hooks/registry/budget.rs`, with six unit tests that need no
subprocess and no clock at all — the durable version of this test.

## Rules this produced

- [DP-5 — assert the outcome, never the elapsed time](../defensive-patterns.md#dp-5--assert-the-outcome-never-the-elapsed-time)
- [DP-8 — a test must fail if its subject never started](../defensive-patterns.md#dp-8--a-test-must-fail-if-its-subject-never-started)
- [DP-9 — no `/tmp`, no `sleep`, no bare POSIX path in a cross-platform test](../defensive-patterns.md#dp-9--no-tmp-no-sleep-no-bare-posix-path-in-a-cross-platform-test)
