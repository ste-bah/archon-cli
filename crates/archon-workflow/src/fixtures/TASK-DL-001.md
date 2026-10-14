```yaml
blocks:
- TASK-DL-002
- TASK-DL-003
- TASK-DL-008
- TASK-DL-009
- TASK-BT-001
- TASK-AHDM-002
- TASK-AHDM-004
complexity: high
deliverable_contracts:
- artifact_path: .archon/trading-lab/data/registry.json
  kind: trading-data-registry
  min_instances: 1
  registry_minimum_count: 0
  required_universe: false
  series_overlap_min_rows: 0
- artifact_path: .archon/trading-lab/data
  kind: trading-data-lake-root
  min_instances: 1
  registry_minimum_count: 0
  required_universe: false
  series_overlap_min_rows: 0
- artifact_path: .archon/trading-lab/data/datasets
  kind: dataset-version-store
  min_instances: 1
  registry_minimum_count: 0
  required_universe: false
  series_overlap_min_rows: 0
- artifact_path: crates/archon-trading/src/data_lake.rs
  kind: data-lake-registry-source
  min_instances: 1
  registry_minimum_count: 0
  required_universe: false
  series_overlap_min_rows: 0
- artifact_path: crates/archon-trading/tests/migration_v1_to_v2.rs
  kind: migration-tests
  min_instances: 1
  registry_minimum_count: 0
  required_universe: false
  series_overlap_min_rows: 0
depends_on: []
implements:
- REQ-DL-001
- REQ-DL-002
- REQ-DL-003
- REQ-DL-004
- REQ-DL-005
- REQ-DL-010
- REQ-DL-011
- REQ-DL-012
- REQ-DL-013
- REQ-DL-070
- REQ-DL-071
- REQ-DL-072
- REQ-DL-073
- REQ-DL-074
- REQ-DL-080
- REQ-DL-081
- REQ-DL-082
- REQ-DL-083
- REQ-DL-084
- REQ-DL-085
- G-DL-001
- G-DL-003
- REQ-DL-130
- REQ-DL-131
- REQ-DL-132
- REQ-DL-133
- DONE-1
- DONE-2
- DONE-3
- DONE-12
- AC-DL-003
required_env_keys: []
required_tools: []
status: ready
task_id: TASK-DL-001
title: Trading Data Lake v1 registry, dataset-version store, and v1-to-v2 migration
```

## Scope

Upgrade the existing Trading Lab store into Trading Data Lake v1 (G-DL-001, G-DL-003) at the existing root `.archon/trading-lab/data` (REQ-DL-001). The store is extended, never replaced and never duplicated (PRD §2). This task owns the registry schema, the dataset-version store layout, the §19 metadata contract, the §18 dataset identity, and the §26 v1→v2 migration. It owns no provider adapters, no validation rules, no coverage matrix, no commands, no backtest gates, and no AHDM work — those belong to the blocked tasks.

**Attempt-6 repair mandate (authoritative finding, carried to closure here):**

1. **REQ-DL-003 must be made true, not merely permitted.** The finding holds that a completion claim resting only on the *permission* structure (an empty-but-v2 registry is a legal persisted state and the idempotent no-op REQ-DL-132 path) leaves the obligation false at the project. The repair, carried inside this body: a **mandatory real-root ingest leg** is added beside the real-root migration. The `#[ignore]`-gated test `data_lake::tests::ingest_real_project_root_fixture` (wrapper in `tests.rs`, impl in a new `real_root_ingest.rs` beside `real_root_migration.rs`, `pub(super)` like its sibling) ingests one small verbatim OHLCV fixture through the real store path (`TradingDataLake::store_ohlcv`, `data_store.rs:136`) against the **project data root**, then asserts post-conditions on disk: the dataset version directory `.archon/trading-lab/data/datasets/<dataset-id>/<version>/` exists carrying `metadata.json`, `validation.json`, `manifest.json`, `ohlcv.jsonl`, and `raw/` (`response.csv`, `request.json`, `headers.redacted.json`, `provider-notes.md`); `ohlcv.jsonl` is non-empty, ascending by `ts`, and round-trips to the ingested rows; `registry.json` is v2 and contains the ingested `datasets[dataset_id][version]` entry (REQ-DL-004 content addressing) whose checksum agrees with the metadata `normalized_sha256`. Re-ingesting the **same fixture bytes** must reuse-or-update the version idempotently without duplicating registry entries or deleting artifacts (REQ-DL-073 exact-checksum reuse; REQ-DL-130 never-delete). The printed `real-root ingest report: dataset=<id> version=<v> rows=<n> status=<validation status>` line is recorded completion evidence. A completion claim without that executed, passing, recorded run is false (fail loud) — the persistence obligation is discharged by the run, not by a contract constant. `registry_minimum_count: 0` and `series_overlap_min_rows: 0` stay frozen contract values; this run is what populates the store so REQ-DL-003 is observably true, and after it the registry is no longer empty, closing the loophole the finding quotes (an empty registry remains legal only *before* this run; the run itself is unconditional).

