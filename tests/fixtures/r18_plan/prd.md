# Product Requirements Document: Trading Data Lake v1 and AHDM-v1

**PRD ID**: PRD-TRADING-DATA-LAKE-AHDM-001
**Version**: 0.1
**Date**: 2026-06-09
**Owner**: Steve / Archon
**Status**: Draft
**Primary mode**: Data-first research and paper-trading validation

---

## 1. Executive Summary

Archon must build a real trading data lake before attempting AHDM-v1, the
Archon Hybrid Directional Model. AHDM-v1 cannot be credible until historical
and current OHLCV data is persisted, versioned, validated, and traceable to
provider-native candle intervals.

The current Trading Lab has a small OHLCV persistence layer under the project
directory. This PRD upgrades that store into Trading Data Lake v1 and then uses
it to build AHDM-v1: a cited, backtested, paper-trading-only daily-bias strategy
system with Pine v6 indicator/strategy artifacts and Archon-native backtests.

Core rule: production candles must be native provider candles. Archon must not
derive, aggregate, downsample, upsample, or resample production candles because
provider session rules, timezones, rollovers, adjusted pricing, and volume
aggregation can differ from TradingView, Polygon, Stooq, yfinance, and broker
data.

---

## 2. Existing Anchor

Trading Data Lake v1 must extend the existing project-local Trading Lab store.
It must not create a second vague storage location.

Current project path:

```text
/Volumes/Externalwork/archon-cli/project-1/.archon/trading-lab/data
```

Current layout:

```text
.archon/trading-lab/data/registry.json
.archon/trading-lab/data/datasets/<dataset-id>/<version>/metadata.json
.archon/trading-lab/data/datasets/<dataset-id>/<version>/ohlcv.jsonl
.archon/trading-lab/data/datasets/<dataset-id>/<version>/raw.csv
```

Current code anchors:

- `crates/archon-trading/src/data_lake.rs`
- `crates/archon-trading/src/data_store.rs`
- `src/command/trading_data.rs`

Current limitations:

- The project registry currently has zero datasets.
- Manual OHLCV ingest exists, but provider-native ingestion is incomplete.
- Coverage matrix, provider capability checks, current snapshots, strong
  validation reports, futures metadata, and no-resampling production gates are
  missing.

---

## 3. Goals

| ID | Goal |
|---|---|
| G-DL-001 | Upgrade the existing OHLCV store into Trading Data Lake v1. |
| G-DL-002 | Ingest native candles from TradingView MCP, OpenBB/Polygon, Stooq, and yfinance fallback. |
| G-DL-003 | Persist raw provider artifacts and normalized OHLCV datasets with replayable version metadata. |
| G-DL-004 | Produce a coverage matrix for ES, NQ, SPY, QQQ, BTCUSDT, and ETHUSDT across required native timeframes. |
| G-DL-005 | Refuse production backtests on derived, unvalidated, degraded, or metadata-incomplete datasets. |
| G-AHDM-001 | Build AHDM-v1 only after the data lake and coverage gates exist. |
| G-AHDM-002 | Extract cited strategy rules from trading KBs before generating strategy logic. |
| G-AHDM-003 | Produce StrategySpec, Pine v6 indicator, Pine v6 strategy, Archon-native backtest, and adversarial review. |

---

## 4. Non-Goals

| ID | Non-goal |
|---|---|
| NG-DL-001 | No production candle resampling or timeframe derivation. |
| NG-DL-002 | No hidden reliance on loose CSV files outside the registry. |
| NG-DL-003 | No claim that Stooq, yfinance, TradingView, or Polygon candles are interchangeable. |
| NG-DL-004 | No live trading enablement in this PRD. |
| NG-DL-005 | No high-probability prediction claim until backtests and paper trading prove it. |
| NG-DL-006 | No overbroad rewrite of Trading Lab unrelated to data lake, strategy spec, Pine, backtest, and readiness gates. |

---

## 5. Best Build Strategy

Build this in two milestones, not as one giant strategy workflow.

Milestone 1: Trading Data Lake v1

1. Extend the existing store schema and paths.
2. Add native-interval metadata and validation gates.
3. Add provider capability checks.
4. Add provider adapters one at a time.
5. Add coverage matrix commands.
6. Add backtest refusal gates for invalid data.

Milestone 2: AHDM-v1

1. Extract KB evidence with citations.
2. Create the StrategySpec.
3. Generate Pine v6 indicator and strategy artifacts.
4. Implement Archon-native backtest rules using registered datasets only.
5. Run focused tests and sample backtests.
6. Produce adversarial review and paper-trading readiness report.

This order prevents strategy work from inventing results before data coverage
and candle provenance are real.

---

## 6. Data Universe

Required instruments:

