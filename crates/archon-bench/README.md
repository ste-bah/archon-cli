# archon-bench

Criterion benchmark harness enforcing archon-cli NFR-PERF gates. Phase-0
creates only the skeleton (this task, TASK-AGS-005). Real bench bodies
are owned by later phase tasks — do NOT merge bodies into this crate
from phase-0.

## Benches and owners

| Bench              | NFR                  | Limit    | Owning phase  | Reference                 |
|--------------------|----------------------|----------|---------------|---------------------------|
| `task_submit`      | NFR-PERF-001         | 100 ms p95 | phase-1       | 02-technical-spec §374    |
| `discovery_scan`   | NFR-PERF-002         | 1000 ms p95 | phase-3       | 02-technical-spec §546    |
| `fanout_100`       | NFR-SCALABILITY-001  | 1000 ms p95 | phase-5       | 02-technical-spec §862    |

Limits live in `threshold.toml` — the single source of truth. Bench
bodies read that file at runtime and assert against it.

## Phase-0 stubs

Every bench currently calls `b.iter(|| {})` inside a `bench_*_stub`
function. This guarantees:

- `cargo check -p archon-bench` succeeds.
- `cargo bench -p archon-bench --no-run` compiles all three benches.
- CI can run `cargo bench -p archon-bench <name> -- --test` for a
  smoke check without waiting on full criterion iterations.

Phase-1..3 tasks replace the stub body with real work and add real
assertions against `threshold.toml`.

## Not in this crate

- CI wiring — owned by TASK-AGS-007 (`dev-flow-run.sh`).
- The 300-agent discovery fixture — owned by TASK-AGS-004; this crate
  only consumes it at phase-3 time.
- Any `criterion` leakage into production crates — keep the dev-dep
  localised here.
