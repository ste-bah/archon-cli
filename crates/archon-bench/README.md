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
| `catalog_representation` | Issue #109 decision evidence | none (measurement only) | Issue #109 | `benches/catalog_representation.rs` |

Limits live in `threshold.toml` — the single source of truth. Bench
bodies read that file at runtime and assert against it.

`catalog_representation` uses deterministic 100-, 1,000-, and 10,000-agent
fixtures and validates deterministic, order-independent entry, name-index,
tag-index, and capability-index checksums before timing. It compares clone,
exact lookup, highest-version lookup, and tag/capability `AND` filter-candidate
construction (including both bucket clones) for equivalent representations; it
does not measure or claim `ArcSwap` publication, metadata collection, sorting,
or complete catalog listing.

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