| Canonical ID | Primary provider symbols | Asset class | Notes |
|---|---|---|---|
| ES | `CME_MINI:ES1!`, Polygon/OpenBB supported symbol if available | Future | Continuous-contract metadata required. |
| NQ | `CME_MINI:NQ1!`, Polygon/OpenBB supported symbol if available | Future | Continuous-contract metadata required. |
| SPY | `SPY`, `SPY.US` | Equity ETF | Corporate action/adjustment metadata required. |
| QQQ | `QQQ`, `QQQ.US` | Equity ETF | Corporate action/adjustment metadata required. |
| BTCUSDT | `BINANCE:BTCUSDT`, provider-supported crypto symbol | Crypto | 24/7 session metadata required. |
| ETHUSDT | `BINANCE:ETHUSDT`, provider-supported crypto symbol | Crypto | 24/7 session metadata required. |

Required native timeframes:

| ID | Timeframe | Production rule |
|---|---|---|
| TF-001 | 1W | Must be fetched as native weekly candles. |
| TF-002 | 1D | Must be fetched as native daily candles. |
| TF-003 | 4H / 240 | Must be fetched as native 4-hour candles. |
| TF-004 | 1H / 60 | Must be fetched as native hourly candles. |
| TF-005 | 15m | Must be fetched as native 15-minute candles. |

Diagnostic resampling is allowed only when explicitly marked
`production_eligible=false` and excluded from promotion/backtests.

---

## 7. Provider Priority

| Priority | Provider | Intended use | Rule |
|---|---|---|---|
| P1 | TradingView MCP | Chart-equivalent current candles, chart snapshots, Pine validation, futures chart symbols | Use exact native timeframe only. |
| P2 | OpenBB/Polygon | Historical OHLCV where exact native interval is available | Use as primary historical source when credentials and instrument coverage exist. |
| P3 | Stooq | Free historical baseline for exact native intervals only | Do not resample Stooq data. If blocked or unavailable, record failure honestly. |
| P4 | yfinance | Degraded fallback | Mark degraded/fallback and require review before paper-trading evidence. |

Provider adapters must be provider-neutral. Adding a future broker/data vendor
must not require changing the registry schema.

---

## 8. Data Lake Requirements

### 8.1 Storage

- REQ-DL-001: Trading Data Lake v1 must use the existing root
  `.archon/trading-lab/data`.
- REQ-DL-002: Raw provider responses must be stored under each dataset version.
- REQ-DL-003: Normalized OHLCV must be stored as replayable JSONL.
- REQ-DL-004: Registry entries must be content-addressed by dataset id and
  version.
- REQ-DL-005: Dataset writes must be atomic enough that interrupted ingests do
  not leave a healthy registry entry pointing at missing artifacts.

### 8.2 Metadata

- REQ-DL-010: Metadata must include dataset id, version, canonical instrument,
  provider, provider symbol, asset class, timeframe, native interval flag,
  session, timezone, adjustment policy, row count, start/end timestamp,
  checksum, raw artifact paths, normalized path, ingestion timestamp, source
  notes, license notes, and production eligibility.
- REQ-DL-011: Futures datasets must include continuous-contract notes, rollover
  policy if known, and provider symbol semantics.
- REQ-DL-012: Equity ETF datasets must record adjusted/unadjusted policy and
  corporate-action limitations.
- REQ-DL-013: Crypto datasets must record 24/7 session assumptions and exchange
  source.

### 8.3 Native Interval Enforcement

- REQ-DL-020: Production datasets must set `native_interval=true`.
- REQ-DL-021: Any derived/resampled dataset must set
  `production_eligible=false`.
- REQ-DL-022: Backtests and StrategySpec promotion gates must reject production
  use of `native_interval=false`.
- REQ-DL-023: Provider adapters must return explicit unavailable reasons when
  an exact native interval is unsupported.

### 8.4 Provider Adapters

- REQ-DL-030: Implement `can_fetch_symbol_timeframe(provider, symbol,
  timeframe)`.
- REQ-DL-031: Implement `fetch_ohlcv_native(provider, symbol, timeframe, start,
  end)`.
- REQ-DL-032: Implement `fetch_current_snapshot(provider, symbol)` when the
  provider supports current data.
- REQ-DL-033: Implement TradingView MCP native OHLCV and current snapshot
  ingestion where the MCP exposes those actions.
- REQ-DL-034: Implement OpenBB/Polygon native OHLCV ingestion where OpenBB and
  credentials support the instrument/timeframe.
- REQ-DL-035: Implement Stooq exact-native interval ingestion or clear
  unavailable handling. Browser verification or access blocking must not be
  bypassed; it must be reported.
- REQ-DL-036: Implement yfinance fallback ingestion with degraded metadata.

### 8.5 Coverage Matrix

