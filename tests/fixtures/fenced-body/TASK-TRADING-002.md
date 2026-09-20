```markdown
```yaml
task_id: TASK-TRADING-002
title: Registry v2 storage, fail-closed migration, and atomic dataset writes
complexity: high
status: ready
depends_on: [{task_id: TASK-TRADING-001, consumes: [{artifact_path: docs/trading/data-lake-gap-audit.md}], ordering_only: false}]
blocks: [TASK-TRADING-003, TASK-TRADING-004, TASK-TRADING-010]
implements: [REQ-DL-001, REQ-DL-002, REQ-DL-003, REQ-DL-004, REQ-DL-005, REQ-DL-010, REQ-DL-073, REQ-DL-080, REQ-DL-081, REQ-DL-082, REQ-DL-083, REQ-DL-084, REQ-DL-085, REQ-DL-130, REQ-DL-131, REQ-DL-132, REQ-DL-133, AC-DL-003, DONE-1, DONE-2, DONE-3]
required_env_keys: []
required_tools: []
deliverable_contracts: [{kind: rust-source, artifact_path: crates/archon-trading/src/data_lake.rs, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}, {kind: rust-source, artifact_path: crates/archon-trading/src/data_lake/registry.rs, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}, {kind: rust-source, artifact_path: crates/archon-trading/src/data_lake/migration.rs, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}, {kind: rust-source, artifact_path: crates/archon-trading/src/data_store.rs, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}, {kind: rust-test, artifact_path: crates/archon-trading/tests/registry_v2.rs, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}, {kind: rust-test, artifact_path: crates/archon-trading/tests/migration_v1_to_v2.rs, min_instances: 1, required_universe: false, registry_minimum_count: 0, series_overlap_min_rows: 0}]
```

## Scope

Upgrade the existing project-local Trading Lab store into Trading Data Lake v1 storage foundations, per PRD `PRD-TRADING-DATA-LAKE-AHDM-001` (§8.1, §8.2, §17–§19, §26) and the gap audit produced by TASK-TRADING-001 (`docs/trading/data-lake-gap-audit.md`). This task delivers the registry v2 storage contract, the non-destructive v1→v2 migration, and the fail-closed atomic dataset-write path. It does **not** deliver validation checks/report semantics (TASK-TRADING-003), provider capability checks (TASK-TRADING-004), or backtest gates (TASK-TRADING-010); those depend on this task's registry surface.

### Requirements implemented

- **REQ-DL-001** — all storage stays under the existing root `.archon/trading-lab/data`; `TradingDataLake::data_root()` (`crates/archon-trading/src/data_store.rs`) resolves `<project>/.archon/trading-lab/data` and no second storage location is introduced.
- **REQ-DL-002** — raw provider responses persist under each dataset version (`datasets/<dataset-id>/<version>/raw/…`: `request.json`, `headers.redacted.json`, `provider-notes.md`, raw body) via `TradingDataLake::write_dataset`.
- **REQ-DL-003** — normalized OHLCV persists as replayable JSONL (`ohlcv.jsonl`, one candle per line, sorted ascending by `ts`).
- **REQ-DL-004** — registry is content-addressed: `RegistryV2.datasets[dataset_id][version]` (`crates/archon-trading/src/data_lake/registry.rs`), schema key `schema_version: "archon-trading-data-registry-v2"`.
- **REQ-DL-005** — dataset writes are atomic: temp-file + same-directory rename for registry and dataset artifacts; an interrupted ingest cannot leave a healthy registry entry pointing at missing artifacts (`verify_artifacts` runs before a registry record is published, and duplicate id/version with changed bytes is rejected).
- **REQ-DL-010** — dataset metadata carries the full §19 field set (dataset id, version, canonical instrument, provider, provider symbol, asset class, timeframe, native interval flag, session, timezone, adjustment policy, row count, start/end, checksums, artifact paths, ingestion timestamp, source notes, license notes, production eligibility), enforced by `validate_metadata` in `crates/archon-trading/src/data_lake.rs`.
- **REQ-DL-073** — re-ingesting identical raw content reuses the existing version only when the raw checksum matches exactly; a same id/version with different normalized or raw bytes is a typed error.
- **REQ-DL-080** — metadata is deterministic JSON (`serde_json` canonical pretty serialization with stable key order via `BTreeMap`-backed schema).
- **REQ-DL-081** — all paths stored in metadata/records are relative to the project root.
- **REQ-DL-082** — secrets, API keys, bearer tokens, cookies, and session ids are rejected before any write (`contains_secret_material` / `contains_secret_bytes` / `contains_secret_text` on raw body, request, redacted headers, metadata, and provider notes).
- **REQ-DL-083** — a missing `native_interval` field means `false`.
- **REQ-DL-084** — a missing `production_eligible` field means `false`.
- **REQ-DL-085** — the existing `archon-trading-data-registry-v1` registry remains readable (`load_registry` accepts the v1 `schema`-keyed flat layout); loading never rewrites the file; v1 metadata is upgraded in memory on read and rewritten only during explicit migration or dataset update.
- **REQ-DL-130** — migration never deletes raw or normalized dataset files: only `registry.json` and `registry-migration-report.json` are created/modified.
- **REQ-DL-131** — unknown native-interval/production status and unknown registry schemas fail closed; a failed migration leaves the registry bytes untouched.
- **REQ-DL-132** — migration is idempotent: a second run on a v2 registry is a byte-identical no-op with a zero-count report.
- **REQ-DL-133** — migration reports counts `migrated`, `skipped`, `degraded`, `failed` (written to `.archon/trading-lab/data/registry-migration-report.json`), plus the backup path.
- **AC-DL-003** — native OHLCV ingestion stores raw artifact, normalized JSONL, metadata, validation report, and registry entry (the write path persists all five and verifies them before publishing the registry record).
- **DONE-1 / DONE-2 / DONE-3** — existing data root upgraded not replaced; v1 registry migration implemented; dataset metadata includes native/provenance fields.

## Files Expected to Change

- `crates/archon-trading/src/data_lake.rs` — exists (382 lines)
- `crates/archon-trading/src/data_lake/registry.rs` — exists (312 lines)
- `crates/archon-trading/src/data_lake/migration.rs` — exists (241 lines)
- `crates/archon-trading/src/data_store.rs` — exists (413 lines)
- `crates/archon-trading/tests/registry_v2.rs` — exists (162 lines)
- `crates/archon-trading/tests/migration_v1_to_v2.rs` — exists (196 lines)

Notes on each deliverable:

- `crates/archon-trading/src/data_lake.rs` — module root for the data lake: `DataLakeError`, metadata validation (`validate_metadata` including REQ-DL-020/083/084 fail-closed flag handling), re-exports of `registry::*` and `migration::*`, dataset-id/provider identity checks, and secret-free redaction re-exports. Changes here are limited to surface needed by the registry/migration/store contract.
- `crates/archon-trading/src/data_lake/registry.rs` — the v2 storage contract: `REGISTRY_SCHEMA_V1`/`REGISTRY_SCHEMA_V2`, `RegistryV2` (nested `datasets[dataset_id][version]`), `RegistryV1Flat`, `load_registry` (accepts v1 `schema` key, never rewrites), `write_registry_v2` / `write_registry_v1_flat` / `publish_registry_v2` / `prepare_registry_for_ingest` (atomic temp+rename, backup on content change), and `DatasetIdComponents::parse` (§18 naming, REQ-DL-070/071).
- `crates/archon-trading/src/data_lake/migration.rs` — `migrate_registry_v2`: missing file → empty v2 registry; v1 → preserve all records, degrade unknown flags fail-closed, write backup `registry.json.backup-<timestamp>`, write `registry-migration-report.json` with the four REQ-DL-133 counts; v2 → byte-identical no-op; unknown schema → typed error before any write.
- `crates/archon-trading/src/data_store.rs` — `TradingDataLake` implementation: root/path resolution (REQ-DL-001), `store_ohlcv` (checksum-bound version reuse per REQ-DL-073, secret rejection per REQ-DL-082, coverage reconciliation), `write_dataset` (atomic artifact writes, verification before registry publish), `validate_ohlcv`, `load_ohlcv`, and registry migration plumbing (`load_registry_migration` with backup).
- `crates/archon-trading/tests/registry_v2.rs` — focused tests for the v2 schema contract (round-trip with `schema_version` key, v1 flat records intact under nested layout, unknown schema fails closed, idempotent write with no spurious backups).
- `crates/archon-trading/tests/migration_v1_to_v2.rs` — focused tests for the migration contract (record preservation, fail-closed flag defaults, exactly one backup per content change, four counts reported, idempotent second run, missing-file and unknown-schema behavior).

## Files Forbidden to Change

- `crates/archon-trading/src/validation.rs` and `crates/archon-trading/src/validation/*` — TASK-TRADING-003 scope.
- `crates/archon-trading/src/data_lake/provider_capability.rs`, `crates/archon-trading/src/providers/capability.rs` — TASK-TRADING-004 scope.
- `crates/archon-trading/src/data_lake/tradingview_mcp.rs`, `crates/archon-trading/src/data_store/tradingview_ingest.rs`, `crates/archon-trading/src/data_lake/stooq.rs`, `crates/archon-trading/src/data_lake/providers/*` — provider adapter scope (TASK-TRADING-005…008).
- `crates/archon-trading/src/coverage.rs`, `crates/archon-trading/src/backtest_gates.rs`, `src/command/trading_data.rs` — TASK-TRADING-009/010 scope.
- `crates/archon-trading/src/ahdm_backtest*`, `crates/archon-trading/src/pine_lab.rs` — AHDM strategy scope (TASK-TRADING-011…014).
- `.mcp.json`, `docs/`, `prds/` — no MCP wiring or documentation changes in this task.

## Acceptance Criteria

1. `load_registry` reads both `archon-trading-data-registry-v2` (nested) and `archon-trading-data-registry-v1` (`schema` key, flat `"<dataset_id>:<version>"` keys) without modifying the file on disk (REQ-DL-085).
2. `write_registry_v2` emits `schema_version: "archon-trading-data-registry-v2"` with the nested content-addressed layout, writes via temp-file + rename, and backs up the previous file only on content change (REQ-DL-004, REQ-DL-005).
3. `migrate_registry_v2` preserves every v1 record verbatim, defaults absent `native_interval`/`production_eligible` to `false` in the migrated view, writes exactly one backup per content change, and reports the REQ-DL-133 counts plus backup path (REQ-DL-130, REQ-DL-131, REQ-DL-133).
4. A second migration run on a v2 registry does not modify the registry bytes and reports zero counts (REQ-DL-132).
5. An unknown registry schema aborts migration before any write, leaving registry bytes unchanged (REQ-DL-131).
6. `store_ohlcv` rejects secret material in any persisted artifact (REQ-DL-082), reuses an existing version only on exact raw-checksum match (REQ-DL-073), and publishes the registry entry only after all artifacts verify (REQ-DL-005).
7. `validate_metadata` fails on missing §19 fields, treats missing `native_interval`/`production_eligible` as `false` (REQ-DL-083/084), and rejects production eligibility on non-native or incomplete metadata.
8. Focused tests in `crates/archon-trading/tests/registry_v2.rs` and `crates/archon-trading/tests/migration_v1_to_v2.rs` pass, and each can fail when the corresponding business logic breaks (they assert on schema keys, record contents, byte identity, counts, and backup behavior — not just "something returned").
9. Changed/new source files stay under 500 lines (NFR-001) and new functions stay at cyclomatic complexity ≤ 15 (NFR-002).

## Focused Tests

Run only these focused tests during implementation (NFR-004); do not run the full workspace suite:

```bash
cargo nextest run -p archon-trading --test registry_v2
cargo nextest run -p archon-trading --test migration_v1_to_v2
cargo nextest run -p archon-trading --test registry_schema_v1 --test registry_migration_v1
cargo nextest run -p archon-trading --lib data_lake::tests
cargo nextest run -p archon-trading --lib data_store::data_store_schema_tests
```

Test-first order (Gate 1): extend `crates/archon-trading/tests/migration_v1_to_v2.rs` and `crates/archon-trading/tests/registry_v2.rs` with any new contract case (e.g. checksum-bound version reuse, secret-rejection write path) before touching the source modules, confirm the new assertions fail against the current code, then make them pass.

If a live provider is unreachable or a credential is absent, the focused tests above remain fully runnable: they are hermetic (tempfile fixtures only) and require no environment keys.

## Commands

```bash
cargo check -p archon-trading --tests
cargo nextest run -p archon-trading --test registry_v2 --test migration_v1_to_v2
cargo clippy -p archon-trading -- -D warnings
cargo fmt --all -- --check
```

## Checks

- **Line-count check** (NFR-001): every changed or new file listed under Files Expected to Change stays under 500 lines; `crates/archon-trading/src/data_store.rs` must not exceed the 500-line preference — extract new logic into the existing child modules (`data_store/registry.rs`, `data_store/io.rs`, `data_store/records.rs`) rather than growing the root file past it.
- **Complexity check** (NFR-002): new/changed functions stay at cyclomatic complexity ≤ 15; prefer early returns in `validate_metadata` and `migrate_registry_v2` paths; extract helpers before adding a second concern.
- **Atomicity check** (REQ-DL-005): every registry/artifact write path goes through temp-file + rename; no in-place mutation of `registry.json`.
- **Fail-closed check** (REQ-DL-083/084/131): absent flags and unknown schemas produce typed errors or `false` defaults — never an optimistic default.

## Adversarial Review Notes

- **Silent data loss**: migration must be proven non-destructive by tests that snapshot the datasets directory and assert no file outside `registry.json` / `registry-migration-report.json` changed (REQ-DL-130). A migration that "works" but drops a record is the primary failure mode.
- **Optimistic defaults**: defaulting unknown `native_interval`/`production_eligible` to `true` would violate fail-closed policy; tests must assert `false` defaults explicitly.
- **Partial-write registry**: a crash between artifact write and registry publish must not leave a healthy record pointing at missing artifacts; `verify_artifacts` before publish is the guard, and the duplicate id/version checksum-mismatch path (REQ-DL-073) must reject rather than overwrite.
- **Secret leakage**: redaction tests must include a bearer-token-bearing header fixture and assert the token never appears in any persisted artifact (REQ-DL-082 / NFR-006).
- **v1/v2 round-trip drift**: `prepare_registry_for_ingest` downgrades v2 → flat v1 for legacy writers; a subsequent `publish_registry_v2` must restore the nested shape without losing records — the ingest interop path is exercised by `registry_migration_v1` focused tests.
- **Idempotence illusion**: a no-op migration that still rewrites bytes (timestamp drift, key reorder) would break REQ-DL-132; the byte-identity assertion is the guard.

## Residual Gaps

None for this task's scope. Validation report semantics (REQ-DL-050…056, §21 checks) are deliberately out of scope here and are TASK-TRADING-003's deliverable; until that task lands, `validate_ohlcv` writes reports through the existing `crate::validation` engine and fails closed on `ValidationStatus::Failed`.
```