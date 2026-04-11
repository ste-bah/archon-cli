# `ci-gate.sh` — Archon-CLI CI orchestrator

One script, run in order, fail-fast. Every CI provider should call this
script instead of replicating its steps inline, so there is one source of
truth for "what does CI check".

Reference: **TASK-AGS-007** (phase-0 prereqs) —
`project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-007.md`.

## Why `--test-threads=2`

Every `cargo test` invocation in this repo runs with `--test-threads=2`.
This is hard-coded in `ci-gate.sh` and is the project-wide safe floor.

1. **REQ-FOR-D1 / D2 / D3** introduce shared global state — the
   `BACKGROUND_AGENTS` DashMap, the task registry, and tempdir-backed
   `.archon/` state. Unlimited parallelism deadlocks on WSL2 hosts.
2. **WSL2 stability** — prior incidents (2026-04-11) crashed the Linux
   subsystem when unlimited parallel `rustc` + test processes saturated
   the kernel. A single-thread floor is too slow for CI budgets; two
   threads balance throughput vs. stability.
3. **Per-test opt-in** — tests that need strict isolation annotate
   themselves with `#[serial_test::serial]` (dev-dep added by
   TASK-AGS-006). The `--test-threads=2` floor is the *default*, not a
   ceiling.

Contributors running `cargo test` locally should export
`CARGO_TEST_THREADS=2` or alias `cargo test` to pass `-- --test-threads=2`.
A future follow-up may encode this in `.cargo/config.toml`; this task
intentionally does not.

## Steps (in order)

| # | Step            | Enforces                                     | Source       |
|---|-----------------|----------------------------------------------|--------------|
| 1 | file-sizes      | ≤500 lines per file (NFR-FOR-D4)             | TASK-AGS-002 |
| 2 | banned-imports  | No `mcp__memorygraph__`, no legacy shims     | TASK-AGS-003 |
| 3 | fmt             | `cargo fmt --all -- --check`                 | stdlib       |
| 4 | clippy          | `-D warnings` workspace-wide                 | stdlib       |
| 5 | test            | `cargo test --workspace -- --test-threads=2` | TASK-AGS-007 |
| 6 | baseline-diff   | No test silently removed                     | TASK-AGS-001 |
| 7 | bench           | `cargo bench -p archon-bench --no-run`       | TASK-AGS-005 |

The wrapper exits non-zero on the first failing step. Steps 6 and 7 depend
on the artifacts from steps 5 and on prior phase-0 tasks; the failure
messages cite which task owns the missing piece.

## Flags

- `--only <step>` — run a single step and exit. Step keys are:
  `file-sizes`, `banned-imports`, `fmt`, `clippy`, `test`, `baseline-diff`,
  `bench`. Useful for local debugging.
- `--skip-bench` — skip step 7. **Local dev convenience only.** CI must
  always run the full pipeline; this flag exists for contributors without
  the criterion build cached.

## Intentional omissions

- **No GitHub Actions YAML.** `.github/workflows/*.yml` is not touched
  by this task. Each CI provider wires into `ci-gate.sh` as a five-line
  shell job; a follow-up outside phase-0 may add the YAML wrapper.
- **No benchmark threshold enforcement.** Step 7 only verifies the
  harness compiles (`--no-run`). Bench bodies and their
  `threshold.toml` assertions are owned by phase-1..3 tasks.
- **No automatic baseline regen.** The baseline at
  `tests/fixtures/baseline/cargo_test_list.txt` is frozen by design.
  Regen is manual via TASK-AGS-001's `regen-baseline.sh`.

## Validation (TASK-AGS-007)

1. `bash scripts/ci-gate.sh` runs end-to-end on a clean checkout, exits 0.
2. `bash scripts/ci-gate.sh --only fmt` runs only fmt.
3. `bash -x scripts/ci-gate.sh --only test 2>&1 | grep -- '--test-threads=2'`
   shows the literal in the trace.
4. Injecting a banned import causes failure at step 2 without reaching
   step 3.
5. Deleting a test causes step 6 to print `ERROR: tests were removed`.
6. `scripts/ci-gate.sh` ≤200 lines, POSIX + cargo only.