- REQ-DL-040: Add a coverage matrix for every required instrument/timeframe.
- REQ-DL-041: Each cell must show selected provider, provider symbol, native
  interval status, available/unavailable, row count, start/end, quality status,
  and fallback reason.
- REQ-DL-042: Coverage reports must be exportable as JSON and readable text.

### 8.6 Validation

- REQ-DL-050: Validate monotonic timestamps.
- REQ-DL-051: Validate duplicate timestamps.
- REQ-DL-052: Validate OHLC fields and OHLC sanity.
- REQ-DL-053: Validate volume presence where expected.
- REQ-DL-054: Validate large gaps and coverage minimums.
- REQ-DL-055: Validate required metadata completeness.
- REQ-DL-056: Produce a dataset quality report per dataset version.

### 8.7 Commands

- REQ-DL-060: Add or extend CLI/TUI commands for `data status`, `data list`,
  `data show`, `data ingest-ohlcv`, `data validate`, `data coverage`,
  `data providers`, `data fetch-native`, `data snapshot`, and `data export`.
- REQ-DL-061: Commands must support `--target` for project root selection.
- REQ-DL-062: Commands must fail with exact provider/action/dataset details.

---

## 9. AHDM-v1 Requirements

- REQ-AHDM-001: AHDM-v1 must not start strategy validation until the data lake
  can report coverage for required instruments and timeframes.
- REQ-AHDM-002: AHDM-v1 must extract cited rules from KBs first.
- REQ-AHDM-003: KB evidence must prioritize `trading-hybrid-system`,
  `trading-market-structure`, `trading-risk-management`, `trading-backtesting`,
  `trading-execution`, `trading-strategy-research`, and
  `trading-postmortems`.
- REQ-AHDM-004: `trading-elliott-wave` is secondary and non-authoritative.
- REQ-AHDM-005: StrategySpec must include instrument universe, timeframe stack,
  required datasets, daily bias formula, confidence scoring, entry models,
  invalidation/stop logic, TP1/TP2/TP3, no-trade filters, position sizing,
  slippage/cost assumptions, data quality gates, promotion gates, and
  paper-trading readiness gates.
- REQ-AHDM-006: Pine v6 indicator must show bias, confidence, entry zone, stop,
  TP1/TP2/TP3, no-trade state, and sizing hint.
- REQ-AHDM-007: Pine v6 strategy must encode the same rules for chart-native
  validation, while promotion remains Archon-native.
- REQ-AHDM-008: Archon-native backtests must consume registered validated
  OHLCV datasets only.
- REQ-AHDM-009: AHDM-v1 must not enable live trading.
- REQ-AHDM-010: AHDM-v1 must not claim high-probability prediction until
  backtests and paper trading prove it.

---

## 10. Backtest Requirements

- REQ-BT-001: Backtests must record dataset ids, versions, providers,
  timeframes, strategy config hash, trades, PnL, drawdown, win rate,
  expectancy, costs, slippage, and equity curve artifact.
- REQ-BT-002: Backtests must refuse unvalidated, degraded, non-native, or
  metadata-incomplete datasets unless a diagnostic override is explicit.
- REQ-BT-003: Diagnostic overrides must never satisfy promotion gates.
- REQ-BT-004: Backtest reports must be replayable from registered artifacts.

---

## 11. Engineering Constraints

- NFR-001: Keep changed/new files under 500 lines.
- NFR-002: Keep new function cyclomatic complexity <= 15.
- NFR-003: Use provider-neutral interfaces; do not hard-code one provider as
  the only future source.
- NFR-004: Use focused tests only during implementation; do not run the full
  heavy suite unless explicitly requested.
- NFR-005: Persist enough evidence for adversarial review.
- NFR-006: No secrets or provider credentials may be stored in dataset
  artifacts.

---

## 12. Acceptance Criteria

| ID | Acceptance criterion |
|---|---|
| AC-DL-001 | `trading data status` shows the existing project data root and registry. |
| AC-DL-002 | A provider capability command reports exact native support or unavailable reason per provider/symbol/timeframe. |
| AC-DL-003 | Native OHLCV ingestion stores raw artifact, normalized JSONL, metadata, validation report, and registry entry. |
| AC-DL-004 | Coverage matrix reports all required instruments and timeframes. |
| AC-DL-005 | Derived/resampled datasets are marked non-production and rejected by production backtests. |
| AC-DL-006 | Invalid timestamps, duplicate bars, bad OHLC, missing metadata, and missing volume fail validation. |
| AC-DL-007 | OpenBB/Polygon, TradingView MCP, Stooq, and yfinance fallback paths either ingest valid native data or report exact unavailable reasons. |
| AC-AHDM-001 | StrategySpec AHDM-v1 references registered dataset ids/versions. |
| AC-AHDM-002 | Pine v6 indicator and strategy artifacts are generated and stored. |
| AC-AHDM-003 | Archon-native backtest runs only on validated native datasets. |
| AC-AHDM-004 | Adversarial review lists KB, data, provider, overfitting, slippage, and paper-readiness risks. |

