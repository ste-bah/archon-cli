# CI gates

archon-cli's CI flow is technical (compile / lint / test). The orchestrator is `scripts/ci-gate.sh` — single source of truth. Any GitHub Actions / GitLab / local hook should call this script rather than replicate its steps.

> **Note:** This is **NOT** root archon's narrative 6-gate Sherlock-review protocol. Root archon (`/home/unixdude/Archon-projects/archon/`) has `scripts/dev-flow-gate.sh` and `scripts/dev-flow-pass-gate.sh` for `project-tasks/TASK-NNN-*` tracking with PreToolUse hooks. archon-cli has neither of those scripts and no equivalent task-tracking enforcement. When working on archon-cli, follow the technical ci-gate flow on this page.

## The 10 ci-gate steps

```
Step 1 — FileSizeGuard           — scripts/check-file-sizes.sh, ratchet-style allowlist
Step 2 — BannedImports           — scripts/check-banned-imports.sh, allowlist-driven
Step 2b— R0 entry gate           — scripts/check-r0-entry-gate.sh, learning-roadmap prerequisites
Step 2c— Deferral markers        — scripts/check-deferral-markers.sh, allowlisted TODO(TICKET) cap
Step 2d— Gate self-tests         — scripts/tests/, proof the guards above can go red
Step 3 — cargo fmt --check       — workspace-wide format check
Step 4 — cargo clippy            — --all-targets --jobs 1 -- -D warnings
Step 5 — cargo test              — --workspace --jobs 1 -- --test-threads=2
Step 6 — baseline test-list diff — vs tests/fixtures/baseline/cargo_test_list.txt
Step 7 — cargo bench --no-run    — archon-bench compile-only check
```

Steps run in order. First failure halts the gate.

## Running locally

```bash
./scripts/ci-gate.sh                # full CI
./scripts/ci-gate.sh --skip-bench   # skip step 7 (faster iteration)
```

Reference rationale per step lives in `scripts/ci-gate.README.md`.

## Why each step exists

