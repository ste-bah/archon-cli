```yaml
task_id: TASK-DL-002
title: Registry v2 schema, atomic dataset writes, v1→v2 migration, and ingest artifact contract
complexity: high
status: ready
depends_on: [{"task_id": "TASK-DL-001", "consumes": [{"artifact_path": "docs/trading-data-lake-gap-audit.md"}], "ordering_only": false}]
blocks: ["TASK-DL-003", "TASK-DL-004", "TASK-DL-010"]
implements: ["REQ-DL-001", "REQ-DL-002", "REQ-DL-003", "REQ-DL-004", "REQ-DL-005", "REQ-DL-010", "REQ-DL-011", "REQ-DL-012", "REQ-DL-013", "REQ-DL-070", "REQ-DL-071", "REQ-DL-072", "REQ-DL-073", "REQ-DL-074", "REQ-DL-080", "REQ-DL-081", "REQ-DL-082", "REQ-DL-083", "REQ-DL-084", "REQ-DL-085", "REQ-DL-130", "REQ-DL-131", "REQ-DL-132", "REQ-DL-133", "AC-DL-003"]
required_env_keys: []
required_tools: []
deliverable_contracts: [{"kind": "registry-schema-v2", "artifact_path": "crates/archon-trading/src/data_lake.rs", "min_instances": 1, "required_universe": false, "registry_minimum_count": 0, "series_overlap_min_rows": 0}]
```

# TASK-DL-002 — Registry v2 Storage, Metadata Schema, Atomic Writes, v1→v2 Migration, and Ingest Artifact Contract (Milestone 1, Phases 1–2 storage lane)

## Objective

Upgrade the existing project-local Trading Lab store to the PRD
`PRD-TRADING-DATA-LAKE-AHDM-001` §17/§18/§19 storage contract, exactly as
frozen in `implements`:

- Use the existing root `.archon/trading-lab/data`; never create a second
  storage location (REQ-DL-001).
- Store raw provider responses under each dataset version per the §17 `raw/`
  layout (REQ-DL-002).
- Store normalized OHLCV as replayable JSONL, one candle per line (REQ-DL-003).
- Content-address registry entries by dataset id and version (REQ-DL-004).
- Make dataset writes atomic: an interrupted ingest never leaves a healthy
  registry entry pointing at missing artifacts (REQ-DL-005).
- Emit the §19 metadata schema, including the full REQ-DL-010 field list and
  the asset-class payloads: futures continuous-contract notes (REQ-DL-011),
  equity-ETF adjusted/unadjusted policy (REQ-DL-012), crypto 24/7 session and
  exchange source (REQ-DL-013).
- Enforce the §18 dataset-id/version naming contract (REQ-DL-070…074).
- Deterministic metadata JSON, project-root-relative paths, no secrets, and
  the missing-field fail-closed defaults (REQ-DL-080…084); v1 metadata is
  upgraded in memory on read and rewritten only on explicit migration or
  dataset update (REQ-DL-085).
- Implement the PRD §26 migration: non-destructive (REQ-DL-130), fail-closed
  on unknown native status (REQ-DL-131), idempotent (REQ-DL-132), and
  count-reporting (REQ-DL-133).
- Make the `trading data ingest-ohlcv` path satisfy AC-DL-003 end to end:
  ingesting a valid CSV stores the raw artifact, normalized `ohlcv.jsonl`,
  `metadata.json`, a `validation.json` with `status=passed` and a correct
  `summary.row_count`, and a registry entry for the exact dataset id/version.

This task is the storage lane only. The deep validation engine (monotonicity,
gap, OHLC-sanity check catalog — REQ-DL-050…056, REQ-DL-090…098, REQ-DL-100…103)
is TASK-DL-003; the provider capability interface is TASK-DL-004; adapters,
coverage/CLI surface expansion, and backtest gates are TASK-DL-005…010. The
only validation behavior in scope here is the minimal write-path emission of
`validation.json` (baseline well-formed input → `passed` with `row_count`) so
AC-DL-003 holds; TASK-DL-003 owns the check catalog and must hook through a
seam this task exposes, not duplicate it.

## Consumed Input (TASK-DL-001 edge)