---

## 13. Focused Tests

Required focused tests:

- data lake registry schema and atomic write behavior
- OHLCV parse and metadata persistence
- native interval enforcement
- provider capability result parsing
- coverage matrix generation
- quality validation failures
- StrategySpec dataset references
- position sizing and risk calculations
- backtest refusal of invalid datasets
- Pine artifact generation

---

## 14. Implementation Phases

| Phase | Scope | Exit gate |
|---|---|---|
| 0 | Inspect existing store, docs, commands, providers, and tests. | Gap report produced. |
| 1 | Upgrade schema/storage and migration behavior. | Registry can read old and new entries. |
| 2 | Add validation reports and native interval gates. | Invalid fixtures fail focused tests. |
| 3 | Add provider capability and native fetch adapters. | Each provider reports support/unavailable honestly. |
| 4 | Add coverage matrix and project commands. | Matrix works for required universe. |
| 5 | Wire backtest refusal gates. | Backtests reject bad/non-native datasets. |
| 6 | Extract KB evidence and build AHDM-v1 StrategySpec. | Cited rule inventory exists. |
| 7 | Generate Pine v6 indicator/strategy. | Pine artifacts pass local structural checks. |
| 8 | Run sample backtests and adversarial review. | Readiness report lists pass/fail and gaps. |

---

## 15. Decomposition Guidance

Recommended task decomposition:

1. `T001-data-lake-gap-audit`
2. `T010-registry-schema-v1`
3. `T020-ohlcv-validation-reports`
4. `T030-provider-capability-interface`
5. `T040-tradingview-mcp-native-ingest`
6. `T050-openbb-polygon-native-ingest`
7. `T060-stooq-native-ingest-or-unavailable`
8. `T070-yfinance-fallback-ingest`
9. `T080-coverage-matrix-command`
10. `T090-backtest-data-gates`
11. `T100-ahdm-kb-evidence-inventory`
12. `T110-ahdm-strategy-spec`
13. `T120-pine-v6-artifacts`
14. `T130-ahdm-native-backtest`
15. `T140-adversarial-review-and-readiness`

---

## 16. Resolved Decisions

- RD-001: The data lake root is `.archon/trading-lab/data`.
- RD-002: Production candles must be native provider candles.
- RD-003: Derived/resampled candles are diagnostic only.
- RD-004: TradingView MCP is preferred for chart-equivalent validation.
- RD-005: OpenBB/Polygon is preferred for historical provider-backed data where
  exact intervals are available.
- RD-006: Stooq is a baseline provider only for exact native intervals.
- RD-007: yfinance is a degraded fallback only.
- RD-008: AHDM-v1 must be built after data lake gates, not before.

---

## 17. Normative Storage Contract

Trading Data Lake v1 uses filesystem artifacts, not Cozo, RocksDB, or the
document-index vector store, for candle data. This keeps market data replayable,
portable, inspectable, and isolated from KB/document storage.

Root:

```text
<project>/.archon/trading-lab/data
```

Required layout:

```text
.archon/trading-lab/data/
  registry.json
  provider-capabilities.json
  coverage/
    latest.json
    latest.md
    history/<timestamp>.json
  datasets/
    <dataset-id>/
      <version>/
        metadata.json
        validation.json
        manifest.json
        ohlcv.jsonl
        raw/
          response.<json|csv|zip|txt>
          request.json
          headers.redacted.json
          provider-notes.md
  snapshots/
    <provider>/
      <canonical-instrument>/
        <timestamp>.json
  runs/
    <backtest-run-id>/
      config.json
      report.json
      trades.jsonl
      equity_curve.jsonl
      postmortem.md
```

Registry file:

```text
.archon/trading-lab/data/registry.json
```

Registry schema version:

```json
{
  "schema_version": "archon-trading-data-registry-v2",
  "datasets": {},
  "snapshots": {},
  "last_updated": "RFC3339"
}
```

The existing `archon-trading-data-registry-v1` registry must remain readable.
The first write after this PRD is implemented must migrate it to v2 while
preserving all existing dataset records.

---

## 18. Dataset Naming Contract

Dataset IDs must be deterministic and human-readable.

Pattern:

```text
<provider>-<canonical-instrument>-<timeframe>-<price-basis>
```

Examples:

```text
tradingview-ES-1D-raw
tradingview-NQ-240-raw
polygon-SPY-15-raw
stooq-QQQ-1D-raw
yfinance-BTCUSDT-1H-raw
```

