# TASK-TDL-130 — AHDM Native Backtest

```yaml
task_id: TASK-TDL-130
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: AHDM Native Backtest
workstream: W5 Backtest + Readiness
complexity: large
status: blocked
depends_on: ['TASK-TDL-110', 'TASK-TDL-090', 'TASK-TDL-120']
blocks: ['TASK-TDL-140']
source_sections: ['10', '27', '29']
implements: [REQ-AHDM-008, REQ-AHDM-024]
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: backtest_config
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/config.json
  - kind: backtest_report
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/report.json
  - kind: backtest_trades
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/trades.jsonl
  - kind: backtest_equity_curve
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/equity_curve.jsonl
```

## Purpose

Implement Archon-native AHDM-v1 backtest using registered validated OHLCV datasets only.

## Scope

### In

- Backtest config hash.
- Dataset ids, versions, providers, timeframes.
- Trades, PnL, drawdown, win rate, expectancy, costs, slippage, equity curve artifact.
- Replayable report from registered artifacts.
- Backtest implementation consumes the same AHDM-v1 rule manifest used by Pine artifacts; divergent native-only rules fail review.
- Backtest artifacts under `.archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/`: `config.json`, `report.json`, `trades.jsonl`, `equity_curve.jsonl`.

### Out

- Loose data files.
- Live trading.
- Promotion with diagnostic data.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- SUFFICIENT HISTORY REQUIRED: the backtest MUST run over datasets whose bar counts are statistically meaningful for the strategy's timeframe (a probe-sized dataset of a handful of bars is not a backtest). If only probe/fixture-sized datasets exist, BLOCK HONESTLY and record the observed bar counts rather than reporting backtest metrics.
- REAL DATA BACKTEST (not a fixture): the delivered `report.json` / `trades.jsonl` / `equity_curve.jsonl` MUST be produced by running the backtest against real production-eligible datasets with captured live-fetch provenance (as gated by TASK-TDL-090). The happy-path fixture may exercise the backtest engine in tests only; it MUST NOT satisfy the backtest deliverables. If no real production-eligible dataset exists for the required universe, BLOCK HONESTLY rather than backtesting on fixture/synthetic data.
- AC-AHDM-003 satisfied.
- Backtest reports are replayable from registry artifacts.
- Invalid/non-native/degraded datasets are refused unless diagnostic override is explicit.
- Diagnostic reports cannot satisfy promotion gates.
- Backtest run writes required artifact layout under `.archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/` with `config.json`, `report.json`, `trades.jsonl`, and `equity_curve.jsonl`.
- Native backtest rules match the shared rule manifest used by Pine artifacts; divergence fails review.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading native_backtest_happy_path` — native backtest happy-path fixture.
- `cargo test -p archon-trading backtest_invalid_dataset_refusal` — the refusal tests inherited from TASK-TDL-090.
- `cargo test -p archon-trading native_backtest_replayability` — report replayability.
- `cargo test -p archon-trading shared_rule_manifest_parity` — shared rule manifest parity against the Pine-owned manifest.
- `cargo test -p archon-trading native_backtest_costs_slippage` — costs and slippage accounting.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `needs_review` (semantic)

> Verification rejected TASK-TDL-130 acceptance: code and focused tests cover some fail-closed gates, but the promotion artifact evidence is not trustworthy as real live production data, current worktree contains unrelated dirty changes, and the native run executes generic CloseMomentum logic rather than the AHDM shared rule semantics it records.

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
