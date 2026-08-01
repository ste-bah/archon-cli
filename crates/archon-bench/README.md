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
timing. The capability checksum is also asserted after a complete clone. Entry
digests include every `AgentMetadata` field; JSON schemas are recursively
canonicalized by object key before hashing. It compares production-equivalent
exact lookup, highest-version resolution, and tag/capability `AND` indexed reads.

### Issue #109 Criterion evidence

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

Each read benchmark uses a representation-specific façade with the same work:
`ArcSwap::load_full`, lookup or descending version scan, `AgentMetadata` clone,
valid-state filtering, and a consumed metadata/result checksum. The indexed
façades additionally clone both index buckets, construct the intersection, and
clone/filter each returned metadata entry. The `ArcSwap` targets are built before
timing.

The table contains **Criterion median point estimates** from the recorded run
below; these are medians, not means. `DashMap / standard-map` above `1.0x` means
DashMap took longer, so the standard-map representation is faster. The
standard-map change is `(standard / DashMap - 1)`; a positive value would be a
regression.

Command and artifacts: `cargo bench -p archon-bench --bench
catalog_representation -- --sample-size 100 --measurement-time 3 --warm-up-time
1`; Criterion `new/estimates.json` artifacts were read from `target/criterion/`
after that run.

| Group | Entries | DashMap median | Standard-map median | DashMap / standard-map | Standard-map change |
|---|---:|---:|---:|---:|---:|
| Complete publication | 100 | 126.340 µs | 75.916 µs | 1.6642x | -39.91% |
| Complete publication | 1,000 | 1.178800 ms | 784.700 µs | 1.5022x | -33.43% |
| Complete publication | 10,000 | 16.467000 ms | 11.877000 ms | 1.3865x | -27.87% |
| Exact get | 100 | 5.489 µs | 5.475 µs | 1.0026x | -0.26% |
| Exact get | 1,000 | 5.525 µs | 5.529 µs | 0.9993x | +0.07% |
| Exact get | 10,000 | 5.329 µs | 5.390 µs | 0.9887x | +1.14% |
| Highest-version resolution | 100 | 5.567 µs | 5.519 µs | 1.0087x | -0.86% |
| Highest-version resolution | 1,000 | 5.388 µs | 5.444 µs | 0.9897x | +1.04% |
| Highest-version resolution | 10,000 | 5.507 µs | 5.507 µs | 1.0000x | +0.00% |
| Tag/capability indexed read | 100 | 20.505 µs | 20.777 µs | 0.9869x | +1.33% |
| Tag/capability indexed read | 1,000 | 202.250 µs | 198.650 µs | 1.0181x | -1.78% |
| Tag/capability indexed read | 10,000 | 1.955300 ms | 1.975200 ms | 0.9899x | +1.02% |

Binding acceptance threshold: complete-publication median improvement must be
at least **15%** at both 1,000 and 10,000 entries, while each
production-equivalent exact-get, highest-version-resolution, and indexed-read
median regression must be at most **10%** at both sizes. The publication gains
are 33.43% and 27.87%; the largest read regression is 1.33%. The binding gate
therefore **passes**. Compatibility-first V2 production migration is **required**;
production stages and publishes the measured `ImmutableCatalogSnapshot` type, while
the deprecated legacy snapshot API performs compatibility conversion on demand.

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