Version pattern:

```text
<YYYYMMDD>-<provider-run-id-or-short-hash>
```

Examples:

```text
20260609-7f3a2c9b
20260609-tv-session-a12d
```

Rules:

- REQ-DL-070: Dataset IDs must not contain spaces.
- REQ-DL-071: Dataset IDs must encode provider, instrument, timeframe, and
  price basis.
- REQ-DL-072: Versions must change when raw provider data changes.
- REQ-DL-073: Re-ingesting identical raw content may reuse the existing version
  only when checksum matches exactly.
- REQ-DL-074: Dataset IDs must be stable across machines and sessions.

---

## 19. Dataset Metadata Schema

Each dataset version must write `metadata.json` with this minimum shape:

```json
{
  "schema_version": "archon-trading-dataset-v1",
  "dataset_id": "tradingview-ES-1D-raw",
  "version": "20260609-7f3a2c9b",
  "canonical_instrument": "ES",
  "asset_class": "future",
  "provider": "tradingview",
  "provider_symbol": "CME_MINI:ES1!",
  "timeframe": "1D",
  "native_interval": true,
  "production_eligible": true,
  "price_basis": "raw",
  "timezone": "America/New_York",
  "session": "provider_default",
  "adjustment": "raw",
  "continuous_contract": {
    "is_continuous": true,
    "provider_symbol_semantics": "front continuous",
    "roll_policy": "provider_defined",
    "notes": "TradingView CME_MINI:ES1! semantics"
  },
  "coverage": {
    "start": "RFC3339",
    "end": "RFC3339",
    "row_count": 0,
    "expected_bars": null,
    "missing_bars": null,
    "gap_count": 0,
    "duplicate_timestamp_count": 0
  },
  "checksums": {
    "raw_sha256": "hex",
    "normalized_sha256": "hex",
    "metadata_sha256": "hex"
  },
  "paths": {
    "raw": "relative/path",
    "normalized": "relative/path",
    "validation": "relative/path",
    "manifest": "relative/path"
  },
  "source": {
    "license_notes": "research use; provider terms apply",
    "url_or_endpoint": "redacted-safe endpoint or provider action",
    "retrieved_at": "RFC3339",
    "credential_required": false
  },
  "quality_status": "pending|passed|degraded|failed",
  "created_at": "RFC3339"
}
```

Rules:

- REQ-DL-080: Metadata must be JSON and deterministic where possible.
- REQ-DL-081: Paths stored in metadata must be relative to project root.
- REQ-DL-082: Secrets, API keys, bearer tokens, cookies, and session IDs must
  never be written to metadata, raw headers, validation reports, or manifests.
- REQ-DL-083: A missing `native_interval` field means `false`.
- REQ-DL-084: A missing `production_eligible` field means `false`.
- REQ-DL-085: Existing v1 metadata must be upgraded in memory when read, but
  only rewritten during explicit migration or dataset update.

---

## 20. Normalized OHLCV Schema

Normalized candles must be JSONL, one candle per line:

```json
{
  "ts": "RFC3339",
  "open": 100.0,
  "high": 101.0,
  "low": 99.5,
  "close": 100.5,
  "volume": 12345.0,
  "provider_ts": "optional original timestamp",
  "provider_extra": {}
}
```

Rules:

- REQ-DL-090: `ts`, `open`, `high`, `low`, and `close` are required.
- REQ-DL-091: `volume` is required for production datasets except when the
  provider proves the instrument/timeframe has no volume field.
- REQ-DL-092: `ts` must be normalized to RFC3339 with timezone offset or UTC Z.
- REQ-DL-093: Normalized candles must be sorted ascending by `ts`.
- REQ-DL-094: Duplicate timestamps fail validation for production datasets.
- REQ-DL-095: Non-finite numeric values fail validation.
- REQ-DL-096: Negative prices fail validation.
- REQ-DL-097: Negative volume fails validation.
- REQ-DL-098: `high < low`, `open` outside `[low, high]`, or `close` outside
  `[low, high]` fail validation.

---

## 21. Validation Report Schema

Each dataset version must write `validation.json`:

```json
{
  "schema_version": "archon-trading-validation-v1",
  "dataset_id": "tradingview-ES-1D-raw",
  "version": "20260609-7f3a2c9b",
  "status": "passed|degraded|failed",
  "native_interval": true,
  "production_eligible": true,
  "checks": [
    {
      "id": "ohlcv.monotonic_timestamps",
      "status": "passed",
      "severity": "error",
      "message": "timestamps are strictly ascending"
    }
  ],
  "summary": {
    "row_count": 0,
    "duplicate_timestamp_count": 0,
    "gap_count": 0,
    "bad_ohlc_count": 0,
    "missing_volume_count": 0
  },
  "validated_at": "RFC3339"
}
```

