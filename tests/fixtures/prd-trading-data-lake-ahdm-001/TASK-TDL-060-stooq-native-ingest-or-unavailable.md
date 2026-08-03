# TASK-TDL-060 — Stooq Native Ingest or Unavailable

```yaml
task_id: TASK-TDL-060
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Stooq Native Ingest or Unavailable
workstream: W2 Providers + Coverage
complexity: medium
status: blocked
depends_on: ['TASK-TDL-030']
blocks: ['TASK-TDL-080']
source_sections: ['7', '8.4', '25.3']
implements: [REQ-DL-035]
required_env_keys: []
required_tools: []
shared_append_target_files: [.archon/trading-lab/data/registry.json]
deliverable_contracts:
  - kind: native_dataset_manifest
    artifact_path: .archon/trading-lab/data/datasets/<dataset-id>/<version>/manifest.json
  - kind: provider_capability_record
    artifact_path: .archon/trading-lab/data/provider-capabilities.json
  - kind: trading_data_registry_entry
    artifact_path: .archon/trading-lab/data/registry.json
```

## Purpose

Implement Stooq exact-native ingest where directly available, otherwise honest unavailable handling.

## Scope

### In

- Exact native intervals only.
- HTML/block/verification detection.
- `provider_blocked_or_unavailable` mapping.
- Baseline quality metadata.
- Complete dataset artifact contract from TASK-TDL-010 for successful native ingest, including `manifest.json`, `ohlcv.jsonl`, `validation.json`, `raw/request.json`, `raw/headers.redacted.json`, `raw/response.<json|csv|zip|txt>`, and `raw/provider-notes.md`, or explicit fail-closed unavailable records.
- CLI/TUI provider path for `data fetch-native --provider stooq`, returning exact unavailable details when native data is unavailable or blocked.

- Full PRD fetch command signature: `archon trading data fetch-native --provider <PROVIDER> --symbol <SYMBOL> --timeframe <TF> --start <RFC3339|YYYY-MM-DD> --end <RFC3339|YYYY-MM-DD> --dataset-id <ID> [--target <PROJECT>]`.
- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.

### Out

- Bot-detection bypass.
- Weekly-from-daily, 4H-from-hourly, or 15m-from-5m derivation.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, **`src/command/trading_data_provider/stooq.rs`** (the Stooq fetch implementation — REQUIRED write scope for this task), `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- REAL LIVE FETCH REQUIRED (production deliverable — not a fixture): any Stooq dataset recorded `production_eligible=true`/Healthy MUST originate from an actual live Stooq fetch this run, evidenced by captured `raw/request.json`, `raw/response.<json|csv|zip|txt>`, and `raw/provider-notes.md` showing the real call returned real native candles. The CSV-parse and HTML-block fixture tests may validate parse/detection logic only; they MUST NOT satisfy the dataset deliverable or back a production-eligible registry entry.
- REAL VOLUME REQUIRED, NOT A PROBE: any production-eligible Stooq dataset's bar count MUST be consistent with the requested `--start`/`--end` span (hundreds+ bars for a multi-year daily request). A token/probe fetch proves connectivity ONLY and MUST NOT be recorded `production_eligible=true`/Healthy. State the requested span and returned bar count explicitly; a material shortfall is a residual gap.
- No fixture, synthetic, or placeholder dataset may be recorded `production_eligible=true`/Healthy. If Stooq is unavailable/blocked, record an honest `provider_capability_record` unavailable state with captured evidence and `production_eligible=false` — never a fabricated dataset.
- AC-DL-007 for Stooq path.
- Blocks and non-data HTML are reported honestly.
- No resampling path can produce production-eligible Stooq data.
- Successful ingest writes a v2 registry entry pointing to metadata, validation report, manifest, normalized JSONL, and raw artifacts; interrupted ingest must not leave a healthy registry entry.
- Clap parsing and command dispatch exist for every owned CLI command, with matching TUI slash alias routing where required.
- CLI parse tests require all mandatory `fetch-native` flags: `--provider`, `--symbol`, `--timeframe`, `--start`, `--end`, and `--dataset-id`; `--target` remains optional.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test --bin archon stooq_csv_parse_fixture` — Stooq CSV parse fixture.
- `cargo test --bin archon stooq_non_data_response` — non-data HTML/block fixture.
- `cargo test --bin archon stooq_native_interval_refusal` — native interval refusal.
- `cargo test --bin archon fetch_native_stooq_parse` — CLI parse and TUI slash-alias routing for every owned command.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `failed` — but for an INFRASTRUCTURE reason, not a judgement on the work:
> fanout branch 'verification-wave-verify-task-tdl-060-45-0' produced invalid structured output after repair

The verifier never rendered an opinion on this task. Treat the prior run as giving you no
information about correctness here — neither confidence nor suspicion.

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
