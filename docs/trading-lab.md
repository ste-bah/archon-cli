# Trading Research and Execution Lab

The Trading Lab is Archon's governed trading research substrate. It is designed
to turn trading knowledge into auditable strategy specs, Pine Script prototypes,
deterministic backtests, paper-trading evidence, and eventually tightly gated
live-trading readiness.

It is not a signal bot and it is not an "LLM places orders" feature. The core
rule is simple:

```text
knowledge -> strategy spec -> deterministic tests -> paper evidence
          -> postmortem -> promotion gate -> live readiness review
```

Every step should leave evidence that can be replayed, audited, and rejected.

## Current Status

The implementation currently provides the core crate, tests, command/tool
facade primitives, a user-facing command surface, and a TUI panel model:

| Layer | Status | Notes |
|---|---|---|
| `archon-trading` crate | Implemented | Core strategy, risk, backtest, Pine, paper/live, audit, and learning primitives |
| Backtest and NFR tests | Implemented | `cargo test -p archon-trading` covers the core invariants |
| Agent/tool dispatch facade | Implemented | `crates/archon-tools/src/trading` has policy-fenced command routing primitives |
| TUI trading panel model | Implemented | `crates/archon-tui/src/trading` renders status, ledger, risk rows, and kill button model |
| User-facing `archon trading` and `/trading` command | Implemented | Setup/status, TradingView MCP CLI pass-through, Pine generation/checks, governed OpenBB fetches, spec validation, native backtests, paper checks, TradingView replay-paper submit, Trading Lab workflow spec generation, promotion checks, live-readiness gates, route inspection, fenced dispatch checks, and out-of-band kill controls |
| Real broker live trading | Disabled by design | Live enablement requires explicit policy, certification, evidence, and maker-checker approval |

If you are a user trying this today, treat the Trading Lab as a governed
research and validation system. The command surface proves routes and gates; it
does not turn Archon into a one-command broker terminal.

## Command Surface

The shell and TUI paths use the same implementation:

```text
archon trading status
/trading status
```

Available subcommands:

| Command | Purpose |
|---|---|
| `status` | Show Trading Lab readiness and live-trading safety state |
| `routes` | Show which crate module owns each command family |
| `setup` | Run project-local TradingView MCP/OpenBB setup via `scripts/setup-trading-tools.sh` |
| `tools status` | Inspect project-local Node/Python, TradingView MCP, OpenBB, and `.mcp.json` readiness |
| `tv status` / `tv launch` / `tv cli` | Use the installed TradingView MCP CLI from `.archon/tools/tradingview-mcp` |
| `pine generate` / `pine analyze` / `pine check` | Generate Pine v6 files from StrategySpec JSON and validate them through TradingView MCP helpers |
| `openbb status` / `openbb fetch` | Inspect the OpenBB runtime and fetch governed datasets through explicit metadata/quality gates |
| `spec validate` | Validate a StrategySpec JSON file and emit a content hash |
| `backtest run` | Run the native deterministic backtest harness from config/fill JSON |
| `data status` / `data ingest-ohlcv` / `data list` / `data show` / `data export-ohlcv` | Persist, inspect, and export versioned OHLCV datasets under `.archon/trading-lab/data` |
| `backtest run-ohlcv` | Run deterministic candle backtests from stored OHLCV datasets with built-in or custom strategy rules |
| `paper submit` / `paper sample` | Submit paper order intents through the risk governor and evaluate paper-sample gates |
| `promote check` | Evaluate one-step promotion gates from spec and evidence JSON |
| `live enable-check` / `live pilot` / `live phase5-check` | Evaluate live-readiness gates and pilot limits without broker submission |
| `dispatch` | Exercise command/action/persona gates without placing orders |
| `kill` | Trigger the out-of-band kill-switch path and show its receipt |

Examples:

```text
archon trading routes
archon trading setup --target /path/to/project
archon trading tools status --target /path/to/project
archon trading tv status --target /path/to/project
archon trading pine generate --strategy-id demo --spec strategy-spec.json --out ./pine
archon trading spec validate --spec strategy-spec.json
archon trading data ingest-ohlcv --source candles.csv --format csv --dataset-id btc-1d --version v1 --provider openbb --symbol BTCUSD
archon trading data list
archon trading backtest run --config backtest.json --fills fills.json
archon trading backtest run-ohlcv --config backtest.json --dataset-id btc-1d --version v1 --quantity 1 --strategy-rules strategy-rules.json
archon trading paper sample --sample paper-sample.json
archon trading promote check --spec strategy-spec.json --target paper --evidence evidence.json
archon trading live enable-check --request live-enable.json
archon trading openbb fetch --request request.json --metadata metadata.json --quality quality.json --store-ohlcv --response-format json
archon trading dispatch backtest --action run-backtest --persona per05-execution-agent
archon trading dispatch kb --action write-kb --persona per07-observer
archon trading kill --actor operator --reason "manual halt" --working-orders 0
```