Validation severities:

| Severity | Production behavior |
|---|---|
| error | Dataset fails production eligibility. |
| warning | Dataset may be degraded and requires explicit review. |
| info | Informational only. |

Rules:

- REQ-DL-100: A dataset with any `error` check must have `status=failed`.
- REQ-DL-101: A dataset with warnings but no errors must have
  `status=degraded`.
- REQ-DL-102: A dataset with no warnings or errors must have `status=passed`.
- REQ-DL-103: Production backtests may use only `status=passed` unless a
  diagnostic override is explicit.

---

## 22. Provider Capability Schema

Provider capability checks must return this structure:

```json
{
  "provider": "tradingview",
  "canonical_instrument": "ES",
  "provider_symbol": "CME_MINI:ES1!",
  "timeframe": "1D",
  "can_fetch": true,
  "native_interval": true,
  "current_snapshot_supported": true,
  "historical_supported": true,
  "requires_credentials": false,
  "unavailable_reason": null,
  "checked_at": "RFC3339"
}
```

Unavailable example:

```json
{
  "provider": "stooq",
  "canonical_instrument": "ES",
  "provider_symbol": "ES",
  "timeframe": "240",
  "can_fetch": false,
  "native_interval": false,
  "current_snapshot_supported": false,
  "historical_supported": false,
  "requires_credentials": false,
  "unavailable_reason": "exact native 4H futures candles unavailable or blocked",
  "checked_at": "RFC3339"
}
```

Rules:

- REQ-DL-110: Capability checks must not perform expensive full downloads.
- REQ-DL-111: Capability checks may perform a small probe when needed.
- REQ-DL-112: Any provider block, 401, 403, 404, missing credential, or missing
  exact interval must be represented as `can_fetch=false`.
- REQ-DL-113: Provider blocks must not be bypassed.

---

## 23. Coverage Matrix Schema

Coverage matrix JSON:

```json
{
  "schema_version": "archon-trading-coverage-v1",
  "generated_at": "RFC3339",
  "instruments": ["ES", "NQ", "SPY", "QQQ", "BTCUSDT", "ETHUSDT"],
  "timeframes": ["1W", "1D", "240", "60", "15"],
  "cells": [
    {
      "canonical_instrument": "ES",
      "timeframe": "1D",
      "selected_provider": "tradingview",
      "provider_symbol": "CME_MINI:ES1!",
      "dataset_id": "tradingview-ES-1D-raw",
      "version": "20260609-7f3a2c9b",
      "available": true,
      "native_interval": true,
      "production_eligible": true,
      "quality_status": "passed",
      "row_count": 5000,
      "coverage_start": "RFC3339",
      "coverage_end": "RFC3339",
      "fallback_reason": null
    }
  ],
  "gaps": [
    {
      "canonical_instrument": "NQ",
      "timeframe": "15",
      "reason": "no provider returned native 15m history"
    }
  ]
}
```

Provider selection order:

1. Use an existing passed dataset in the registry when it is fresh enough.
2. Prefer TradingView MCP for chart-equivalent futures/current chart data.
3. Prefer OpenBB/Polygon for historical data when exact native interval exists.
4. Use Stooq only for exact native baseline data.
5. Use yfinance only as degraded fallback.

Freshness defaults:

- Daily/weekly historical datasets: stale after 7 days.
- Intraday historical datasets: stale after 24 hours.
- Current snapshots: stale after 5 minutes.

These defaults are resolved decisions for v1. Changing them requires a config
schema update or follow-on PRD; v1 must not leave them undefined.

---

## 24. CLI and TUI Command Contract

The CLI and TUI slash surfaces must expose the same capabilities.

Required CLI commands:

```text
archon trading data status [--target <PROJECT>]
archon trading data list [--target <PROJECT>] [--json]
archon trading data show --dataset-id <ID> --version <VERSION> [--target <PROJECT>]
archon trading data ingest-ohlcv --source <PATH> --format csv|json --dataset-id <ID> --version <VERSION> --provider <PROVIDER> --symbol <SYMBOL> [--target <PROJECT>]
archon trading data validate --dataset-id <ID> --version <VERSION> [--target <PROJECT>]
archon trading data providers [--target <PROJECT>] [--json]
archon trading data capability --provider <PROVIDER> --symbol <SYMBOL> --timeframe <TF> [--target <PROJECT>]
archon trading data fetch-native --provider <PROVIDER> --symbol <SYMBOL> --timeframe <TF> --start <RFC3339|YYYY-MM-DD> --end <RFC3339|YYYY-MM-DD> --dataset-id <ID> [--target <PROJECT>]
archon trading data snapshot --provider <PROVIDER> --symbol <SYMBOL> [--target <PROJECT>]
archon trading data coverage --universe trading-core-v1 [--target <PROJECT>] [--json]
archon trading data export --dataset-id <ID> --version <VERSION> --out <PATH> [--target <PROJECT>]
```