`depends_on` freezes consumption of `docs/trading-data-lake-gap-audit.md` (the
fresh Phase 0 gap report). Its per-anchor verdicts govern extend-vs-create and
are authoritative where they disagree with anything below. Known prior
evidence to reconcile, never copy: the advisory audit
`docs/trading/data-lake-gap-audit.md` (checkout
`/Volumes/Externalwork/archon-cli/archon-cli`, commit `68ccb344`, dirty tree)
found `crates/archon-trading/src/data_lake.rs` (~321 lines, 12 submodules
under `data_lake/`), `data_store.rs` (registry path at lines 71–76), and
`src/command/trading_data.rs` (14 dispatch arms) present, with the live
project-1 registry at schema `archon-trading-data-registry-v1` under the key
**`schema`** (not `schema_version`), 0 datasets, 0 snapshots, ~58
`registry.json.backup-*` files, a `registry-migration-report.json`, a
`provider-capabilities.json` flat file **and** a `provider-capabilities/`
directory, and fixture dataset dirs (e.g. `manual-BTCUSD-1D-raw/20260101-fixture`)
lacking `metadata.json`. At authoring time the project-1 checkout did not
expose `crates/` or `src/command/trading_data.rs`; if the fresh gap report
confirms absence, this task **creates** the frozen deliverable path and the
minimum crate scaffolding needed to build it, and records the extend-vs-create
decision in the run evidence. Neither audit's findings may be attributed to the
other root.

## Scope

1. **Registry v2 model (`crates/archon-trading/src/data_lake.rs` + new
   submodules under `crates/archon-trading/src/data_lake/`).**
   - Load/write `registry.json` with `"schema_version":
     "archon-trading-data-registry-v2"`, `datasets`, `snapshots`, RFC3339
     `last_updated` (§17). The reader must accept both the v2 key and the v1
     `schema` key (serde alias) with every record preserved (REQ-DL-085); the
     writer always emits `schema_version`.
   - Entries content-addressed `datasets[dataset_id][version]` (REQ-DL-004).
   - Dataset ids validated against §18
     `<provider>-<canonical-instrument>-<timeframe>-<price-basis>`: no spaces
     (REQ-DL-070), all four components encoded and non-empty (REQ-DL-071),
     stable across machines/sessions (REQ-DL-074); rejection errors carry the
     exact offending id. Versions change when raw provider content changes
     (REQ-DL-072); identical raw content may reuse a version only on exact
     checksum match (REQ-DL-073) — expose the checksum hook now; fetch
     adapters arrive in TASK-DL-005…008.
2. **Metadata schema (§19; REQ-DL-010…013, 080…084).** Serialize the full
   minimum shape: identity, provenance (`provider`, `provider_symbol`),
   `native_interval` / `production_eligible` (absent field reads as `false`,
   REQ-DL-083/084), price basis, timezone, session, adjustment, asset-class
   payloads (`continuous_contract` for futures, adjustment/corporate-action
   notes for equity ETFs, 24/7 session + exchange for crypto), `coverage`
   block, `checksums`, project-root-relative `paths` (REQ-DL-081), redacted
   `source` block, `quality_status`, `created_at`. JSON is deterministic
   (stable field order; byte-identical across identical writes, REQ-DL-080).
   No secret, API key, token, cookie, or session id may reach metadata, raw
   headers, validation reports, or manifests (REQ-DL-082).
3. **Atomic write discipline (REQ-DL-005).** Write version artifacts
   (`metadata.json`, `validation.json`, `ohlcv.jsonl`, `raw/` incl.
   `request.json` and redacted `headers.redacted.json` per §17) first; publish
   the registry entry last; temp-file + rename within the same filesystem. A
   mid-write failure leaves `registry.json` byte-identical.
4. **Minimal validation write seam.** The ingest path emits `validation.json`
   (`schema_version: archon-trading-validation-v1`, `status: passed`,
   `summary.row_count`) for well-formed input and carries the hook point where
   TASK-DL-003's check catalog will attach. No REQ-DL-050…056/090…098 check
   logic is implemented here.
