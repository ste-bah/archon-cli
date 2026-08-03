# TASK-TDL-110 — AHDM-v1 StrategySpec

```yaml
task_id: TASK-TDL-110
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: AHDM-v1 StrategySpec
workstream: W4 AHDM Strategy
complexity: large
status: blocked
depends_on: ['TASK-TDL-100']
blocks: ['TASK-TDL-120', 'TASK-TDL-130']
source_sections: ['9', '27', '28']
implements: [REQ-AHDM-001, REQ-AHDM-005, REQ-AHDM-020, REQ-AHDM-021]
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: strategy_spec
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
```

## Purpose

Create deterministic AHDM-v1 StrategySpec referencing registered dataset ids/versions.

## Scope

### In

- Instrument universe and timeframe stack.
- Required datasets by id/version.
- Daily bias formula and confidence scoring, including exact initial component weights: higher-timeframe trend/regime 20, liquidity location/prior highs-lows 20, vector/volume behavior 20, ADR/AWR/range state 15, VWAP/EMA relationship 10, session timing/macro filter 10, recent postmortem penalty 5.
- Entry models, invalidation, stops, TP1/TP2/TP3, filters, sizing.
- The three PRD entry models with evidence requirements and fail-closed no-trade handling when required evidence is missing.
- Slippage/cost assumptions, data quality gates, promotion gates, paper-readiness gates.
- Source citations and residual gaps.
- StrategySpec artifact path `.archon/trading-lab/strategies/AHDM-v1/strategy-spec.json`.
- Shared AHDM-v1 rule manifest consumed by Pine and Archon-native backtest implementations.

### Out