Required TUI aliases:

```text
/trading data status
/trading data list
/trading data show ...
/trading data validate ...
/trading data providers
/trading data capability ...
/trading data fetch-native ...
/trading data snapshot ...
/trading data coverage ...
/trading data export ...
```

Rules:

- REQ-DL-120: Commands must print the project data root in `status`.
- REQ-DL-121: Commands must use `--target` consistently.
- REQ-DL-122: Commands that write data must print the dataset id, version,
  validation status, and artifact paths.
- REQ-DL-123: Commands must return non-zero on failed validation unless an
  explicit diagnostic flag is supplied.
- REQ-DL-124: TUI commands must not hide provider errors behind generic failure
  text.

---

## 25. Provider-Specific Contracts

### 25.1 TradingView MCP

TradingView MCP is used for chart-equivalent candles, current chart state,
Pine compile/checks, and visual validation.

Required behavior:

- Use MCP status/state before fetch.
- Use provider symbols exactly as TradingView expects.
- Use exact timeframe values: `1W`, `1D`, `240`, `60`, `15`.
- Store current chart snapshots separately from OHLCV history.
- If MCP returns only a limited number of bars, store the actual coverage and
  mark remaining requested range as unavailable, not synthesized.
- If TradingView requires a visible chart/session, return an actionable error.

TradingView MCP must not be treated as an institutional data vendor. It is
chart-equivalent research and validation data.

### 25.2 OpenBB/Polygon

OpenBB/Polygon is the preferred historical data path when exact native interval
data is available.

Required behavior:

- Detect missing Polygon credentials before attempting full fetch.
- Store OpenBB request, response, and provider metadata with secrets redacted.
- Record whether Polygon returned adjusted or unadjusted data.
- Record exchange/session assumptions when exposed.
- Do not use OpenBB/yfinance data as Polygon data.

### 25.3 Stooq

Stooq is a free baseline provider.

Required behavior:

- Use only exact native intervals Stooq directly supplies.
- Do not derive weekly from daily, 4H from hourly, or 15m from 5m.
- If the site returns JavaScript verification, 403, 404, or non-data HTML,
  report `provider_blocked_or_unavailable`.
- Do not implement bot-detection bypass.
- Store Stooq data as `quality=baseline`.

### 25.4 yfinance

yfinance is a degraded fallback.

Required behavior:

- Always mark provider as `yfinance`.
- Always mark quality as `degraded` unless a committed validation policy
  explicitly upgrades a dataset class.
- Record Yahoo symbol and interval limitations.
- Do not use yfinance datasets for promotion gates unless a human explicitly
  approves diagnostic use.

---

## 26. Migration Plan

Migration from current v1 store:

1. Read `.archon/trading-lab/data/registry.json`.
2. If missing, create v2 registry with no datasets.
3. If schema is `archon-trading-data-registry-v1`, preserve all records.
4. For each existing dataset:
   - load `metadata.json`
   - infer missing fields conservatively
   - set `native_interval=false` when unknown
   - set `production_eligible=false` when unknown
   - set `quality_status=degraded` when no validation report exists
   - write `validation.json` with migration warnings
5. Write a backup:

```text
.archon/trading-lab/data/registry.json.backup-<timestamp>
```

6. Write v2 registry atomically.

Rules:

- REQ-DL-130: Migration must never delete raw or normalized dataset files.
- REQ-DL-131: Unknown native interval status must fail closed.
- REQ-DL-132: Migration must be idempotent.
- REQ-DL-133: Migration must report counts: migrated, skipped, degraded,
  failed.

---

## 27. AHDM-v1 Artifact Contract

AHDM-v1 artifacts must live under:

```text
.archon/trading-lab/strategies/AHDM-v1/
```

Required layout:

```text
.archon/trading-lab/strategies/AHDM-v1/
  strategy-spec.json
  evidence/
    kb-rule-inventory.md
    citations.json
  pine/
    AHDM-v1-indicator.pine
    AHDM-v1-strategy.pine
    compile-report.json
  backtests/
    <run-id>/
      config.json
      report.json
      trades.jsonl
      equity_curve.jsonl
      adversarial-review.md
  readiness/
    paper-trading-readiness.md
```

StrategySpec required fields:

1. instrument universe
2. timeframe stack
3. required datasets
4. daily bias formula
5. confidence scoring
6. entry models
7. invalidation logic
8. stop logic
9. TP1/TP2/TP3 logic
10. no-trade filters
11. position sizing
12. slippage/cost assumptions
13. data quality gates
14. promotion gates
15. paper-trading readiness gates
16. source citations
17. residual gaps