The second dispatch should be accepted. The third should be refused because
`PER-07` is read-only. This is intentional: the command exists partly so users
can verify the policy fences before building richer trading workflows.

Live dispatch checks remain fail-closed unless explicit policy and
maker-checker flags are present:

```text
archon trading dispatch live \
  --action submit-live-order \
  --persona per01-human-governor \
  --maker-checker-approved \
  --live-policy-enabled
```

This is a route/policy probe only. The dedicated `spec`, `backtest`, `paper`,
`promote`, `live`, `pine`, and `openbb` subcommands run the Trading Lab domain
paths. Broker orders are never submitted by default.

## Mental Model

The Trading Lab separates "thinking about trades" from "being allowed to trade":

| Concern | What owns it |
|---|---|
| Source-backed trading knowledge | Document store and KB |
| Claims, contradictions, provenance | `archon-knowledge`, `archon-provenance`, `archon-trading::kb` |
| Strategy definition | `archon-trading::spec_registry` |
| Pine Script generation/compile proof | `archon-trading::pine_lab` and TradingView MCP adapter |
| Market data registry | `archon-trading::data_lake` and OpenBB allowlist/adapter |
| Deterministic replay/backtest | `archon-trading::backtest` |
| Risk controls | `archon-trading::risk_governor`, `risk_policy`, `risk_controls` |
| Paper execution evidence | `archon-trading::paper_terminal` |
| Live execution readiness | `archon-trading::live_enablement`, `live_terminal`, `dryrun_cert` |
| Audit trail | `archon-trading::audit_ledger` |
| Learning from outcomes | `archon-trading::learning_hooks`, postmortems, memory/world-model consumers |

The LLM can propose, analyze, code, critique, and summarize. It cannot bypass
the deterministic policy gates.

## Core Modules

### Knowledge and Claims

`archon-trading::kb` provides the trading-specific evidence model:

- closed KB taxonomy for trading topics
- source-backed claims
- contradiction packets
- unsupported-media handling
- invalidation when a cited source chunk changes or disappears

Use the general document/KB commands to populate this layer:

```text
/docs ingest ./assets/research-paper/trading
/docs index
/kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
/kb search --kb trading-elliott-wave "Elliott wave invalidation rule"
```

### Strategy Spec Registry

`archon-trading::spec_registry` defines the mandatory 15-field strategy spec.
A strategy cannot advance unless the fields are present and type-valid.

| Field | Meaning |
|---|---|
| `SPEC-F01` | instrument universe |
| `SPEC-F02` | timeframe and session |
| `SPEC-F03` | market regime assumptions |
| `SPEC-F04` | data dependencies |
| `SPEC-F05` | exact entry/exit rules |
| `SPEC-F06` | indicator formulas |
| `SPEC-F07` | position sizing |
| `SPEC-F08` | stops, take-profit, trailing rules, max drawdown ceiling |
| `SPEC-F09` | invalidation rules |
| `SPEC-F10` | no-trade conditions |
| `SPEC-F11` | slippage and fee assumptions |
| `SPEC-F12` | benchmark |
| `SPEC-F13` | expected failure modes |
| `SPEC-F14` | data-quality/staleness tolerances |
| `SPEC-F15` | promotion status |

Promotion status is one-step only:

```text
idea -> research -> backtest -> paper -> live-pilot -> retired
```

### Pine Script Lab

`archon-trading::pine_lab` creates Pine Script v6 indicator and strategy
variants from approved strategy specs.

Important safety constraints:

- Pine docs must be checked before code generation.
- Scripts are stored by hash and strategy id.
- Compile status requires a compile proof tied to the script source hash.
- Pine alerts are never authoritative execution. Alerts become order intents
  and still pass the Risk Governor.
- Multi-symbol specs become multiple single-symbol Pine scripts plus one
  portfolio-level Archon spec. Cross-symbol aggregation lives in Archon, not
  Pine.

