# Trading Lab Examples

These fixtures are copy-paste starting points for the Trading Lab command
surface. They use tiny synthetic data and no broker credentials.

Use a scratch project root when testing data-lake writes:

```bash
PROJECT=/tmp/archon-trading-lab-example
mkdir -p "$PROJECT"
```

## Validate A Strategy Spec

```bash
archon trading spec validate \
  --spec examples/trading-lab/strategy-spec.json
```

## Ingest OHLCV And Run Candle Backtests

```bash
archon trading data ingest-ohlcv \
  --target "$PROJECT" \
  --source examples/trading-lab/ohlcv-btc-1d.csv \
  --format csv \
  --dataset-id btc-1d-demo \
  --version 2026-06-06 \
  --provider manual-fixture \
  --symbol BTCUSD \
  --timezone UTC \
  --adjustment raw \
  --license research

archon trading data show \
  --target "$PROJECT" \
  --dataset-id btc-1d-demo \
  --version 2026-06-06

archon trading backtest run-ohlcv \
  --target "$PROJECT" \
  --config examples/trading-lab/backtest-config.json \
  --dataset-id btc-1d-demo \
  --version 2026-06-06 \
  --quantity 0.01 \
  --rule sma-cross \
  --fast-len 3 \
  --slow-len 5

archon trading backtest run-ohlcv \
  --target "$PROJECT" \
  --config examples/trading-lab/backtest-config.json \
  --dataset-id btc-1d-demo \
  --version 2026-06-06 \
  --quantity 0.01 \
  --strategy-rules examples/trading-lab/strategy-rules.json
```

## Run Fill-Based Backtest

```bash
archon trading backtest run \
  --config examples/trading-lab/backtest-config.json \
  --fills examples/trading-lab/fills.json \
  --dataset-status healthy \
  --source native-harness
```

## Generate Pine

```bash
archon trading pine generate \
  --strategy-id btc-demo \
  --spec examples/trading-lab/strategy-spec.json \
  --out "$PROJECT/pine"
```

## Paper Gate

```bash
archon trading paper submit \
  --intent examples/trading-lab/paper-order-intent.json \
  --account examples/trading-lab/paper-account.json \
  --market examples/trading-lab/paper-market.json

archon trading paper sample \
  --sample examples/trading-lab/paper-sample.json
```

## Promotion Gate

```bash
archon trading promote check \
  --spec examples/trading-lab/strategy-spec.json \
  --target backtest \
  --evidence examples/trading-lab/promotion-evidence.json
```

## OpenBB Request Shapes

These files show the JSON shape expected by `archon trading openbb fetch`.
They require a running OpenBB API for a live fetch:

```bash
archon trading openbb fetch \
  --request examples/trading-lab/openbb-request.json \
  --metadata examples/trading-lab/openbb-metadata.json \
  --quality examples/trading-lab/openbb-quality.json \
  --mode research \
  --store-ohlcv \
  --response-format json
```

## Live Readiness Is Fail-Closed

`live-enable-request.blocked.json` intentionally uses a stale policy hash so
users can see the gate reject instead of accidentally blessing a live path.

```bash
archon trading live enable-check \
  --request examples/trading-lab/live-enable-request.blocked.json
```

For real live readiness, generate a fresh policy hash through Archon's risk
policy path, attach a real dry-run certification report, and keep
maker/checker actors distinct.