Rules:

- REQ-AHDM-020: Every strategy rule must cite KB evidence or be marked
  `hypothesis`.
- REQ-AHDM-021: Hypotheses may be backtested but cannot be promoted until
  evidence is attached.
- REQ-AHDM-022: Pine and Archon-native rules must share a rule manifest so
  divergence can be reviewed.
- REQ-AHDM-023: Pine Strategy Tester results are exploratory only.
- REQ-AHDM-024: Archon-native backtests are the promotion evidence lane.

---

## 28. AHDM-v1 Initial Model Definition

AHDM-v1 is not a magic predictor. It is a governed daily-bias and trade-plan
model. The initial implementation must be deterministic and explainable.

Minimum daily-bias components:

| Component | Weight | Required evidence |
|---|---:|---|
| Higher-timeframe trend/regime | 20 | data + KB citation |
| Liquidity location / prior highs-lows | 20 | data + KB citation |
| Vector/volume behavior where available | 20 | data + KB citation |
| ADR/AWR/range state | 15 | data + KB citation |
| VWAP/EMA relationship | 10 | data + KB citation or hypothesis |
| Session timing / macro filter | 10 | KB citation or configured filter |
| Recent postmortem penalty | 5 | postmortem evidence |

Initial entry models:

1. Liquidity sweep reversal.
2. Trend continuation pullback.
3. Range mean reversion.

Initial output:

```json
{
  "bias": "long|short|neutral",
  "confidence": 0.0,
  "trade_mode": "reversal|continuation|range|no_trade",
  "entry_zone": {"low": 0.0, "high": 0.0},
  "stop": 0.0,
  "tp1": 0.0,
  "tp2": 0.0,
  "tp3": 0.0,
  "position_size": 0.0,
  "reason_codes": []
}
```

Rules:

- REQ-AHDM-030: Confidence is a score, not a probability claim.
- REQ-AHDM-031: `confidence >= 0.70` may allow paper-trading consideration
  only after backtest gates pass.
- REQ-AHDM-032: `confidence < 0.55` must output `no_trade`.
- REQ-AHDM-033: Daily direction cannot be marketed as high probability until
  out-of-sample and paper-trading evidence proves it.

---

## 29. Backtest Gate Details

Backtests must fail closed when any of these are true:

- dataset missing
- dataset validation missing
- validation status failed
- native interval false
- production eligible false
- metadata missing provider/timeframe/symbol/session/adjustment
- checksum mismatch
- normalized file missing
- raw artifact missing
- strategy references loose file path instead of dataset id/version

Diagnostic override:

```text
--diagnostic-allow-degraded-data
```

Diagnostic override behavior:

- allowed only for exploratory reports
- report must be marked `diagnostic=true`
- report cannot satisfy promotion gates
- report must list every overridden dataset issue

---

## 30. Task Acceptance Checklist

Every implementation task produced from this PRD must include:

- scope
- files expected to change
- files forbidden to change
- acceptance criteria
- focused tests
- line-count check
- complexity check where applicable
- adversarial review notes

No task may say "investigate later" without creating an explicit residual gap
artifact and a fail-closed behavior.

---

## 31. Done Definition

This PRD is done only when:

1. Existing data root is upgraded, not replaced.
2. v1 registry migration is implemented.
3. Dataset metadata includes native/provenance fields.
4. Validation reports exist.
5. Provider capability checks exist.
6. Native fetch exists or honest unavailable exists for all four providers.
7. Coverage matrix exists.
8. Backtests reject invalid/non-native datasets.
9. AHDM-v1 StrategySpec exists and references registered datasets.
10. Pine v6 indicator and strategy exist.
11. Archon-native backtest exists.
12. Focused tests pass.
13. Adversarial review exists.
14. Residual gaps are explicit and fail closed.

---

## 32. Explicit Residual-Gap Policy

Residual gaps are allowed only if they are explicit, scoped, and fail closed.

Residual-gap record:

```json
{
  "id": "GAP-DL-001",
  "area": "provider|data|strategy|pine|backtest|paper",
  "description": "exact missing capability",
  "impact": "what cannot be trusted",
  "fail_closed_behavior": "what Archon refuses to do",
  "owner": "Archon",
  "created_at": "RFC3339"
}
```

Forbidden vague phrases:

- "should work"
- "probably available"
- "later"
- "TBD"
- "some provider"
- "best effort" without fail-closed behavior

---

## 33. Final No-Open-Questions Statement

This PRD intentionally has no open questions. All unresolved provider or data
uncertainty is handled through capability checks, coverage reports, unavailable
reasons, residual-gap records, and fail-closed behavior.
