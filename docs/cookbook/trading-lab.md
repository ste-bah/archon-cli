# Trading Lab Cookbook

This cookbook shows how to use the Trading Research and Execution Lab as it
exists today. The important thing to understand up front:

```text
The Trading Lab is a governed research and validation system.
It is not yet a one-command trading terminal.
```

You use it with Archon's document store, KB, research/workflow systems, and the
implemented `archon-trading` primitives. Live trading remains gated and disabled
unless your policy, broker adapter, certification, and maker-checker evidence
all say it is allowed.

## What You Are Building Toward

A complete strategy lifecycle is:

```text
KBs -> research -> 15-field spec -> Pine prototypes -> data registration
    -> deterministic backtest -> paper trading -> postmortem
    -> promotion review -> live dry-run certification
```

The core idea is that every promotion needs evidence, not confidence.

## Phase 0: Prepare Trading Knowledge

Create separate KBs so Archon can reason over the right evidence without mixing
unrelated domains.

Suggested KBs: `trading-market-structure`, `trading-elliott-wave`,
`trading-execution`, `trading-risk-management`, `trading-backtesting`,
`trading-strategy-research`, and `trading-postmortems`.

Example ingest flow in the TUI:

```text
/docs ingest ./assets/research-paper/trading/trading-market-structure
/docs ingest ./assets/research-paper/trading/trading-risk-management
/docs ingest ./assets/research-paper/trading/trading-backtesting
/docs ingest ./assets/research-paper/trading/trading-elliott-wave
/docs index
```

If you have videos:

```text
/video ingest "https://youtu.be/<id>" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
/docs index
```

Then extract structured knowledge:

```text
/kb process --kb trading-market-structure --claims --entities --relations --contradictions
/kb process --kb trading-risk-management --claims --entities --relations --contradictions
/kb process --kb trading-backtesting --claims --entities --relations --contradictions
/kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
```

Check that retrieval works before you ask Archon to build strategy specs:

```text
/kb search --kb trading-elliott-wave "wave 2 retracement invalidation"
/docs answer "what do my sources say about preventing overfit in walk-forward tests?"
```

## Phase 1: Research a Strategy Thesis

Use `/archon-research` or `/workflow` to turn source material into a structured
thesis. Keep the prompt evidence-focused.

Example TUI prompt:

```text
/archon-research Research whether an Elliott Wave + volatility-regime filter can produce a testable Bitcoin swing-trading strategy. Use the trading-elliott-wave, trading-risk-management, and trading-backtesting KBs. Extract exact rules, invalidation conditions, no-trade conditions, known failure modes, data dependencies, and backtest requirements. Do not claim profitability. Mark unsupported ideas as hypotheses.
```

What you want from the output:

- exact rule candidates
- source citations
- contradictions between sources
- no-trade conditions
- failure modes
- risk controls
- backtest design

Do not move to implementation if the research output is vague. Restart or
rewind the weak stage instead.

## Phase 2: Create a 15-Field Strategy Spec

Every strategy must become a full spec. If one of these fields is missing, the
strategy should remain an idea.

Template:

```yaml
strategy_id: btc-elliott-vol-regime-v1
SPEC-F01_instrument_universe:
  - symbol: BTCUSD
    venue: approved-data-provider
    asset_class: crypto
SPEC-F02_timeframe_session:
  timeframe: 4h
  session_hours: 24x7
SPEC-F03_market_regime_assumptions:
  - trend regime required
  - avoid extreme event/news windows
SPEC-F04_data_dependencies:
  - dataset_id: btc_ohlcv_4h
    version: v1
  - dataset_id: crypto_fees
    version: v1
SPEC-F05_entry_exit_rules:
  rules:
    - enter only after source-backed Elliott count and volatility filter agree
    - exit on count invalidation or stop/take-profit
SPEC-F06_indicator_formulas:
  formulas:
    - volatility regime classifier
    - swing high/low count helper
SPEC-F07_position_sizing:
  model: fixed_fractional
  max_risk_pct: "1"
SPEC-F08_stops:
  stop_rules:
    - invalidation below wave reference level
  take_profit_rules:
    - partial at measured move
  trailing_rules:
    - trail after first target
  max_strategy_drawdown_pct: 8.0
SPEC-F09_invalidation_rules:
  rules:
    - wave count invalidated
    - volatility regime flips
SPEC-F10_no_trade_conditions:
  rules:
    - data stale
    - major scheduled market event
    - spread/slippage above policy
SPEC-F11_cost_assumptions:
  slippage_bps: 5
  fee_bps: 2
SPEC-F12_benchmark:
  symbol: BTCUSD
  source: approved
SPEC-F13_expected_failure_modes:
  - subjective wave labeling
  - regime whipsaw
  - exchange fee/slippage drift
SPEC-F14_data_quality_tolerances_ms:
  btc_ohlcv_4h: 5000
SPEC-F15_promotion_status: idea
```

Ask Archon to validate the spec against the 15 fields before doing anything
else:

```text
/workflow run Validate this trading strategy spec against the Trading Lab 15-field contract. Identify missing fields, type errors, unsupported evidence, contradiction risks, and promotion blockers. Do not write code.
```

## Phase 3: Generate Pine Script Variants

The Trading Lab expects two Pine outputs for a research-or-later strategy:

- indicator variant for visual signals, overlays, alerts, diagnostics
- strategy variant for TradingView Strategy Tester support

Important constraints:

- Pine must be v6.
- TradingView docs should be checked before writing code.
- Alerts are not orders.
- Multi-symbol strategies become one Pine script per symbol.
- Compile proof must be tied to the source hash.

Good workflow prompt:

```text
/workflow run Generate Pine Script v6 indicator and strategy variants for the attached 15-field strategy spec. Check Pine v6 docs before writing. Include alertcondition or strategy alert messages for handoff, but mark alerts as non-authoritative order intents. Do not use cross-symbol portfolio logic inside Pine. Produce compile-check instructions and a script registry entry with source hash fields.
```

What to inspect:

- `//@version=6`
- no unsupported multi-symbol aggregation
- inputs for thresholds/windows/sessions/risk display
- alert handoff is explicit
- strategy variant has exact entries/exits from `SPEC-F05`
- compile result is recorded against the source hash

## Phase 4: Register Market Data

A backtest is only useful if the data is known and replayable.

Dataset metadata should include:

```yaml
dataset_id: btc_ohlcv_4h
provider: approved_provider
symbol_mapping:
  canonical: BTCUSD
timezone: UTC
adjustment_mode: raw
license: licensed
coverage:
  start: 2021-01-01
  end: 2026-01-01
gaps:
  expected_bars: 10950
  missing_bars: 12
checksum: blake3-or-provider-checksum
version: v1
required: true
```

Promotion rules:

- required datasets must be healthy
- gap coverage above 1 percent is degraded
- degraded optional data does not satisfy mandatory data
- research-only data is not promotion evidence
- TradingView Strategy Tester output is auxiliary only

OpenBB can be used as a data gateway only for provider/data-type pairs that are
allowed by the OpenBB allowlist. Do not pass API keys or credentials as dataset
parameters; the adapter rejects secret-looking fields.

## Phase 5: Run Deterministic Backtests

Backtests must be replayable from:

- strategy config hash
- data snapshot checksum
- pinned numeric-library version
- cost assumptions
- robustness suite settings

Minimum evidence expected:

| Evidence | Why |
|---|---|
| out-of-sample result | prevents pure in-sample fitting |
| walk-forward result | checks rolling adaptation |
| Monte Carlo reshuffle | checks path dependence |
| parameter stability | catches brittle parameters |
| regime-sliced metrics | checks market-state dependence |
| full cost model | stops fantasy fills |

The report must include the 11 metrics enforced by the harness: `net_profit`,
`gross_profit`, `gross_loss`, `max_drawdown`, `sharpe`, `sortino`,
`profit_factor`, `win_rate`, `trade_count`, `avg_trade`, and `cost_total`.

Backtest evidence is not accepted for promotion if:

- it is exploratory
- it is unpersisted
- it is from Strategy Tester alone
- it uses research-only data
- it uses degraded datasets

## Phase 6: Paper Trade

Paper trading uses the same order-intent and risk-governor path as live trading.
That is deliberate. The goal is to test the controls before real money is
involved.

Every paper order should produce:

- requested record
- accepted/rejected/partial/filled/cancelled record
- risk decision
- immutable ledger hash
- strategy id
- policy version

The paper sample gate requires at least 200 closed trades, 60 calendar days, 2
regimes, and postmortem readiness.

The gate reports the longest binding missing condition. If it says
`min_regimes`, adding more trades in the same regime will not help.

## Phase 7: Write Postmortems

Every paper/live session needs a structured postmortem.

Minimum postmortem fields:

```yaml
session_id: paper-session-001
mode: paper
strategy_ids:
  - btc-elliott-vol-regime-v1
trades:
  - trade_id: paper-1
    instrument: BTCUSD
    quantity: 0.1
    realized_pnl: -25.0
realized_pnl: -25.0
risk_events:
  - spread halt
spec_f13_deviations:
  - wave-count ambiguity higher than expected
lessons:
  - add no-trade rule for low-liquidity weekend sessions
session_closed_unix_ms: 1780000000000
completed_unix_ms: 1780000900000
```

Learning can record lessons and blocked unsafe attempts, but it must not change
live risk limits automatically.

## Phase 8: Promotion Review

Promotion is one step at a time:

```text
idea -> research -> backtest -> paper -> live-pilot
```

You cannot jump from `idea` to `paper`, and you cannot enter `live-pilot`
without paper evidence and postmortem evidence.

Promotion to `backtest` needs accepted OOS and walk-forward evidence.

Promotion to `paper` still needs backtest evidence and a valid spec.