| Step | Why |
|---|---|
| 1. FileSizeGuard | Files over the 500-line cap accumulate complexity (`THRESHOLD=500` in the script). Ratchet allowlist documents grandfathered over-size files. New code must comply. |
| 2. BannedImports | Workspace-wide policy on cross-crate imports. Prevents architectural creep (e.g. archon-tui depending on archon-pipeline directly). |
| 2b. R0 entry gate | Re-verifies every statement in `docs/development/r0-entry-gate.evidence` — per finding, the closure commit, the source anchors that must remain, the defect signatures that must stay absent, the live call site and its tests. The learning roadmap forbids promoting an R2–R6 slice until findings 9, 11, 17 and 40–43 are closed, and a gate that is only a paragraph cannot enforce that (#86). |
| 2c. Deferral markers | A slice that was wired can quietly regress to a stub, and the `TODO(TICKET)` left behind is the only trace — the compiler cannot see it, and in a library crate `pub` suppresses `dead_code` too. `scripts/check-deferral-markers.sh` scans `src/` and `crates/` for `TODO(TICKET)`, `FIXME(TICKET)` and `Deferred (TICKET)` and fails on any marker not in `check-deferral-markers.allowlist` — **and** on any allowlist line whose marker has gone, so it caps outstanding deferrals rather than suppressing them. Adding one means editing the allowlist, which means it is reviewed. |
| 2d. Gate self-tests | A gate nobody has watched go red is indistinguishable from a gate that cannot go red, and a silently-permissive gate is worse than none: it reports GREEN for work it never checked. See `scripts/tests/README.md` for what may live there. |
| 3. cargo fmt | Format consistency. No exception. |
| 4. cargo clippy | Lint with warnings-as-errors. No `#[allow(...)]` to silence — fix the underlying issue. |
| 5. cargo test | Workspace-wide test run. `--test-threads=2` is mandatory because of shared global state (BACKGROUND_AGENTS, task registry, tempdir-based `.archon/`) that deadlocks under unlimited parallelism on WSL2. |
| 6. baseline test-list diff | Detects accidentally added or removed tests. Update the baseline only deliberately. |
| 7. cargo bench --no-run | Bench compile check — catches bench-only API drift without running benchmarks. |

## TUI-specific gates

Run separately from ci-gate.sh; invoked from TUI workflow paths:

| Script | Purpose |
|---|---|
| `scripts/check-tui-file-sizes.sh` | 500-line limit for `crates/archon-tui/src/`, with a shrink-only allowlist |
| `scripts/tui-banned-patterns-gate.sh` | Banned-pattern detection in TUI sources |
| `scripts/check-tui-duplication.sh` | Duplication detection |
| `scripts/check-tui-coverage.sh` | Coverage tracking |
| `scripts/check-tui-module-cycles.sh` | Module dependency cycle detection |
| `scripts/check-tui-complexity.sh` | Complexity ratchet |

## Other guards

| Script | Purpose |
|---|---|
| `scripts/check-preserve-invariants.sh` | Preservation invariant tests for migration phases |
| `scripts/check-banned-imports.sh` | Cross-workspace banned-import policing |
| `scripts/check-context-window-literals.sh` | No hardcoded context windows or 4/5 budget fractions in production code |
| `scripts/check-r0-entry-gate.sh` | Mechanical re-verification of the learning roadmap's R0 entry gate. Also step 2b of ci-gate.sh; the `r0-entry-gate` CI job runs it with `fetch-depth: 0` and `--require-commits`, because commit ancestry cannot be checked from a shallow clone and the script reports a skip rather than pretending |
| `scripts/check-deferral-markers.sh` | Allowlisted cap on `TODO(TICKET)` / `FIXME(TICKET)` / `Deferred (TICKET)` markers. Also step 2c. Has `--self-test`, which plants markers in a synthetic tree and requires the same scan the real run uses to find them — and to ignore lowercase `TODO(name)` and bare `TODO:` |

Every script in this section runs in CI. A gate that is not wired in has no
enforcement and the thing it guards drifts past it unnoticed — that is what
happened to `check-context-window-literals.sh` (#132). If you add a gate here,
add it to `.github/workflows/ci.yml` in the same change.

## What a gate may not be

`scripts/tests/` once held 31 scripts, 24 of which grepped a hardcoded file
path for `pub fn <name>` to prove some slice had been wired up. They were
deleted in one change, and the reasoning generalises:

- **Do not assert what the compiler already asserts.** `render/mod.rs` calls
  `draw_skills_menu`; delete that function and the build fails. A gate that
  greps for the name restates the call site in a weaker language.
- **Do not hardcode paths.** Every one of the 8 that were failing when they
  were removed was a false alarm — a file split under the 500-line
  FileSizeGuard, or a symbol renamed. Zero real defects across every failure.
  The repo's real gate was breaking its fake ones.
- **Do not let a gate manufacture its evidence.** Two wrappers in
  `src/command/plan_file.rs` existed only so `pub fn plan_audit_path` would
  match. Nothing called them, so the module needed `#![allow(dead_code)]` to
  compile quietly.
- **Wire it, or do not write it.** Only two of the 31 were referenced by
  `ci.yml`; the other 29 had no call site anywhere in the tree, so their
  verdicts — true or false — were never seen by anyone.

If the claim is about Rust source, write a `#[test]` that exercises the
behaviour. `crates/archon-core/tests/plan_mode_interception_wired.rs` is the
worked example: it replaced a five-pattern grep by dispatching a real `Write`
in plan mode and asserting the file was not created and the attempt was
audited. Rename the function and it fails to compile; stop calling it and the
assertion goes red.

## WSL2 thread policy

`scripts/ci-gate.sh` enforces `--test-threads=2` on every cargo test invocation. Reasons:

1. REQ-FOR-D1/D2/D3 introduce shared global state (BACKGROUND_AGENTS DashMap, task registry, tempdir-based `.archon/`) that deadlocks under unlimited parallelism on WSL2 hosts.
2. Prior incidents (2026-04-11) crashed WSL2 when unlimited parallel rustc+test processes saturated the kernel; `--test-threads=2` is the project-wide safe floor.
3. Tests that need stricter isolation can opt into `#[serial_test::serial]` individually.

Native Linux / macOS / Windows tolerate higher concurrency, but ci-gate.sh keeps the WSL2 safe floor for portability.

## Pre-commit hook

Install the local hook bridge:
```bash
./scripts/install-hooks.sh
```

This wires git pre-commit to call `scripts/ci-gate.sh --skip-bench`. The full `ci-gate.sh` runs in CI (GitHub Actions).

## Sherlock review (separate concept — review pattern, not a gate)

When orchestrating subagent ticket execution, the parent context MUST run an independent cold-read audit before accepting any "COMPLETE" claim:

1. Independently re-read the diff (`git diff main..HEAD`)
2. Verify scope: only the spec'd files changed; nothing leaked
3. Run the tests independently
4. Run `cargo fmt --all -- --check` and `cargo build --release --bin archon -j1`
5. Confirm fresh binary mtime + version SHA matches HEAD
6. Approve OR reject with specific findings; never blanket-approve

This pattern applies REGARDLESS of which CI gates ran — it's about not trusting agent self-reports, not about which scripts to invoke.

## See also

- [Contributing](contributing.md) — workflow guide
- [Release process](release-process.md) — version bumps, tagging, deploy
- `scripts/ci-gate.README.md` in the repo — per-step rationale
