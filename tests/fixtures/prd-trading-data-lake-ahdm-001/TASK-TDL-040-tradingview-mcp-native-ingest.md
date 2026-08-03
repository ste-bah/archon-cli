# TASK-TDL-040 — TradingView MCP Native Ingest

```yaml
task_id: TASK-TDL-040
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: TradingView MCP Native Ingest
workstream: W2 Providers + Coverage
complexity: medium
status: in_review
depends_on: ['TASK-TDL-030']
blocks: ['TASK-TDL-041', 'TASK-TDL-042', 'TASK-TDL-080']
source_sections: ['7', '8.4', '25.1']
implements: [REQ-DL-033]
required_env_keys: []
required_tools: [tv_health_check, chart_get_state, data_get_ohlcv]
shared_append_target_files: [.archon/trading-lab/data/registry.json]
deliverable_contracts:
  - kind: native_dataset_manifest
    artifact_path: .archon/trading-lab/data/datasets/<dataset-id>/<version>/manifest.json
  - kind: trading_data_registry_entry
    artifact_path: .archon/trading-lab/data/registry.json
```

## Purpose

Implement TradingView MCP native OHLCV historical ingest where MCP exposes actions. Snapshot capture (TASK-TDL-041) and the CLI/TUI surface (TASK-TDL-042) were split out of this task after its combined remediation could not complete within one attempt budget.

## Scope

### In

- TradingView MCP status/state checks before fetch.
- Exact timeframe values `1W`, `1D`, `240`, `60`, `15`.
- Raw MCP artifacts, normalized JSONL, metadata, validation report.
- Complete dataset artifact contract from TASK-TDL-010, including `manifest.json`, `raw/request.json`, `raw/headers.redacted.json`, and `raw/provider-notes.md`, or explicit fail-closed residual gaps for unavailable artifacts.

- PAGE the provider call when the requested span exceeds the per-call bar cap; do not fail closed above the cap when more bars are legitimately available.
- Fetch through the DECLARED MCP tools, not a local CLI/Node shim standing in for them.
- Write the v2 registry entry on successful ingest; an ingest that persists artifacts without registering them is incomplete.

### Out

- Snapshot capture and staleness (TASK-TDL-041 owns those).
- Clap arguments and TUI alias routing (TASK-TDL-042 owns those).
- Treating TradingView as institutional vendor.
- Synthesizing missing range when MCP returns limited bars.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, **`src/command/trading_data_provider/tradingview.rs`** (the TradingView MCP fetch implementation — REQUIRED write scope for this task), relevant dispatch files, and command-specific tests. CLI argument and TUI routing files belong to TASK-TDL-042.
- KNOWN DEFECT TO FIX IN THIS TASK: `src/command/trading_data_provider/tradingview.rs` requests a fixed bar count (`const TRADINGVIEW_REQUESTED_BARS: usize = 100`) instead of deriving the count from the requested `--start`/`--end` span, so a requested span can never be satisfied and real volume cannot be proven. Derive the requested count from the span and timeframe (and page/repeat the MCP call if the provider caps per-call bars), then record requested-vs-returned counts honestly. A previous run diagnosed this correctly but its fix was discarded because the file was outside the declared write scope — the scope above now includes it.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- REAL LIVE MCP FETCH REQUIRED (production deliverable — not a mock/fixture): the delivered native dataset MUST originate from an actual live invocation of the declared TradingView MCP tools this run (`tv_health_check`, `chart_get_state`, `data_get_ohlcv`), evidenced by captured `raw/request.json` and `raw/provider-notes.md` showing the real MCP call and its returned native candles. The `mocked MCP response` focused test may validate capability mapping only; it MUST NOT satisfy the dataset deliverable or back a `production_eligible=true`/Healthy registry entry.
- REAL VOLUME REQUIRED, NOT A PROBE: the delivered production dataset's bar count MUST be consistent with the requested `--start`/`--end` span (a multi-year daily request means hundreds+ bars, not a handful). A token/probe fetch (e.g. `count: 5`) proves connectivity ONLY — it is diagnostic evidence and MUST NOT be recorded `production_eligible=true`/Healthy, and MUST NOT satisfy this task's dataset deliverable. State the requested span and the returned bar count explicitly; if the provider returns materially fewer bars than the requested span, record the shortfall as a residual gap and keep the dataset non-production-eligible.
- No mock, fixture, synthetic, or placeholder dataset may be recorded `production_eligible=true`/Healthy. Absent captured live-MCP-fetch evidence the registry entry MUST be `production_eligible=false`.
- If the TradingView MCP/session is unreachable, BLOCK HONESTLY with the captured failure; do NOT mint a placeholder dataset to pass. A no-op that asserts pre-existing code satisfies the task is NOT acceptance — the declared MCP tools must be exercised live this run.
- AC-DL-003 and AC-DL-007 for TradingView path.
- Limited bar coverage is recorded honestly, AND the fetch pages the provider call when the requested span exceeds the per-call cap rather than failing closed.
- Visible chart/session requirements return actionable errors.
- Chart-equivalent semantics are documented in metadata.
- Successful ingest writes a v2 registry entry pointing to metadata, validation report, manifest, normalized JSONL, and raw artifacts; interrupted ingest must not leave a healthy registry entry.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test --bin archon tradingview_capability_mapping` — TradingView capability mapping against a mocked MCP response.
- `cargo test --bin archon tradingview_native_normalization` — native ingest normalization.
- `cargo test --bin archon tradingview_span_paging` — a requested span above the per-call cap returns the full span across multiple calls. This is the test that fails against the fixed `TRADINGVIEW_REQUESTED_BARS` defect.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

#### KNOWN FAILING TEST — `d44_tradingview_zero_or_missing_volume_stays_degraded`

This test is **red on the current committed tree** (`1739be45`). Fix it first: it is not a
reviewer's opinion to be weighed, it is a failing assertion with its own proof.

- **Where:** `src/command/trading_data_provider_tests/tradingview.rs:212`
- **What:** A TradingView dataset with zero/missing volume returns `production_eligible: true` where the test requires `false`, and the registry record is not marked `DatasetStatus::Degraded`. A degraded dataset presented as production-eligible is a **fail-open**: downstream promotion and backtest gates trust that flag.
- **Attribution:** Fails deterministically in isolation at `1739be45`. Attributed by building at `1739be45` WITHOUT the engine-fix commit — it fails there too, so it is the prior run's own work, not the engine fixes and not an interaction between them.

Do not resolve this by changing the test's expectation. The assertion encodes the fail-closed
contract; the production code is what drifted from it.

**Prior outcome:** `failed` (semantic)

> Verification rejects TASK-TDL-040 acceptance: live TradingView MCP tools were exercised in this verification run, but the repository implementation still fetches through a local Node CLI shim rather than declared MCP tools, the implementation workflow result is failed/rejected, the main TradingView provider file is already over the 500-line cap, and inspected production-eligible artifacts include probe/small or incomplete datasets that cannot satisfy the task deliverable.

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