This mandatory ingest run composes with, and never replaces, the mandatory real-root migration run (attempt-2 repair 1, retained unchanged below): migration first (v2 registry exists), then ingest (datasets persisted). Both runs are recorded in the completion evidence. If the ingest run fails, the failure is repaired at its source and re-run; there is no skip, conditional, or defer path.

The implementer's job is surgical completion of the frozen contracts below — every changed line traces to a REQ in `implements`; no re-architecture, no new crate, no new dependencies (`serde`, `serde_json`, `chrono`, `blake3` in `crates/archon-trading/Cargo.toml`; `tempfile` and `serial_test` dev-deps already cover the tests).

Work items:

1. **Registry v2 content addressing (REQ-DL-004, §17).** `datasets[dataset_id][version]` with `schema_version: "archon-trading-data-registry-v2"` always written. The legacy v1 flat layout (`schema` key, `"<dataset-id>:<version>"` keys) must keep loading with every record preserved (REQ-DL-085). Reads never rewrite bytes; writes are temp-file + rename inside the target directory with a backup of the previous bytes on any content change.
2. **Dataset identity — the complete REQ-DL-070…074 rule.** A dataset id is valid iff **all** of: (a) no whitespace (REQ-DL-070); (b) lossless decomposition into exactly four non-empty components `<provider>-<canonical-instrument>-<timeframe>-<price-basis>`; (c) every component alphanumeric; (d) encoded components agree with the stored metadata identity — `provider` (case-normalized), `canonical_instrument`, `normalize_timeframe(timeframe)`, `price_basis` — exactly, or extend it only by a provenance suffix (`…-raw-tdl080-live200` carve-out already accepted by `dataset_id_matches_metadata`, `identity.rs:48–75`); (e) provider prefix matches the metadata provider for known providers (`provider_identity_matches_dataset`, `data_lake.rs:345–359`). Version is `<YYYYMMDD>-<suffix>`; a version is reused only on an exact raw-checksum match (REQ-DL-073, `data_store.rs:256–262`); changed raw content forces a new version (REQ-DL-072); ids are stable across machines and sessions (REQ-DL-074 — pure function of identity components, `identity.rs:5–12`). Rejections carry the exact offending id and violated rule (`DatasetIdComponents::parse`, `registry.rs:304–331`; lib pin `tests.rs:244–265`; integration pins `dataset_naming.rs` six tests).
3. **Metadata contract (REQ-DL-010, REQ-DL-080…085, §19).** All §19 fields persisted per dataset version, including `native_interval`, `production_eligible`, `coverage`, `checksums`, `paths`, and `source`. JSON deterministic where possible (REQ-DL-080), paths root-relative (REQ-DL-081), no secrets anywhere (REQ-DL-082, NFR-006), missing `native_interval`/`production_eligible` read as `false` (REQ-DL-083/084), v1 records upgraded in memory on read with on-disk bytes untouched (REQ-DL-085, pinned by `in_memory_v1_read_upgrade_leaves_disk_bytes_identical` and `ingest_preparation_never_downgrades_the_registry`). Production eligibility requires `native_interval=true` plus `quality_status=passed`; yfinance is always degraded and never production-eligible; provider identity cannot be mislabelled.
4. **Asset-class provenance is required content, not a tag (REQ-DL-011/012/013).** The persisted `metadata.json` of every dataset version must carry the §19 disclosure its asset class owes, and `validate_metadata` — the shared gate behind registration and both store write/validate paths — must **reject** the dataset, naming the dataset id and the exact missing disclosure, when the record is absent or bare:
   - **REQ-DL-011 (futures — `asset_class` `future`/`futures`):** a §19-shaped `continuous_contract` record carrying non-empty continuous-contract `notes`, a `roll_policy` (the known policy, or an explicit unknown-policy record such as `unknown`/`not published`), and non-empty `provider_symbol_semantics`.
   - **REQ-DL-012 (equity ETF — `asset_class` `equity`/`etf`):** the adjusted/unadjusted policy carried by the mandatory `adjustment` field **plus** non-empty explicit corporate-action limitation record `corporate_action_notes`.
   - **REQ-DL-013 (crypto — `asset_class` `crypto`):** `session` recording the 24/7 assumption in its `24x7` form and non-empty `exchange_source`.
   
   Vehicle, resolved per ARCHON.md Rule 6 (one pattern, no blending): the §19-shaped top-level records are the single persisted contract. The unwired `AssetProvenance` variant structs in `model.rs` are removed in the same change and replaced by a `ContinuousContractProvenance` struct (`is_continuous`, `provider_symbol_semantics`, `roll_policy`, `notes`) plus the `corporate_action_notes` and `exchange_source` string fields on `DatasetMetadata`; the stale `// Includes typed asset-specific provenance…` comment on the `registry` re-export in `data_lake.rs` is corrected. All three additions serialize only when populated (`#[serde(default, skip_serializing_if = …)]`), so writers that record no provenance emit byte-identical metadata and existing pinned-artifact tests stay green. Reads stay lenient — `upgrade_v1_record_in_memory` and v1 defaults leave missing provenance empty rather than failing a read (REQ-DL-085); the gate binds writes. Compile-driven completion of every exhaustive `DatasetMetadata` literal and disclosure additions to lanes/fixtures constructing affected asset classes are part of this work item: mechanical, behavior-neutral edits forced by the compiler or by the named focused tests, with the exported store-lane crypto constants (`YFINANCE_CRYPTO_SESSION`, `YFINANCE_CRYPTO_EXCHANGE_SOURCE` in `crates/archon-trading/src/data_store/yfinance_ingest.rs`) remaining the single source of truth for the 24/7 and exchange-source wordings.