TradingView MCP is represented by `archon-trading::adapters::tv_mcp`.
The project setup script installs the concrete
[`tradesdontlie/tradingview-mcp`](https://github.com/tradesdontlie/tradingview-mcp)
package under `.archon/tools/tradingview-mcp`, adds a project-local
`.mcp.json` entry named `tradingview`, and exposes the same tools to agents as
`mcp__tradingview__<tool>`.

It has two tiers:

| Tier | Default | Examples |
|---|---|---|
| Read | enabled | docs lookup, compile check, screenshot capture, script version sync |
| Write | disabled | chart deploy, alert setup, terminal interaction |

Write-tier operations require sandbox certification and maker-checker approval.
`archon-trading::adapters::tv_paper` adds the supported paper/replay bridge:
Archon first runs the internal paper Risk Governor, then calls the TradingView
MCP replay-trade path for market buy/sell evidence. It rejects live intents and
non-market replay orders. TradingView MCP does not execute broker trades; it
controls the local TradingView Desktop chart/editor through the CDP port you
explicitly launch.

### OpenBB and Market Data

`archon-trading::adapters::openbb_allowlist` defines approved provider/data-type
pairs. `archon-trading::adapters::openbb` enforces:

- allowlisted provider/data-type combinations
- data quality gates
- licensed vs research-only evidence
- fail-closed live misses
- zero-tolerance credential/secret rejection in request parameters

`archon-trading::data_lake` stores dataset metadata:

- provider
- symbol mapping
- timezone
- adjustment mode
- license
- coverage/gaps
- checksum
- version
- required vs optional flag

Degraded optional datasets do not satisfy mandatory promotion data.
The setup script installs OpenBB into `.archon/tools/openbb-venv` and provides
`scripts/start-openbb-api.sh`, which starts `openbb-api` on `127.0.0.1:6900`
unless `OPENBB_HOST` or `OPENBB_PORT` override it.

### Backtest Harness

`archon-trading::backtest` provides deterministic replay with:

- data snapshot checksum
- config hash
- pinned numeric-library version in the hash
- fee, spread, slippage, market impact, latency, partial-fill, and unavailable
  liquidity costs
- walk-forward, out-of-sample, Monte Carlo reshuffle, parameter-stability, and
  regime-sliced robustness records
- exactly 11 report metrics

Strategy Tester output from TradingView can be stored as auxiliary evidence,
but it cannot be the sole promotion gate.

### Risk Governor

`archon-trading::risk_governor` and `risk_controls` implement deterministic
pre-trade controls. The Risk Governor can approve, reject, halt, or retire a
strategy/order intent. It owns hard checks such as:

- max order notional
- max strategy/account exposure
- symbol concentration
- daily loss
- strategy drawdown
- open order count
- order rate
- liquidity/spread/slippage
- stale data
- broker health
- leverage
- consecutive-loss halt
- cooldown
- correlated exposure
- manual approval requirements

Risk policy changes live in `archon-trading::risk_policy`. Upward risk changes
need maker-checker approval and audit logging.

### Paper Terminal

`archon-trading::paper_terminal` uses the same order-intent and governor path as
live trading, but records paper execution evidence.

The CLI can also mirror an approved paper order into TradingView replay mode:

```bash
archon trading paper tradingview-replay-submit \
  --target /path/to/project \
  --intent order-intent.json \
  --adapter-pin tradesdontlie@abcdef1 \
  --write-tier-enabled \
  --sandbox-certified \
  --approval-id tv-replay-1 \
  --maker alice \
  --checker bob \
  --rationale "sandbox replay test"
```

That command does not place a broker order. It fails closed unless the
OrderIntent is `Paper`, the order type is `Market`, the internal risk gate
accepts it, and the TradingView write-tier requirements are satisfied.

Promotion from paper requires a sample gate:

- minimum closed trades
- minimum calendar days
- minimum regimes
- postmortem-ready evidence

The sample gate reports the longest binding missing condition, so the user sees
the real bottleneck instead of a random missing field.

### Live Terminal and Live Enablement

`archon-trading::live_terminal` models broker interaction through a broker
adapter trait and immutable execution ledger rows.

`archon-trading::live_enablement` is the live-readiness gate. Live is disabled
by default and requires:

- supported jurisdiction and account identifiers
- valid risk policy hash
- kill switch validation
- production evidence
- dry-run certification report
- maker-checker approval from distinct actors
- pilot capital limits

Partial fills, rejects, cancels, and replacements are distinct immutable ledger
records. Broker rejects do not trigger automatic live retries.

### Kill Switch

`archon-trading::kill_switch` supports:

- in-app trigger
- out-of-band CLI-style trigger
- halt latency/cancel latency receipt
- fail-closed behavior if cancel transport fails

The TUI panel model exposes a kill-button path, but a full end-user trading UI
is not the same as a certified broker integration.

### Audit Ledger

`archon-trading::audit_ledger` is hash-chained and log-before-act oriented. It:

- redacts secrets before durable write
- stores maker-checker records
- stores tax/account fields
- stores artefact digests
- detects tampering
- supports strategy reconstruction

The audit ledger is the evidence spine for promotion, live enablement, and
postmortem reconstruction.

### Learning Hooks

`archon-trading::learning_hooks` lets Archon's learning systems observe trading
outcomes without letting them directly change risk limits or emit live orders.

It records:

- blocked live-limit preferences
- blocked order-like recommendations
- gated attempts missing maker-checker approval
- approved advisory-only learning events

Learning can improve research behavior and postmortem quality. It cannot
autonomously raise live risk or bypass the Risk Governor.

## Personas and Permissions

The implementation follows the PRD persona model:

| Persona | Role | Key constraint |
|---|---|---|
| `PER-01` | Human governor | final approval authority |
| `PER-02` | Quant research agent | research/data/backtest, no direct execution |
| `PER-03` | Pine Script agent | code generation and compile checks, write tier gated |
| `PER-04` | Risk agent | deterministic HALT/REJECT, no autonomous limit increases |
| `PER-05` | Execution agent | paper/live terminal path only, no KB or risk-policy writes |
| `PER-06` | Postmortem agent | lessons and failure patterns, no live-limit changes |
| `PER-07` | Compliance/audit reviewer | read-only |

The tool facade in `archon-tools/src/trading` enforces a smaller command/action
matrix for agents:

| Command | Route |
|---|---|
| `kb` | `archon_trading::kb` |
| `spec` | `archon_trading::spec_registry` |
| `pine` | `archon_trading::pine_lab` |
| `backtest` | `archon_trading::backtest` |
| `paper` | `archon_trading::paper_terminal` |
| `live` | `archon_trading::live_enablement` |
| `promote` | `archon_trading::promotion` |

The primary `/trading` command mirrors the shell `archon trading ...` surface,
including setup/status helpers, TradingView MCP CLI pass-through, Pine
generation/checks, governed OpenBB fetches, native backtests, paper-order
checks, promotion checks, live-readiness gates, dispatch fences, and
kill-switch checks.

## What You Can Do Today

Use the Trading Lab today as a source-backed research and validation workflow:

1. Ingest trading material into Docs/KB.
2. Use `/archon-research`, `/workflow`, or manual review to draft a strategy
   thesis.
3. Convert the thesis into a 15-field strategy spec.
4. Generate Pine Script indicator/strategy variants through the Pine Lab path.
5. Use approved data snapshots and the backtest harness for deterministic
   evidence.
6. Use paper-terminal evidence, optional TradingView replay evidence, and
   postmortems before promotion.
7. Evaluate live-readiness gates explicitly; broker submission remains
   fail-closed unless a certified adapter and policy allow it.

To generate a provider-neutral dynamic workflow for this lifecycle:

```bash
archon trading workflow plan \
  --idea "BTC Elliott Wave volatility-regime swing strategy" \
  --repository /Volumes/Externalwork/archon-cli/archon-cli \
  --prd /Volumes/Externalwork/archon-cli/project-1/prds/archon-trading-research-execution-lab/PRD.md \
  --tasks /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-LAB-001 \
  --kb trading-elliott-wave \
  --kb trading-risk-management \
  --tradingview-replay \
  --out /Volumes/Externalwork/archon-cli/project-1/trading-lab-workflow.yaml

archon workflow run --spec-file /Volumes/Externalwork/archon-cli/project-1/trading-lab-workflow.yaml --live
```

See the detailed cookbook: [Trading Lab cookbook](cookbook/trading-lab.md).

## Safety Rules

- Do not treat model text as a trade.
- Do not treat Pine alerts as orders.
- Do not promote research-only or degraded data as live evidence.
- Do not use TradingView Strategy Tester as sole promotion evidence.
- Do not allow learning systems to modify live risk limits.
- Do not enable live trading without dry-run certification and maker-checker
  approval.
- Do not run live execution against a real broker until the broker adapter,
  policy, logs, and kill switch have been reviewed in your environment.

## Developer Verification

Useful focused checks while changing this subsystem:

```bash
cargo fmt --check
cargo test -p archon-trading -j1 -- --test-threads=2
cargo check -p archon-tools
cargo check -p archon-tui --lib
```

The Trading Lab task set also keeps per-file line budgets. The largest
production modules are intentionally split to stay under the task caps and keep
complexity reviewable.

## See Also

- [Trading Lab cookbook](cookbook/trading-lab.md)
- [Document intelligence](docs.md)
- [Knowledge base](knowledge.md)
- [Dynamic workflows](cookbook/dynamic-workflows.md)
- [Trading and asset analysis with `/gametheory`](cookbook/trading-with-gametheory.md)
- [Governed learning](governed-learning.md)