Promotion to `live-pilot` needs a valid 15-field spec, accepted OOS and
walk-forward evidence, healthy required data, a passed paper sample gate,
postmortem evidence, maker-checker approval, and a live enablement gate.

## Phase 9: Live Readiness

Live trading is disabled by default. That is correct.

Before any live pilot, require:

- explicit policy enabling trading and live policy
- supported jurisdiction
- account identifiers
- valid risk policy hash
- kill switch validation
- dry-run certification
- production evidence
- maker-checker approval from distinct actors
- pilot capital cap: at most min(1 percent equity, USD 1000) unless policy says
  an even lower cap

Dry-run certification checks:

- broker capability manifest supports the order intent
- submit/cancel/replace/ledger path works
- broker health path works
- pre-trade p99 latency meets the SLO
- in-app and out-of-band kill-switch channels work

If any check fails, do not enable live.

## Day-One Example: Elliott Wave KB to Paper Candidate

This runbook uses the Elliott Wave material you ingested and produces a
paper-trading candidate without pretending it is validated.

1. Build the KB:

```text
/docs ingest ./assets/research-paper/trading/trading-elliott-wave
/docs ingest ./assets/Elliott Wave Cheat Sheet_ All You Need To Count.pdf
/docs index
/kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
```

2. Research a source-backed candidate:

```text
/archon-research Use trading-elliott-wave and trading-backtesting to design an evidence-backed Elliott Wave strategy candidate. Separate source-backed rules from subjective analyst judgement. Include invalidation rules and how to avoid hindsight wave labeling.
```

3. Convert it to a spec:

```text
/workflow run Convert the Elliott Wave strategy candidate into a Trading Lab 15-field strategy spec. Require objective entry/exit rules. If Elliott labels are subjective, mark them as a failure mode and require a confirmation filter.
```

4. Generate Pine variants:

```text
/workflow run Generate Pine v6 indicator and strategy variants for the Elliott Wave spec. The indicator should show candidate wave references and invalidation zones. The strategy should only encode objective rules from SPEC-F05/SPEC-F06. Alerts are non-authoritative order intents.
```

5. Backtest outside the LLM monologue. Use persisted data snapshots, config
hashes, fills, metrics, costs, OOS, walk-forward, Monte Carlo, parameter
stability, and regime slices. If the rules require subjective labels that
cannot be computed deterministically, the strategy stays at `research`.

6. Paper trade only after the spec and Risk Governor accept the order intent.
The paper gate needs 200 trades, 60 days, 2 regimes, and postmortem readiness.

7. Review promotion blockers:

```text
/workflow run Review this strategy for Trading Lab promotion readiness. Check 15-field spec completeness, data quality, OOS/WF evidence, paper sample gate, postmortem readiness, risk-policy hash, maker-checker requirements, and live-readiness blockers. Be adversarial.
```

## How Learning Fits In

Learning systems should improve the process, not trade directly.

| Learning input | Allowed effect |
|---|---|
| repeated bad strategy assumptions | update research heuristics |
| false claims in strategy notes | create completion/trust incidents |
| postmortem lessons | improve future research prompts and checks |
| blocked unsafe order recommendation | log safety pattern |
| repeated data-quality failure | propose stricter data policy |

Learning must not:

- raise live limits
- bypass maker-checker
- emit orders
- mark unverified claims as evidence
- promote a strategy without deterministic evidence

## Troubleshooting

### "Can I just ask Archon if the strategy is good?"

No. Ask it to produce evidence requirements, then run the deterministic checks.
Model confidence is not promotion evidence.

### "TradingView Strategy Tester says it works. Can I promote?"

No. Strategy Tester output is auxiliary. Promotion needs Archon's backtest
evidence with data snapshots, config hashes, costs, and robustness tests.

### "The Pine alert fired. Is that an order?"

No. Pine alerts produce order intents. The Risk Governor still has to approve.

### "Can learning auto-adjust risk limits?"

No. Learning can record lessons and propose changes, but upward risk changes
need maker-checker and audit.

### "Where is the `/trading` command?"

There is no primary `/trading` command yet. The implemented pieces are the
`archon-trading` crate, `archon-tools` trading dispatch primitives, and TUI
render model. Use `/docs`, `/kb`, `/archon-research`, `/workflow`, and the
library/tool facade until the full end-user command is wired.

### "Can I connect a real broker now?"

Only after you implement/certify the broker adapter in your environment, pass
dry-run certification, enable the relevant policy, and retain maker-checker
approval. The default posture is live disabled.

## Developer Commands

For engineering work on this subsystem:

```bash
cargo fmt --check
cargo test -p archon-trading -j1 -- --test-threads=2
cargo check -p archon-tools
cargo check -p archon-tui --lib
```

The source map is in [Trading Lab reference](../trading-lab.md).

## See Also

- [Trading Lab reference](../trading-lab.md)
- [Document intelligence](../docs.md)
- [Knowledge base](../knowledge.md)
- [Dynamic workflows](dynamic-workflows.md)
- [Trading and asset analysis with `/gametheory`](trading-with-gametheory.md)