5. **Dataset-version store (REQ-DL-002, REQ-DL-003, REQ-DL-005, AC-DL-003, §17).** Each ingest writes `.archon/trading-lab/data/datasets/<dataset-id>/<version>/` containing `metadata.json`, `validation.json`, `manifest.json`, `ohlcv.jsonl` (normalized, replayable, ascending `ts`), and `raw/` (`response.*`, `request.json`, `headers.redacted.json`, `provider-notes.md`) with the raw provider bytes stored verbatim per version. Writes are atomic enough that an interrupted ingest never leaves a healthy registry entry pointing at missing artifacts (REQ-DL-005). The store-side code lives under `crates/archon-trading/src/data_store.rs` (its `#[cfg(test)]` child modules at `data_store.rs:450–461` carry the ingest contract tests) and is TASK-DL-002's source contract: TASK-DL-001 proves this layout through its focused tests and must not restructure `data_store.rs`; defects found there are handed to TASK-DL-002, not patched here.
6. **Migration v1→v2 (REQ-DL-130…133, §26).** `migrate_registry_v2` is non-destructive (touches only `registry.json`, its backup, and `registry-migration-report.json` beside the registry — REQ-DL-130), fail-closed (unknown flags default `false`; unknown schema aborts before any write — REQ-DL-131), idempotent (second run is a byte-identical no-op with zero counts — REQ-DL-132), reports `migrated`/`skipped`/`degraded`/`failed` (REQ-DL-133), backs up exactly once per content change, and creates an empty v2 registry when the file is missing (§26 step 2).
7. **Mandatory real-root ingest leg (attempt-6 repair, REQ-DL-003).** New `real_root_ingest.rs` beside `real_root_migration.rs`, `pub(super)`, `#[ignore]`-gated, wrapper in `tests.rs`: ingests a small verbatim multi-row OHLCV fixture (≥ 5 rows, valid OHLC, volumes present, ascending RFC3339 UTC `ts`) through the real store write path against the project data root; asserts the full per-version artifact set on disk, `ohlcv.jsonl` round-trip, the v2 registry entry with agreeing checksums, and idempotent re-ingest of identical bytes (no duplicate registry entries, no artifact deletion); prints the recorded `real-root ingest report: …` line. The fixture lives inside the test (no repo-root scratch files). This test is a mandatory completion step exactly like the migration run: excluded from routine/CI loops only because it mutates the real data root, never optional for completion.

No MCP tool is exercised by this task: it is pure local schema/storage/migration/ingest work with no provider, network, or credential involvement. The `tradingview` server in the project `.mcp.json` (`mcp__tradingview__tv_health_check`, `mcp__tradingview__data_get_ohlcv`, …) is exercised by TASK-DL-003/004. `required_env_keys` is `[]` and must stay that way — the no-credential path is a permitted, preserved behavior (REQ-DL-082, NFR-006).

## Files Expected to Change

