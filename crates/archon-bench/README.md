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
fixtures and validates every complete entry digest plus deterministic,
order-independent name-index, tag-index, and capability-index checksums before
timing. Entry digests include every `AgentMetadata` field; JSON schemas are
recursively canonicalized by object key before hashing. It compares exact
lookup, highest-version lookup, and tag/capability `AND` filter-candidate
construction (including both bucket clones) for equivalent representations.

### Issue #109 publication evidence

The reproducible decision group is
`catalog_representation/complete_publication`. Each timed iteration exactly
models production's publication statement,
`cached_snapshot.store(Arc::new(staging.clone()))`: it deep-clones the complete
representation, wraps it in `Arc`, and stores it in an equivalent `ArcSwap`
target. Fixtures and the initially empty targets are constructed outside the
timed loop; `black_box` covers the prepared snapshot, store input, and load
readback. Metadata validation, staging-lock acquisition, and staging/index
mutation are outside this boundary because they occur before the production
`publish` statement; neither representation includes them here.

Actual Criterion central estimates from the reproducible run are below.
`DashMap / standard-map` is the ratio, so a value above `1.0x` means the
DashMap-backed publication was slower; the final column is the standard-map
reduction relative to DashMap.

| Entries | DashMap clone + `ArcSwap::store` | Standard-map clone + `ArcSwap::store` | DashMap / standard-map | Standard-map reduction |
|---:|---:|---:|---:|---:|
| 100 | 127.28 µs | 75.648 µs | 1.683x | 40.57% |
| 1,000 | 1.1292 ms | 783.54 µs | 1.441x | 30.61% |
| 10,000 | 18.777 ms | 15.145 ms | 1.240x | 19.34% |

Acceptance threshold: pursue a representation migration only if this complete
publication group shows the standard-map representation at least 50% faster
than DashMap at **every** measured size (equivalently,
`DashMap / standard-map >= 2.0x` at every size). The observed reductions are
19.34%–40.57%, so the threshold is not met. Conclusion: **no migration** from
the production DashMap representation based on this evidence. This replaces
and does not rely on prior undocumented 0.89%/0.15% claims.

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
