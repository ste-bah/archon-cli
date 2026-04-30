# CI gates

archon-cli's CI flow is technical (compile / lint / test). The orchestrator is `scripts/ci-gate.sh` — single source of truth. Any GitHub Actions / GitLab / local hook should call this script rather than replicate its steps.

> **Note:** This is **NOT** root archon's narrative 6-gate Sherlock-review protocol. Root archon (`/home/unixdude/Archon-projects/archon/`) has `scripts/dev-flow-gate.sh` and `scripts/dev-flow-pass-gate.sh` for `project-tasks/TASK-NNN-*` tracking with PreToolUse hooks. archon-cli has neither of those scripts and no equivalent task-tracking enforcement. When working on archon-cli, follow the technical ci-gate flow on this page.

## The 7 ci-gate steps

```
Step 1 — FileSizeGuard           — scripts/check-file-sizes.sh, ratchet-style allowlist
Step 2 — BannedImports           — scripts/check-banned-imports.sh, allowlist-driven
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
| 1. FileSizeGuard | Files over the 1500-line cap accumulate complexity. Ratchet allowlist documents grandfathered over-size files. New code must comply. |
| 2. BannedImports | Workspace-wide policy on cross-crate imports. Prevents architectural creep (e.g. archon-tui depending on archon-pipeline directly). |
| 3. cargo fmt | Format consistency. No exception. |
| 4. cargo clippy | Lint with warnings-as-errors. No `#[allow(...)]` to silence — fix the underlying issue. |
| 5. cargo test | Workspace-wide test run. `--test-threads=2` is mandatory because of shared global state (BACKGROUND_AGENTS, task registry, tempdir-based `.archon/`) that deadlocks under unlimited parallelism on WSL2. |
| 6. baseline test-list diff | Detects accidentally added or removed tests. Update the baseline only deliberately. |
| 7. cargo bench --no-run | Bench compile check — catches bench-only API drift without running benchmarks. |

## TUI-specific gates

Run separately from ci-gate.sh; invoked from TUI workflow paths:

| Script | Purpose |
|---|---|
| `scripts/tui-file-size-gate.sh` | Ratchet-style file-size enforcement for `crates/archon-tui/` |
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
