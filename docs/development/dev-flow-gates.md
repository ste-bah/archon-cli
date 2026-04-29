# Dev flow gates

Every task tracked under `project-tasks/TASK-NNN-*` must pass 6 gates in order before being marked complete. The PreToolUse hook on `TaskUpdate` blocks marking tasks as completed without all 6 gates passed.

## The 6 gates

| Gate | Name | Check |
|---|---|---|
| 1 | `tests-written-first` | Test file exists BEFORE implementation file (TDD) |
| 2 | `implementation-complete` | Code compiles, no errors. Includes Gate 2 file-size auto-check (files < 1500 lines) |
| 3 | `sherlock-code-review` | Sherlock forensic adversarial review of implementation. Evidence MUST contain APPROVED/PASS verdict |
| 4 | `tests-passing` | All tests pass. Evidence MUST include count |
| 5 | `live-smoke-test` | Feature actually invoked end-to-end. Fraud detection blocks fake evidence ("tests pass", "library crate", "not yet wired", etc.) |
| 6 | `sherlock-final-review` | Sherlock final review: integration + wiring verified. Evidence MUST contain APPROVED/PASS verdict |

## Running gates

```bash
# Pass an individual gate
scripts/dev-flow-pass-gate.sh TASK-ID gate-name "evidence string"

# Verify all gates before marking complete
scripts/dev-flow-gate.sh TASK-ID
```

## Hardening

The system enforces gates structurally — they cannot be bypassed by "forgetting" to call gate scripts:

- The PreToolUse hook on `TaskUpdate` blocks marking any `TASK-NNN` as completed without all 6 gate files present
- Gate 3 + 6: evidence must contain `APPROVED`, `PASS`, or `INNOCENT` (Sherlock verdicts). `REJECTED` blocks the gate
- Gate 5: fraud detection blocks "tests pass", "library crate", "not yet wired", and similar generic claims that don't prove end-to-end execution. Requires real execution proof
- A task with missing gate files is NOT done regardless of human intent

## Why each gate exists

- **Gate 1 (tests-first)** prevents post-hoc rationalization. The test names the requirement before the code names the implementation.
- **Gate 2 (implementation)** is a baseline correctness check. Compile errors block progression.
- **Gate 3 (Sherlock review)** catches issues the implementation author missed. Sherlock independently reads the code; never trusts prior agent outputs.
- **Gate 4 (tests pass)** is the obvious check.
- **Gate 5 (smoke test)** catches the "module exists but not wired" failure mode. Code that compiles and tests pass can still not be reachable from runtime.
- **Gate 6 (final Sherlock)** verifies integration. The implementation may pass tests in isolation but fail when wired into the broader system.

## Common violations and recovery

### "I'll skip Sherlock just this once because the change is small"

No. Run Sherlock anyway. Small changes are where regressions hide. Historical incident: 2026-04-02, 12 of 18 tasks shipped without Sherlock; multiple regressions surfaced post-merge.

### "All tests pass" without count

Gate 4 requires the count. `tests/foo.rs: 3 passed` is acceptable. `tests pass` is not.

### Gate 5 fraud detection blocking "library crate, no smoke test possible"

If the change adds a public API, the smoke test is "another crate or test imports and uses the new API end-to-end". If the change is internal, the smoke test is "the existing user-facing feature still works end-to-end and exercises the changed code path."

### Gate 2 file-size violation

The Gate 2 auto-check (added v0.1.23) blocks files > 1500 lines. Split the file:
- One file per module if possible
- If logically inseparable, split by section with clear cross-references

### Hook fails on TaskUpdate

The hook checks for gate files at `project-tasks/TASK-ID/.gates/`. If files are missing, fix the gate first. Don't try to bypass the hook.

## Bypassing in extreme cases

There's no bypass. If you genuinely cannot pass a gate, the work isn't ready. File a follow-up task to address whatever's blocking the gate.

## See also

- [Contributing](contributing.md)
- [Release process](release-process.md)
- `scripts/dev-flow-gate.sh` and `scripts/dev-flow-pass-gate.sh` in the repo