- Live trading.
- Probability marketing claims.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-AHDM-001 satisfied.
- Confidence is a score, not probability.
- `confidence < 0.55` produces no-trade.
- `confidence >= 0.70` only allows paper consideration after backtest gates pass.
- StrategySpec references datasets, not loose files.
- StrategySpec is written to `.archon/trading-lab/strategies/AHDM-v1/strategy-spec.json` and includes all PRD §27 required fields.
- StrategySpec encodes the exact initial bias components, weights, evidence requirements, and three entry models from the PRD.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading strategy_spec_schema_completeness` — StrategySpec schema and field completeness.
- `cargo test -p archon-trading strategy_spec_bias_weight_sum` — bias component weight sum and required evidence.
- `cargo test -p archon-trading strategy_spec_entry_model` — entry model completeness.
- `cargo test -p archon-trading strategy_spec_dataset_reference` — dataset reference validation.
- `cargo test -p archon-trading strategy_spec_confidence_threshold` — confidence and no-trade thresholds.
- `cargo test -p archon-trading strategy_spec_position_sizing` — position sizing and risk calculation.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `failed` — but for an INFRASTRUCTURE reason, not a judgement on the work:
> fanout branch 'verification-wave-verify-task-tdl-110-74-0' produced invalid structured output after repair

The verifier never rendered an opinion on this task. Treat the prior run as giving you no
information about correctness here — neither confidence nor suspicion.

**Unremediated findings against this task (6, 5 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `F-TDL110-001` · high · adversarial review**

- *summary:* StrategySpec claims loose_file_references_allowed=false, but delivered dataset refs embed manifest_path, normalized_path, raw_path, and validation_path.
- *evidence:* strategy-spec.json lines 151-240 show dataset_refs with file path fields; TASK acceptance criterion says StrategySpec references datasets, not loose files
- *recommended action:* Remove loose path fields from StrategySpec production evidence or explicitly separate non-evidence diagnostic paths from registered dataset references.

**F2 · `F-TDL110-002` · high · adversarial review**

- *summary:* AHDM dataset selection does not fail closed when AHDM coverage is missing; it falls back to unrelated native production-eligible records.
- *evidence:* ahdm.rs lines 224-228 select all native production_eligible records if ahdm_records.is_empty(); strategy_spec.rs lines 20-27 expects polygon-BTCUSD-1D-raw in an AHDM StrategySpec
- *recommended action:* Require zero or explicit unavailable AHDM refs when AHDM universe cells are absent; do not substitute unrelated datasets.

**F3 · `F-TDL110-003` · medium · adversarial review**

- *summary:* The task source remains status: blocked with depends_on TASK-TDL-100 while workflow acceptance treats TASK-TDL-110 as accepted/noop.
- *evidence:* TASK-TDL-110 markdown lines 10-18
- *recommended action:* Reconcile task lifecycle metadata or provide bounded evidence that TASK-TDL-100 was satisfied before acceptance.

**F4 · `F-TDL110-004` · medium · adversarial review**

- *summary:* Focused test suite validates the non-AHDM fallback instead of rejecting it, so tests can pass while AHDM required-dataset semantics are weakened.
- *evidence:* strategy_spec.rs lines 20-27 asserts polygon-BTCUSD-1D-raw and available_cells=1 for an AHDM StrategySpec
- *recommended action:* Add a test that no non-AHDM dataset can satisfy AHDM required_datasets or coverage gates.

**F5 · `None` · medium · coverage review**

- *finding:* Accepted remediation/verification evidence ran ahdm_strategy_spec_contains_required_model_contract, ahdm_strategy_spec_mirrors_degraded_registry_refs_and_fails_closed, and ahdm_strategy_spec_counts_only_promotable_required_cells, but did not show a position sizing/risk calculation test invocation or result.

**F6 · `None` · low · coverage review**

- *finding:* Accepted task evidence does not show file-size/line-count or complexity checks. Because the accepted task was a noop/read-only verification this may not indicate implementation failure, but it is not covered by the accepted evidence against the source checklist.

**Cross-task findings naming this task (3).**

Authored by the review reduce stage, which sees all tasks at once. No single task owns these;
they are context. Address the part that lies inside your declared scope and say plainly what
belongs elsewhere — do not attempt the whole pattern from here.

**CROSS-TASK — F1 · `REDUCE-XTASK-001` · high · adversarial review**

- *summary:* Multiple tasks are treated as accepted by workflow evidence while their task source metadata remains blocked.
- *evidence:* TDL020-ADV-003: TASK-TDL-020 task file lines 3-11 status: blocked while accepted verifier artifact status: accepted.; F-TDL030-004: Task contract YAML line 10 says status: blocked while verification branch result status is accepted with residual_gaps=[].; F-TDL110-003: TASK-TDL-110 remains status: blocked with depends_on TASK-TDL-100 while workflow acceptance treats it as accepted/noop.; TDL120 F2: Task spec YAML line 10 has status: blocked while TASK-TDL-120 accepted/completed by workflow.
- *impact:* Workflow consumers may derive conflicting execution order and readiness decisions depending on whether they trust task files or verifier artifacts.
- *status:* open
- *recommended action:* Define a single authoritative lifecycle source or update task metadata atomically as part of verified acceptance.

**CROSS-TASK — F2 · `REDUCE-XTASK-004` · medium · adversarial review**

- *summary:* Multiple task verifications overstate test coverage by testing weaker paths than the acceptance criteria require.
- *evidence:* TDL020-ADV-002: slash routing parser test covers list/export only, not required validate or validate-ohlcv.; TDL020-ADV-004: invalid duplicate/OHLC/volume focused tests inspect validation_report directly rather than end-to-end CLI validation of persisted invalid JSONL.; TDL041 F2: focused snapshot tests use ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE and do not verify live provider behavior.; F-TDL110-004: focused test suite validates non-AHDM fallback instead of rejecting it.; Later Pine F5: validation script checks marker strings and JSON key presence only, not semantic risk-analysis correctness.
- *impact:* Tests can pass while business-critical routing, live data provenance, AHDM semantics, or review quality are broken.
- *status:* open
- *recommended action:* Add negative and end-to-end tests that fail when the asserted acceptance semantics are weakened.

**CROSS-TASK — F3 · `REDUCE-XTASK-005` · high · adversarial review**

- *summary:* Several accepted data-lake artifacts weaken source-of-truth or contract semantics while still being presented as compliant evidence.
- *evidence:* Initial F4: wildcard/template storage paths are marked present as 'Observed or contract-required' rather than direct physical evidence.; finding-tdl070-001: 4H/240 yfinance requests can receive 1h Yahoo bars stored as native 4H/240 datasets.; F-TDL110-001: StrategySpec claims loose_file_references_allowed=false while dataset_refs embed manifest_path, normalized_path, raw_path, and validation_path.; F-TDL110-002: AHDM dataset selection falls back to unrelated native production-eligible records when AHDM coverage is missing.
- *impact:* The implementation can appear to satisfy registry/storage/strategy contracts while substituting templates, non-native intervals, loose files, or unrelated datasets.
- *status:* open
- *recommended action:* Fail closed when physical dataset evidence or exact semantic coverage is absent; prohibit contract-required placeholders from counting as observed evidence.

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