5. **v1→v2 migration (§26; new `data_lake/migration.rs`).** Missing registry →
   create v2 empty; v1 → preserve all records, infer missing fields
   conservatively, unknown `native_interval`/`production_eligible` → `false`
   (fail closed, REQ-DL-131), `quality_status=degraded` when no validation
   report exists, write `validation.json` with migration warnings; never
   delete raw/normalized files, backups, coverage, snapshots, or capability
   artifacts (REQ-DL-130); idempotent second run is a byte-identical no-op
   (REQ-DL-132); write `registry.json.backup-<timestamp>` only on content
   change and report `migrated/skipped/degraded/failed` counts to
   `registry-migration-report.json` (REQ-DL-133). Existing residue (~58–90
   backups, divergent capability artifacts, orphan dataset dirs) is left
   untouched.
6. **Ingest command conformance (AC-DL-003).** `trading data ingest-ohlcv
   --source … --format csv|json --dataset-id … --version … --provider …
   --symbol … [--target …]` writes the §17 artifact set and the registry
   entry, prints dataset id/version/validation status/artifact paths, and
   validates against the metadata/naming rules of this task. Command-surface
   expansion beyond this belongs to TASK-DL-009.

## MCP Tool Determination

`.mcp.json` at the project root declares one server, `tradingview` (local
stdio server over CDP), with tools `tv_health_check`, `tv_discover`,
`tv_ui_state`, `chart_get_state`, `quote_get`, `data_get_ohlcv`, `pine_*`,
`alert_*`, `replay_trade`, `tv_launch`. **No MCP invocation is obligated by
this task**, therefore `required_tools: []`. This is offline filesystem
engineering: registry schema, naming, atomic writes, metadata serialization,
and migration touch no provider. TradingView MCP contact (health/discover/
`data_get_ohlcv`) is TASK-DL-004/TASK-DL-005 scope under REQ-DL-030…033 and
REQ-DL-110…113; declaring an MCP call here would force browser/CDP/chart-session
preconditions into storage work. `required_env_keys: []` likewise: focused
tests run against temp fixtures and a temp `--target` root, migration of the
live root needs no credential, and REQ-DL-082 guarantees no secret is written
into any artifact. The deliverable is a Rust source contract, not an MCP
deliverable, so a no-MCP declaration is valid.

## Files Expected to Change

- `crates/archon-trading/src/data_lake.rs` (edit if present per the fresh gap
  report, else create — the frozen `registry-schema-v2` contract path;
  add/hold public v2 + migration entry points).
- `crates/archon-trading/src/data_lake/registry.rs`,
  `crates/archon-trading/src/data_lake/migration.rs` (create — implementation
  submodules so the contract file stays under 500 lines), plus the minimum
  `mod` wiring.
- Minimum crate scaffolding (manifest, `lib.rs`) only if the fresh gap report
  confirms the crate is absent from this checkout.
- `src/command/trading_data.rs` (edit only the `ingest-ohlcv` arm to the §17
  artifact contract; no other command surfaces).
- `crates/archon-trading/tests/registry_v2.rs`,
  `crates/archon-trading/tests/dataset_naming.rs`,
  `crates/archon-trading/tests/migration_v1_to_v2.rs`,
  `crates/archon-trading/tests/ingest_artifacts.rs` (create).
- `.archon/trading-lab/data/registry.json` and
  `.archon/trading-lab/data/registry-migration-report.json` — mutated **only**
  by the migration/ingest code paths during focused runs; never hand-edited.

## Files Forbidden to Change

- Frozen chain: `tasks/PRD-TRADING-DATA-LAKE-AHDM-001/*` (skeleton, contract,
  locks, `TASK-DL-001.md`) and `prds/PRD-TRADING-DATA-LAKE-AHDM-001.md`.
- `crates/archon-trading/src/validation.rs` / deep-validation modules,
  provider/capability/coverage/backtest-gate modules — owned by TASK-DL-003,
  -004, -009, -010 (capabilities/coverage/commands) and TASK-DL-005…008
  (adapters).
- Every file under `.archon/trading-lab/data/` except the two registry
  artifacts named above — including all `registry.json.backup-*`,
  `coverage/`, `snapshots/`, `provider-capabilities*`, and existing dataset
  dirs (REQ-DL-130: delete nothing).
- Anything under `.archon/trading-lab/strategies/` and all Trading Lab code
  unrelated to registry storage (NG-DL-006).
