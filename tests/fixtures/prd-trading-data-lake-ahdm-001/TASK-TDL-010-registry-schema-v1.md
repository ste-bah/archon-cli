# TASK-TDL-010 — Registry Schema v2 + Migration

```yaml
task_id: TASK-TDL-010
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Registry Schema v2 + Migration
workstream: W1 Storage + Validation
complexity: large
status: blocked
depends_on: ['TASK-TDL-001']
blocks: ['TASK-TDL-020']
source_sections: ['8.1', '17', '18', '19', '26']
implements: [REQ-DL-001, REQ-DL-002, REQ-DL-003, REQ-DL-004, REQ-DL-005, REQ-DL-070, REQ-DL-071, REQ-DL-072, REQ-DL-073, REQ-DL-074, REQ-DL-080, REQ-DL-081, REQ-DL-082, REQ-DL-083, REQ-DL-084, REQ-DL-085, REQ-DL-130, REQ-DL-131, REQ-DL-132, REQ-DL-133]
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: trading_data_registry
    artifact_path: .archon/trading-lab/data/registry.json
  - kind: registry_migration_report
    artifact_path: .archon/trading-lab/data/registry-migration-report.json
```

## Purpose

Implement registry v2 and dataset metadata schema while preserving v1 readability and migration safety.

## Scope

### In

- Registry schema `archon-trading-data-registry-v2`.
- Dataset metadata minimum shape from PRD §19.
- Deterministic dataset id/version helpers.
- v1 registry read compatibility.
- Atomic migration with backup file.
- CLI/TUI coverage for `data status`, `data list [--json]`, `data show`, existing manual `data ingest-ohlcv` compatibility, and required PRD command `data export --dataset-id <ID> --version <VERSION> --out <PATH>`.
- Existing `export-ohlcv` must either remain as a backwards-compatible alias or be migrated with parse tests proving the PRD spelling `data export` works.
- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.
- `--target` support for all owned commands.
- Dataset artifact contract for every registered dataset version:
  - `metadata.json`
  - `validation.json`
  - `manifest.json`
  - `ohlcv.jsonl`
  - `raw/response.<json|csv|zip|txt>`
  - `raw/request.json`
  - `raw/headers.redacted.json`
  - `raw/provider-notes.md`

### Out

- Provider fetch adapters.
- Coverage matrix.
- AHDM strategy work.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-DL-001 partly satisfied: status can show root and registry.
- v1 registry remains readable.
- First write migrates to v2 preserving records.
- Unknown native/production fields fail closed.
- Migration is idempotent and never deletes artifacts.
- Registry entries must only point to complete dataset versions satisfying the artifact contract. Unavailable or incomplete provider results must be recorded as provider capability, coverage, or residual-gap records and must not create healthy dataset registry entries.
- `archon trading data list --json` is implemented and tested.
- `archon trading data export --dataset-id <ID> --version <VERSION> --out <PATH>` is implemented and tested; existing `export-ohlcv` cannot be the only supported spelling.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading registry_schema_migration` — registry schema migration.
- `cargo test -p archon-trading registry_atomic_write` — atomic write, including the interrupted-write case.
- `cargo test -p archon-trading dataset_id_version_validation` — dataset id/version validation.
- `cargo test -p archon-trading dataset_artifact_contract` — dataset artifact contract validation.
- `cargo test --bin archon trading_data_list_json_parse` — CLI parse for `data list --json`.
- `cargo test --bin archon trading_data_export_parse` — CLI parse for `data export --dataset-id <ID> --version <VERSION> --out <PATH>` and the backwards-compatible `export-ohlcv` alias.
- `cargo test --bin archon trading_slash_alias_routing` — TUI slash-alias routing for owned commands.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `failed` — but for an INFRASTRUCTURE reason, not a judgement on the work:
> fanout branch 'verification-wave-verify-task-tdl-010-6-0' produced invalid structured output after repair

The verifier never rendered an opinion on this task. Treat the prior run as giving you no
information about correctness here — neither confidence nor suspicion.

**Unremediated findings against this task (3, 2 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `None` · medium · coverage review**

- *finding:* Accepted task evidence does not appear to cover parse tests for every owned TradingCliDataAction variant; executed evidence is limited mainly to list/export/export-ohlcv and manual ingest validation.
- *evidence:* TASK contract lines 36-40 and 79-86 require CLI coverage, --target support, and parse tests.; Accepted verification commands only include trading_data_prd_commands_parse, trading_data_list_json_dispatches_to_registry, trading_data_export_dispatches_to_dataset_bars, manual_ingest_contract_requires_deterministic_id_and_version, and TUI alias test.; trading_market_actions.rs defines additional owned variants not explicitly evidenced by accepted verification: status, show, validate, providers, capability, fetch-native, snapshot, coverage, verify-artifact, verify-coverage.

**F2 · `None` · medium · coverage review**

- *finding:* Accepted evidence covers list/export and TUI aliases, but does not appear to include focused executed tests for CLI data status or data show; ingest coverage is indirect rather than clearly parse/dispatch compatibility coverage.
- *evidence:* TASK contract line 36 names status, list, show, ingest-ohlcv, export.; Accepted verification task_coverage lists list/export/manual ingest deterministic validation but not status/show parse or dispatch commands.

**F3 · `None` · low · coverage review**

- *finding:* Accepted evidence does not report any line-count or complexity check results.
- *evidence:* TASK contract lines 102-103 require line-count and complexity checks.; Accepted verification commands_run contains inspections and focused cargo tests, but no file-size/line-count or complexity check command.

<!-- PRIOR-RUN-FINDINGS:END -->

## Required Task Checklist

- implements (normative requirement IDs)
- scope
- files expected to change
- files forbidden to change
- acceptance criteria
- focused tests
- line-count check
- complexity check where applicable
- adversarial review notes
- explicit residual gaps with fail-closed behavior

## Global Constraints

- Keep changed/new files under 500 lines where possible.
- No hardcoded secrets or provider credentials.
- No production candle resampling.
- No vague "later", "TBD", "probably", or "best effort" without a residual gap record.
