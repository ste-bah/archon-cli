# TASK-TDL-080 — Coverage Matrix Command

```yaml
task_id: TASK-TDL-080
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Coverage Matrix Command
workstream: W2 Providers + Coverage
complexity: large
status: blocked
depends_on: ['TASK-TDL-040', 'TASK-TDL-041', 'TASK-TDL-042', 'TASK-TDL-050', 'TASK-TDL-060', 'TASK-TDL-070']
blocks: ['TASK-TDL-090', 'TASK-TDL-100']
source_sections: ['6', '8.5', '23', '24']
implements: [REQ-DL-040, REQ-DL-041, REQ-DL-042, REQ-DL-061, REQ-DL-062]
required_env_keys: [OPENBB_API_URL, POLYGON_API_KEY]
required_tools: [data_get_ohlcv]
deliverable_contracts:
  - kind: required_universe_registry
    artifact_path: .archon/trading-lab/data/coverage/latest.json
    registry_path: .archon/trading-lab/data/registry.json
    typed_verifier_command: archon trading data verify-coverage {artifact_path} {registry_path}
    required_universe: true
    data_kind: record_series
    universe_fields: [instruments, timeframes]
    cells_field: cells
    cell_identity_fields: [canonical_instrument, timeframe]
    required_true_fields: [available, native_interval, production_eligible]
    required_nonempty_fields: [provider_symbol, timeframe, dataset_id, version]
    positive_count_fields: [row_count]
    minimum_count_fields: {row_count: 200}
    registry_minimum_count: 200
    gaps_field: gaps
    registry_records_field: datasets
    registry_key_fields: [dataset_id, version]
    registry_required_true_fields: [native_interval, production_eligible]
    registry_status_field: status
    registry_allowed_statuses: [Healthy]
    registry_count_field: bars
    registry_identity_fields:
      canonical_instrument: symbol
      timeframe: timeframe
    payload_path_field: normalized_path
    payload_format: jsonl
    required_fields: [timestamp, open, high, low, close, volume]
    observed_time_field: timestamp
    non_constant_fields: [close, volume]
    series_value_fields: [open, high, low, close, volume]
    series_overlap_min_rows: 5
    request_path_field: raw_request_path
    requested_count_field: count
    response_path_field: raw_response_path
    response_identity_fields:
      provider_symbol: symbol
      timeframe: timeframe
    validation_path_field: validation_path
    validation_status_field: status
    validation_checks_field: checks
    validation_check_status_field: status
    validation_failed_values: [failed]
    validation_passed_values: [passed]
```

## Purpose

Generate JSON and readable coverage matrix for the required trading-core-v1 universe.

## Scope

### In

- Universe: ES, NQ, SPY, QQQ, BTCUSDT, ETHUSDT.
- Timeframes: 1W, 1D, 240, 60, 15.
- Provider selection order and freshness defaults, including current snapshots stale after 5 minutes.
- CLI command `archon trading data coverage --universe trading-core-v1`.
- Required TUI alias surface for `/trading data coverage ...`; if the TUI command registry does not yet exist, this task must create or extend it rather than accept CLI-only behavior.

- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.

### Out

- Forcing downloads in coverage generation unless explicitly requested.
- Treating gaps as pass conditions.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- COVERAGE REQUIRES REAL VOLUME: a cell counts as covered only when its backing dataset meets the contract's declared `minimum_count_fields` / `registry_minimum_count` thresholds. Probe-sized datasets (a handful of bars) are NOT coverage and must be reported as gaps with the observed bar count in the reason.
- NO FIXTURE-BACKED COVERAGE: a coverage cell may be marked `available=true` / `production_eligible=true` / `native_interval=true` ONLY when its backing registry dataset originated from a real live provider fetch (captured `raw/request.json` + `raw/response.*` provenance) with genuine native candles. A dataset that is fixture-, mock-, synthetic-, or placeholder-derived, or that lacks captured live-fetch evidence, MUST appear as a gap with an exact reason — never as satisfied coverage. Minimal-row placeholder datasets crafted only to clear `series_overlap_min_rows` do NOT constitute coverage.
- AC-DL-004 satisfied.
- Coverage JSON schema matches PRD §23.
- Text report is human-readable.
- Gaps include exact reasons.
- Coverage consumes the snapshot freshness contract from TASK-TDL-030; snapshots older than 5 minutes are stale.
- `--target` is supported consistently.
- `/trading data coverage ...` provides TUI command parity with the CLI coverage command.
- Clap parsing and command dispatch exist for every owned CLI command, with matching TUI slash alias routing where required.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading coverage_matrix_generation` — coverage matrix generation.
- `cargo test -p archon-trading coverage_provider_selection_order` — provider selection order.
- `cargo test -p archon-trading coverage_freshness_classification` — freshness classification.
- `cargo test --bin archon trading_data_coverage_output` — JSON and text output.
- `cargo test --bin archon trading_slash_alias_routing` — TUI `/trading data coverage` alias routing, and every other owned alias.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `failed` (semantic)

> TASK-TDL-080 is not verified: the required coverage artifact exists but fails its typed verifier because all 30 required trading-core-v1 cells are gaps, and the working tree contains uncommitted changes outside the task's likely ownership anchors. Code includes a coverage command, verifier, fail-closed volume/provenance checks, CLI parse surface, and TUI command listing, but focused verification exposed missing/weak evidence for provider-order and TUI alias routing tests, and artifact contract is not satisfied.

**Findings:** none. The mandatory reviews inspect only ACCEPTED tasks, so a task that was
never accepted produced no findings. Absence of findings here is absence of review, not a
clean bill of health.

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