- `/Volumes/Externalwork/archon-cli/archon-cli/.archon/trading-lab/data/registry.json` — absent (runtime artifact, gitignored; materialized by the mandatory real-root runs)
- `/Volumes/Externalwork/archon-cli/archon-cli/.archon/trading-lab/data` — absent (runtime artifact directory; materialized by the mandatory real-root runs)
- `/Volumes/Externalwork/archon-cli/archon-cli/.archon/trading-lab/data/datasets` — absent (runtime artifact directory; materialized by the mandatory real-root ingest)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake.rs` — exists (382 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/registry.rs` — exists (339 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/migration.rs` — exists (263 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/identity.rs` — exists (87 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/metadata_gate.rs` — exists (133 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/model.rs` — exists (310 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/real_root_migration.rs` — exists (49 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/real_root_ingest.rs` — absent (new `pub(super)` real-root ingest test module, attempt-6 repair)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/tests.rs` — exists (466 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/tests/artifact_tolerance.rs` — exists (161 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/src/data_lake/tests/provider_capability.rs` — exists (197 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/migration_v1_to_v2.rs` — exists (249 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/registry_schema_v1.rs` — exists (223 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/registry_v2.rs` — exists (162 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/dataset_naming.rs` — exists (109 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/registry_migration_v1.rs` — exists (183 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/registry_read_compat.rs` — exists (43 lines)
- `/Volumes/Externalwork/archon-cli/archon-cli/crates/archon-trading/tests/dataset_identity_metadata_v1.rs` — exists (167 lines)

The three `.archon/...` paths are runtime artifacts produced under the data root by `write_registry_v2` / `migrate_registry_v2` / the store ingest path; they are gitignored and absent from the tracked repository at the base commit (the `.archon` subtree is host-excluded for inspection) — the mandatory real-root runs are what materialize `registry.json` and the per-version dataset trees in place; the ingest run is the persistence act the REQ-DL-003 / G-DL-003 obligation rests on. The eight integration test files on this list — `migration_v1_to_v2.rs`, `registry_v2.rs`, `registry_schema_v1.rs`, `dataset_naming.rs`, `dataset_identity_metadata_v1.rs`, `registry_migration_v1.rs`, `registry_read_compat.rs` — are the files this task's focused `cargo nextest … --test <name>` commands run; each is observed under the repository root with the line count shown and is declared here so this task owns the suites it runs and a regression in any of them routes to this task. `registry_v2.rs` and `dataset_naming.rs` open `//! TASK-DL-002 focused tests …` doc comments dating from the earlier decomposition; per the TASK-DL-002 convention, doc-comment labels are not re-owned by editing — the frozen chain under this PRD assigns these contracts to TASK-DL-001, and this task's focused runs are what keep them green. All source paths above are observed under the repository root with the line counts shown; re-read each file before editing and keep every count below the cap in the Line-Count Check.

## Files Forbidden to Change

- `crates/archon-trading/src/data_store.rs` — TASK-DL-002 `validation-report-source` owner; TASK-DL-001 verifies REQ-DL-002/003/005 and AC-DL-003 through tests only.
- `crates/archon-trading/tests/validation_rules.rs`, `crates/archon-trading/tests/validation_report.rs` — TASK-DL-002.
- `.archon/trading-lab/data/provider-capabilities.json` and `.archon/trading-lab/data/snapshots/` — TASK-DL-003 runtime artifacts (`provider-capability-artifact`, `provider-snapshot-store`).
- `crates/archon-trading/src/data_lake/provider_capability.rs`, `crates/archon-trading/src/data_lake/providers/`, `crates/archon-trading/src/data_lake/stooq.rs`, `crates/archon-trading/src/data_lake/tradingview_mcp.rs` — TASK-DL-003…007 adapter/capability surface.
- `crates/archon-trading/src/coverage.rs` — TASK-DL-008 `coverage-matrix-source`.
- `src/command/trading_data.rs` and `crates/archon-trading/tests/data_commands_surface.rs` — TASK-DL-009 command dispatch; TASK-DL-001 adds no CLI/TUI surface.
- `crates/archon-trading/tests/backtest_data_gates.rs`, `backtest_gate_refusals.rs`, `backtest_diagnostic_override.rs`, `backtest_report_replay.rs`, `ahdm_backtest.rs` — TASK-BT-001.
- `crates/archon-trading/src/data_store/ahdm.rs` — TASK-AHDM-001.
- `crates/archon-trading/Cargo.toml` — no dependency changes; existing deps suffice.

## Acceptance Criteria

- **DONE-1 / G-DL-001:** the existing data root is upgraded in place **by an executed run of the mandatory real-root migration command in Focused Tests** (`cargo test -p archon-trading --lib data_lake::tests::migrate_real_project_root -- --ignored`), which must pass and must leave `.archon/trading-lab/data/registry.json` on disk carrying the v2 `schema_version` plus the `datasets`, `snapshots`, and `last_updated` keys — the test asserts exactly these post-conditions, so its pass is the in-place evidence. The printed `real-root migration report: …` line must be recorded in the completion evidence. A pre-existing v1 registry there is preserved record-for-record, a missing one becomes an empty v2 registry, and no raw or normalized artifact is deleted (REQ-DL-130); `changed=false` is the legal idempotent no-op (REQ-DL-132). **If the command has not been run, DONE-1 and G-DL-001 are unmet and the task is not complete.**
- **REQ-DL-003 / G-DL-003 (attempt-6 repair):** raw provider artifacts and normalized OHLCV datasets are **persisted in the project store with replayable version metadata**, proven by the **mandatory real-root ingest run** (`cargo test -p archon-trading --lib data_lake::tests::ingest_real_project_root_fixture -- --ignored`), which must pass and must leave, under `.archon/trading-lab/data/datasets/<dataset-id>/<version>/`, the full artifact set (`metadata.json`, `validation.json`, `manifest.json`, non-empty ascending `ohlcv.jsonl` round-tripping to the ingested rows, and `raw/` with verbatim `response.csv`, `request.json`, `headers.redacted.json`, `provider-notes.md`), plus a v2 registry entry `datasets[dataset_id][version]` whose checksum agrees with the metadata `normalized_sha256` (REQ-DL-004). The re-ingest leg of the same test proves REQ-DL-073 reuse and REQ-DL-130 non-destruction on identical bytes. The printed `real-root ingest report: dataset=… version=… rows=… status=…` line must be recorded in the completion evidence. The frozen `registry_minimum_count: 0` and `series_overlap_min_rows: 0` contract values are preserved exactly — after this run the registry is populated, so the finding's "registry can legitimately be empty" loophole is closed by execution, not by editing the contract. A completion claim whose only persistence evidence is a tempfile fixture, or whose real-root run output is unrecorded, is false (fail loud).
- **DONE-2:** migration is implemented and proven: every v1 record preserved verbatim under `datasets[dataset_id][version]` (REQ-DL-004), records missing either flag counted `degraded` and defaulted `false` (REQ-DL-131), one backup per content change whose report path names the file actually on disk, `migrated`/`skipped`/`degraded`/`failed` reported (REQ-DL-133), second run byte-identical (REQ-DL-132), unknown schema aborting before any write.
- **DONE-3:** dataset metadata carries the native/provenance fields (REQ-DL-010…013, REQ-DL-080…085): fail-closed flag defaults, root-relative paths, no secrets, deterministic JSON, in-memory v1 upgrade that leaves disk bytes identical — and the asset-class disclosures are **required content, not a bare tag**: a futures dataset without its populated §19 `continuous_contract` record, an equity-ETF dataset without `adjustment` plus `corporate_action_notes`, or a crypto dataset without `24x7` session and `exchange_source` is rejected by `validate_metadata` with the dataset id and the missing disclosure named. An empty or absent record fails; only populated disclosures pass.
- **REQ-DL-011 / REQ-DL-012 / REQ-DL-013:** every stored futures metadata records continuous-contract notes, a rollover policy (or explicit unknown-policy record) and provider symbol semantics; every equity-ETF metadata records the adjustment policy and an explicit corporate-action limitation; every crypto metadata records the 24/7 session assumption and the exchange source — enforced fail-closed by the provenance gate on the shared `validate_metadata` path and proven by focused tests that fail on the bare record.
- **AC-DL-003:** the ingest path stores raw artifact, normalized JSONL, metadata, validation report, and registry entry for a dataset version, and the manifest byte-checks against the in-memory record — proven on fixtures **and** on the real root by the mandatory ingest run.
- **REQ-DL-070…071 (complete rule):** ids carry no whitespace, decompose into exactly four non-empty alphanumeric components, agree with stored metadata identity (or extend it only by a provenance suffix), match the provider prefix for known providers, and every rejection names the exact offending id and violated rule.
- **REQ-DL-072…074:** versions change when raw content changes; identical raw content reuses the version only on an exact checksum match; ids are stable across machines and sessions.
- **DONE-12:** every routine focused test listed below passes, both mandatory real-root runs (migration, then ingest) have been executed and passed with their reports recorded, and `scripts/ci-gate.sh --skip-bench` exits 0 before any PR or push (ARCHON.md gates).

## Focused Tests

Run from the repository root. NFR-004: focused tests only — do not run the full workspace suite. The two deliberate exceptions are the mandatory real-root runs in the second list; they do not make the routine suite heavy and are required for completion, in the order given (migration before ingest).

Routine focused tests (tempfile fixtures; must pass in every run and in CI). Every `--test <name>` target maps 1:1 to a `crates/archon-trading/tests/<name>.rs` file declared under Files Expected to Change:

- `cargo nextest run -p archon-trading --test migration_v1_to_v2` — REQ-DL-130…133: record preservation, fail-closed defaults, one-backup-per-change, on-disk backup path, idempotent second run, missing-file → empty v2, unknown-schema abort, exact degraded counts, inert repo-root report.
- `cargo nextest run -p archon-trading --test registry_v2` — REQ-DL-004/REQ-DL-085: v2 round-trip with `schema_version`, v1 `schema`-keyed loads, content addressing, unknown-schema failure.
- `cargo nextest run -p archon-trading --test registry_schema_v1` — AC-DL-003: store ingest writes the full artifact set (raw, `ohlcv.jsonl`, `metadata.json`, `manifest.json`, `validation.json`) plus the registry entry; extended by this task to assert end-to-end that the persisted `metadata.json` carries the crypto `24x7` session and the `exchange_source` record through the real store write path.
- `cargo nextest run -p archon-trading --test dataset_naming` — REQ-DL-070…071 (complete rule) plus REQ-DL-072…074 store-level legs: exact-offending-id rejections, four-component lossless decomposition, metadata-agreement encoding, checksum-bound version reuse.
- `cargo nextest run -p archon-trading --test dataset_identity_metadata_v1` — AC-DL-003 / REQ-DL-010…013: v1 metadata compatibility, raw-checksum-bound identity/version, secret-free metadata through the store.
- `cargo nextest run -p archon-trading --test registry_migration_v1` — REQ-DL-130…133 against a v1-shaped registry and manifests, including the manifest byte-check and artifact-inventory retention.
- `cargo nextest run -p archon-trading --test registry_read_compat` — flat-keyed v2 loads and ingest preparation that never downgrades or rewrites the registry.
- `cargo nextest run -p archon-trading --lib data_lake` — unit contracts: metadata field gates, REQ-DL-020 production/native consistency, REQ-DL-071's full encoding leg pinned by `dataset_id_must_encode_provider_instrument_timeframe_and_price_basis` (`tests.rs:244–265`), REQ-DL-083/084/131 defaults, REQ-DL-085 byte-identical read upgrade, REQ-DL-082 header redaction, id/version syntax, provider-identity and yfinance-degraded rules, registry status/promotion behavior — **plus the asset-class provenance gate, which must exist and must fail on the bare tag**: `req_dl_011_futures_metadata_without_continuous_contract_disclosure_is_rejected` (record absent, or any of `notes`/`roll_policy`/`provider_symbol_semantics` empty, rejected with dataset id and missing item named; `req_dl_011_explicit_unknown_roll_policy_is_accepted` pins the honest unknown-policy path), `req_dl_012_equity_etf_metadata_without_corporate_action_limitation_is_rejected`, and `req_dl_013_crypto_metadata_without_24x7_session_and_exchange_source_is_rejected`; companion positive tests prove a populated record passes, serializes the exact §19 keys, and that unset provenance serializes nothing (byte-identical writers) — **and the attempt-6 REQ-DL-003 unit pins**: a store-path ingest of a ≥ 5-row fixture into a temp data root leaves a non-empty ascending `ohlcv.jsonl` that round-trips, plus the registry entry/checksum agreement, so replayable normalized storage is asserted, not merely permitted; if these push `tests.rs` past 500 lines they move into a new child test module beside `tests/artifact_tolerance.rs` / `tests/provider_capability.rs` (`mod`-declared at `tests.rs:8–9`), the same in-crate child pattern, not a new `tests/` integration file.

### Mandatory real-root completion runs (DONE-1 / G-DL-001 / REQ-DL-003 / G-DL-003 evidence)

Both are excluded from routine and CI loops because they mutate the real data root — but **required exactly once per completion of this task, unconditionally, in this order**. Together they are the only thing that makes DONE-1, G-DL-001, REQ-DL-003 and G-DL-003 true; the routine tests prove the contracts on fixtures and cannot substitute. Do not report the task complete without both recorded outputs.

- `cargo test -p archon-trading --lib data_lake::tests::migrate_real_project_root -- --ignored` — **mandatory, first**: executes the real-root migration against `<repo>/.archon/trading-lab/data/registry.json` (§26 steps 1–6: backup first, never delete artifacts, print counts). Passing proves the root is upgraded in place (v2 `schema_version`, `datasets`/`snapshots`/`last_updated` asserted by the test, `real_root_migration.rs:38–48`). Record the printed `real-root migration report: migrated=… skipped=… degraded=… failed=… backup=… changed=…` line; `changed=false` is an acceptable pass (idempotent no-op, REQ-DL-132). If it fails, repair at the source and re-run until it passes; a completion claim without the recorded passing output is false (fail loud).
- `cargo test -p archon-trading --lib data_lake::tests::ingest_real_project_root_fixture -- --ignored` — **mandatory, second (attempt-6 REQ-DL-003 repair)**: executes the real-root ingest of the built-in ≥ 5-row verbatim OHLCV fixture through the real store write path against `<repo>/.archon/trading-lab/data`, then re-ingests the identical bytes. Passing proves on disk: the per-version dataset tree with `metadata.json`/`validation.json`/`manifest.json`/`ohlcv.jsonl`/`raw/{response.csv,request.json,headers.redacted.json,provider-notes.md}`; `ohlcv.jsonl` non-empty, ascending, round-tripping (REQ-DL-003 replayable storage — the act the finding demanded); the v2 registry entry with checksum agreement (REQ-DL-004); and idempotent re-ingest with no duplicated entries and no deleted artifacts (REQ-DL-073/REQ-DL-130). Record the printed `real-root ingest report: dataset=… version=… rows=… status=…` line in the completion evidence. This command has no skip, no conditional, and no defer path: if it fails, the failure is repaired at its source and the command re-run; if its output is not captured verbatim, the run is repeated. A completion claim without the recorded passing output is false (fail loud). After this run the project registry is populated — the empty-registry state the finding quoted is closed by execution, and `registry_minimum_count: 0` / `series_overlap_min_rows: 0` remain the frozen contract values, untouched.

## Line-Count Check

- NFR-001: every changed or new file stays under 500 lines. Current counts: `data_lake.rs` 382, `registry.rs` 339, `model.rs` 310, `tests.rs` 466, `migration.rs` 263, `metadata_gate.rs` 133, `identity.rs` 87, `real_root_migration.rs` 49, `migration_v1_to_v2.rs` 249, `registry_schema_v1.rs` 223, `registry_v2.rs` 162, `dataset_naming.rs` 109, `registry_migration_v1.rs` 183, `registry_read_compat.rs` 43, `dataset_identity_metadata_v1.rs` 167 — all compliant.
- `tests.rs` (466) is nearest the cap: the provenance-gate and REQ-DL-003 unit pins go to a child test module rather than growing the parent (see Focused Tests). New `real_root_ingest.rs` is a small focused module (fixture + assertions + report line), comparable to `real_root_migration.rs` (49 lines) and well under the cap; it is `mod`-declared beside `real_root_migration` at `data_lake.rs:376–377` with the wrapper added in `tests.rs` beside `tests.rs:462–466` — budget roughly 15 lines there, keeping `tests.rs` under the cap even before the child-module relocation rule. `model.rs` nets down — the removed unwired variant structs outweigh the new §19 records. `data_lake.rs` gains one gate call plus one error variant and stays well under the cap. `scripts/ci-gate.sh` FileSizeGuard is the enforcement point.

## Complexity Check

- NFR-002: cyclomatic complexity ≤ 15 for changed/new functions; ARCHON.md's general rule is stricter (≤ 10), so keep new functions ≤ 10 and never above the PRD's 15. The asset-class provenance gate is a dedicated flat guard-clause function invoked from `validate_metadata` — not inlined branching — so the shared validator's complexity does not grow. The real-root ingest test is linear arrange/act/assert with at most two legs (ingest, re-ingest); keep its helpers small and flat.

## Adversarial Review Notes

- **Attempt-6 repair — REQ-DL-003 is discharged by the ingest run, not by a permission.** The quoted loophole ("an empty-but-v2 registry is a legal persisted state and the idempotent no-op REQ-DL-132 path") is closed structurally: the mandatory real-root ingest run populates the store with a replayable, checksum-bound dataset version and is a completion obligation exactly like the migration run — recorded output or the completion claim is false (fail loud). The frozen contract constants are preserved; execution, not contract-editing, is the repair.
- **Attempt-2 repair 1 — G-DL-003 persistence via the run, retained.** `registry_minimum_count: 0` and `series_overlap_min_rows: 0` stay frozen; persistence is proven by the executed runs and their recorded reports, with the no-op path leaving persisted artifacts intact.
- **Attempt-2 repair 2 — REQ-DL-071 pinned as the full four-component encoding rule, retained.** The lib pin `dataset_id_must_encode_provider_instrument_timeframe_and_price_basis` and the `dataset_naming.rs` suite are the regression guard; the acceptance sweep (`SUP-REQ-DL-071`) verifies every stored id against its stored metadata through the delivered lane.
- **Provenance is content, not a tag (carried repair).** A bare futures/ETF/crypto dataset — record absent, or present with empty fields — must fail `validate_metadata` with the dataset id and the exact missing item. The three named focused tests exist precisely so a green suite with bare metadata is impossible; a test that only asserts "something returned" is decoration (Rule 9).
- **Lane compatibility is proven, not assumed.** The gate binds every `validate_metadata` call site (registration, store write `data_store.rs:396`, store validate `data_store.rs:332`, `gates.rs:26`), so any lane whose fixtures record an affected asset class gains the owed disclosure in the same change; the exported store-lane crypto constants (`YFINANCE_CRYPTO_SESSION`/`YFINANCE_CRYPTO_EXCHANGE_SOURCE`) stay the single source of truth for the 24/7 and exchange-source wordings, and the twin-lane crypto and equity artifact tests must stay green. Unset provenance serializes nothing, so writers that record none emit byte-identical metadata.
- **Ordering of the mandatory runs.** Migration before ingest: the ingest asserts a v2 registry; running ingest first on a v1/absent registry would fail spuriously. The completion evidence records both report lines in that order.
- **Idempotency trap (both runs).** A second migration on v2 must be a byte-identical no-op with zero counts and no new backup (REQ-DL-132); a re-ingest of identical bytes must reuse the version on exact checksum match (REQ-DL-073) and never duplicate registry entries or delete artifacts (REQ-DL-130). Any "convenient" rewrite breaks these; the real-root runs are repeatable and must stay so.
- **Backup integrity.** `report.backup_path` must name the file that exists on disk (sanitized timestamp), not a re-stamped name — regression `contested-6cee5ea3` is pinned by `repo_root_report_is_inert_to_migration_contract`, which also proves the repo-root `registry-migration-report.json` placeholder is never read or modified by migration.
- **Non-destructiveness.** Migration touches only `registry.json`, its backup, and the report beside it (REQ-DL-130); raw and normalized artifacts are never deleted — by migration **or** by the re-ingest leg. Atomic temp+rename plus manifest byte-checks keep REQ-DL-005 true: an interrupted ingest cannot leave a healthy registry entry pointing at missing artifacts.
- **Secrets (REQ-DL-082 / NFR-006).** `redact_secret_headers` strips `Authorization`/`X-API-Key`/`Set-Cookie`/session-shaped headers; no credentials, tokens, or cookies may reach `metadata.json`, manifests, validation reports, or raw headers — including the real-root ingest fixture's `headers.redacted.json`. `required_env_keys: []` is a contract, not an oversight — do not introduce env-key reads into this task's code paths.
- **Identity attacks.** A dataset whose id prefix contradicts its provider must fail (`provider_identity_cannot_be_mislabelled_for_known_providers`); yfinance can never be production-eligible; production requires native + passed (REQ-DL-020 gate fires with the dataset id on every construction path).
- **Reads are pure.** Loading or preparing a registry must leave on-disk bytes identical (REQ-DL-085) — verified by the byte-identical read-upgrade and ingest-preparation pins; the real-root ingest writes only through the store path, never by rewriting registry bytes ad hoc.
- **Fixture hygiene.** The real-root ingest fixture is embedded in the test module; no scratch files are written to the repository root (ARCHON.md file-organisation rules), and the ingest target is the data root, not the repo root.
- **Residual gaps.** None may be deferred silently (PRD §30/§32): any gap discovered during implementation gets a §32 record with an explicit `fail_closed_behavior` — "should work", "later", or "best effort" without fail-closed behavior are forbidden phrasings.
- **Real-root runs are completion obligations, not decorations.** Both `#[ignore]` markers exist solely to keep mutating runs out of routine/CI loops; neither is ever optional for this task: execute migration, then ingest, record both printed reports (`changed=`/`backup_path`; `dataset=`/`version=`/`rows=`/`status=`) as the DONE-1/G-DL-001/REQ-DL-003/G-DL-003 evidence, and treat a completion claim without those recorded runs as false (fail loud). There is no path through this task's completion that leaves either run unexercised.
```