- The advisory audit `docs/trading/data-lake-gap-audit.md` (read-only
  reference at a different path from TASK-DL-001's deliverable).

## Acceptance Criteria

- AC-1: `crates/archon-trading/src/data_lake.rs` exists, is non-empty, under
  500 lines, and exposes public registry-v2 load/write and migration entry
  points (contract `registry-schema-v2`, `min_instances=1`; path frozen).
- AC-2: A v2-written `registry.json` carries
  `"schema_version": "archon-trading-data-registry-v2"` and round-trips; a v1
  file keyed by `schema` with non-empty datasets loads with all records intact
  and unmodified in count (REQ-DL-085).
- AC-3: Migration on a v1 fixture with ≥1 dataset preserves every record,
  defaults unknown `native_interval`/`production_eligible` to `false`, writes
  one backup per content change, reports the four REQ-DL-133 counts, and a
  second run is byte-identical (REQ-DL-130…132).
- AC-4: An ingest write on a temp root produces `metadata.json`,
  `validation.json` (`status=passed`, correct `row_count`), `ohlcv.jsonl`
  (ascending, one candle per line, required fields), a non-empty raw artifact
  reachable from `metadata.paths`, and the registry entry for the exact
  dataset id/version; a simulated pre-publish failure leaves `registry.json`
  byte-identical (REQ-DL-002/003/005, AC-DL-003).
- AC-5: Dataset ids with a space, wrong component count, or empty components
  are rejected with the exact offending id; version reuse requires an exact
  checksum match (REQ-DL-070…074).
- AC-6: Metadata JSON is byte-identical across two identical writes, stores
  only project-root-relative paths, carries the REQ-DL-010 field set plus the
  asset-class payloads, and contains no secret-shaped strings (REQ-DL-010…013,
  080…082).
- AC-7: The live-root migration run leaves every pre-existing backup, the
  `provider-capabilities*` pair, `coverage/`, and existing dataset dirs
  present (file counts before/after match, REQ-DL-130).
- AC-8: Checkout diff shows only files under "Files Expected to Change" (plus
  the two registry artifacts from the code path).

## Focused Tests

Run from `/Volumes/Externalwork/archon-cli/project-1`. All must exit 0
(NFR-004: focused only; never the full heavy suite).

```bash
# 1. Focused unit/integration tests
#    (REQ-DL-002/003/004/005/010-013/070-074/080-085/130-133, AC-DL-003)
cargo nextest run -p archon-trading --test registry_v2
cargo nextest run -p archon-trading --test dataset_naming
cargo nextest run -p archon-trading --test migration_v1_to_v2
cargo nextest run -p archon-trading --test ingest_artifacts

# 2. Deliverable contract exists, non-empty, under the NFR-001 cap (AC-1)
test -s crates/archon-trading/src/data_lake.rs
awk 'END{exit (NR>500)}' crates/archon-trading/src/data_lake.rs

# 3. AC-DL-003 differential: valid CSV ingest stores the full §17 artifact set
#    (mirrors the frozen acceptance check; temp target, no live-root mutation)
B=./target/release/archon; T=$(mktemp -d)
cargo build --release --bin archon
printf 'ts,open,high,low,close,volume\n2024-01-01T00:00:00Z,100,101,99,100.5,1000\n2024-01-02T00:00:00Z,100.5,102,100,101.5,1200\n2024-01-03T00:00:00Z,101.5,103,101,102.5,1400\n' > "$T/bars.csv"
"$B" trading data ingest-ohlcv --source "$T/bars.csv" --format csv --dataset-id stooq-SPY-1D-raw --version 20240103-0000000f --provider stooq --symbol SPY --target "$T"
D="$T/.archon/trading-lab/data/datasets/stooq-SPY-1D-raw/20240103-0000000f"
test -s "$D/metadata.json" && test -s "$D/validation.json" && test -s "$D/ohlcv.jsonl"
jq -e '.status == "passed" and .summary.row_count == 3' "$D/validation.json"
jq -e '.dataset_id == "stooq-SPY-1D-raw" and .native_interval == true' "$D/metadata.json"
python3 -c "import json,sys; rows=[json.loads(l) for l in open(sys.argv[1]) if l.strip()]; assert rows and all(all(k in r for k in ('ts','open','high','low','close')) for r in rows); ts=[r['ts'] for r in rows]; assert ts==sorted(ts) and len(set(ts))==3" "$D/ohlcv.jsonl"
jq -e '.datasets["stooq-SPY-1D-raw"]' "$T/.archon/trading-lab/data/registry.json" >/dev/null

# 4. Naming rejections carry the offending id (AC-5, REQ-DL-070/071)
if "$B" trading data ingest-ohlcv --source "$T/bars.csv" --format csv --dataset-id "bad id" --version 20240103-x1 --provider stooq --symbol SPY --target "$T" 2>"$T/name.err"; then echo 'FAIL: spaced id accepted'; exit 1; fi
grep -q 'bad id' "$T/name.err"

# 5. Migration idempotency + non-destruction on the live root (AC-3/AC-7, REQ-DL-130/132)
B0=$(ls .archon/trading-lab/data/registry.json.backup-* 2>/dev/null | wc -l)
grep -q 'archon-trading-data-registry' .archon/trading-lab/data/registry.json
test -s .archon/trading-lab/data/registry-migration-report.json
grep -Eq 'migrated|skipped|degraded|failed' .archon/trading-lab/data/registry-migration-report.json
B1=$(ls .archon/trading-lab/data/registry.json.backup-* 2>/dev/null | wc -l)
test "$B1" -ge "$B0"
test -d .archon/trading-lab/data/coverage

# 6. NFR-002 lint gate on the touched crate
cargo clippy -p archon-trading -- -D warnings

# 7. File-size gate
bash scripts/check-file-sizes.sh
```

## Line-Count / Complexity Checks (NFR-001/002)

- `crates/archon-trading/src/data_lake.rs` hard cap 500 lines; registry and
  migration logic lives in new `data_lake/registry.rs` / `data_lake/migration.rs`
  submodules, each < 500 lines, functions < 50 lines, single responsibility.
- New functions: cyclomatic complexity ≤ 15 (NFR-002); clippy
  `cognitive_complexity` / `too_many_lines` / `too_many_arguments` clean at
  `-D warnings`.

## Adversarial Review Notes

- **Key-rename risk.** §17 mandates `schema_version`; the live v1 file uses
  `schema`. A reader that rejects the legacy key, or a writer that silently
  rewrites on read-only loads, breaks REQ-DL-085. Verify both spellings
  round-trip and v1 records survive load→write with unchanged count.
- **Fail-closed inversion risk.** Migration defaulting unknown
  `native_interval`/`production_eligible` to `true` inverts REQ-DL-131/083/084
  and poisons every downstream gate (TASK-DL-003 validation, TASK-DL-010
  backtest refusal). Reject on sight.
- **Atomicity theater risk.** "Atomic" that publishes the registry entry
  before artifacts, or renames across filesystems, fails REQ-DL-005. The test
  must simulate a mid-write failure and diff registry bytes.
- **Idempotency risk.** A migration that rewrites backups or bumps
  `last_updated` on a no-op second run fails REQ-DL-132; `cmp` first- vs
  second-run registry bytes.
- **Validation-scope creep risk.** Any REQ-DL-050…056/090…098 check logic in
  this diff belongs to TASK-DL-003; the write-path emission here must stay a
  minimal seam. Likewise any capability probe, adapter, or coverage code
  belongs to TASK-DL-004…009 (Rule 3: surgical changes).
- **Deletion risk.** The data root carries backup accumulation, divergent
  `provider-capabilities*` artifacts, and orphan dataset dirs. Any deletion
  here violates REQ-DL-130; cleanup is a separate approved change.
- **Secret-leak risk.** REQ-DL-082: reviewers must reject any diff writing
  credential-shaped values into metadata, raw headers, validation reports, or
  manifests; `headers.redacted.json` must actually redact.
- **Fresh-evidence risk.** Anchor verdicts above derive from the advisory
  audit of a sibling checkout; the frozen `depends_on` deliverable
  `docs/trading-data-lake-gap-audit.md` governs where they disagree, including
  the extend-vs-create decision for the contract file (Rule 7).
- **Second-storage-location risk.** REQ-DL-001/NG-DL-002: any new data path
  outside `.archon/trading-lab/data` (or reliance on loose CSVs outside the
  registry) is a rejection.
```