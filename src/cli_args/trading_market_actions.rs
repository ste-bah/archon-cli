use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum TradingCliBacktestAction {
    /// Run a deterministic native backtest from config and fill JSON files
    Run {
        /// BacktestConfig JSON file
        #[arg(long)]
        config: PathBuf,
        /// JSON array of FillInput records
        #[arg(long)]
        fills: PathBuf,
        /// Dataset health gate
        #[arg(long, value_enum, default_value = "healthy")]
        dataset_status: TradingCliDatasetStatus,
        /// Mark evidence exploratory; exploratory evidence cannot promote
        #[arg(long)]
        exploratory: bool,
        /// Evidence source
        #[arg(long, value_enum, default_value = "native-harness")]
        source: TradingCliBacktestSource,
        /// Optional JSON output path for BacktestReport
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run a deterministic candle backtest from a stored OHLCV dataset
    RunOhlcv {
        /// BacktestConfig JSON file
        #[arg(long)]
        config: PathBuf,
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Stored dataset id
        #[arg(long)]
        dataset_id: String,
        /// Stored dataset version
        #[arg(long)]
        version: String,
        /// Allow degraded data for diagnostic exploratory reports only
        #[arg(long)]
        diagnostic_allow_degraded_data: bool,
        /// Units/contracts/shares per trade
        #[arg(long)]
        quantity: f64,
        /// Built-in candle strategy rule used when --strategy-rules is omitted
        #[arg(long, value_enum, default_value = "close-momentum")]
        rule: TradingCliOhlcvRule,
        /// Custom deterministic strategy-rules JSON file
        #[arg(long)]
        strategy_rules: Option<PathBuf>,
        /// Fast SMA length for sma-cross
        #[arg(long, default_value_t = 10)]
        fast_len: usize,
        /// Slow SMA length for sma-cross
        #[arg(long, default_value_t = 30)]
        slow_len: usize,
        /// Mark evidence exploratory; exploratory evidence cannot promote
        #[arg(long)]
        exploratory: bool,
        /// Evidence source
        #[arg(long, value_enum, default_value = "native-harness")]
        source: TradingCliBacktestSource,
        /// Optional JSON output path for OhlcvBacktestReport
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run AHDM-v1 native backtest and persist the full replay artifact suite
    RunAhdmNative {
        /// BacktestConfig JSON file
        #[arg(long)]
        config: PathBuf,
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Stable AHDM backtest run id
        #[arg(long)]
        run_id: String,
        /// Stored dataset id
        #[arg(long)]
        dataset_id: String,
        /// Stored dataset version
        #[arg(long)]
        version: String,
        /// Units/contracts/shares per trade
        #[arg(long)]
        quantity: f64,
        /// RFC3339 timestamp for deterministic artifact generation
        #[arg(long)]
        generated_at: Option<String>,
        /// Optional JSON output path for the created artifact directory report
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

// PRD-TRADING-DATA-LAKE work in progress; variant layout settles with the PRD.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliDataAction {
    /// Show persistent Trading Lab data-lake status
    Status {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
    },
    /// Ingest OHLCV CSV or JSON into the persistent Trading Lab data lake
    IngestOhlcv {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Source CSV/JSON file
        #[arg(long)]
        source: PathBuf,
        /// Source format
        #[arg(long, value_enum)]
        format: TradingCliOhlcvFormat,
        /// Stable dataset id referenced by StrategySpec SPEC-F04
        #[arg(long)]
        dataset_id: String,
        /// Immutable dataset version, for example v1 or 2026-06-06
        #[arg(long)]
        version: String,
        /// Data provider/source name
        #[arg(long)]
        provider: String,
        /// Canonical trading symbol
        #[arg(long)]
        symbol: String,
        /// Dataset timezone
        #[arg(long, default_value = "UTC")]
        timezone: String,
        /// Provider-native symbol when it differs from the canonical symbol
        #[arg(long)]
        provider_symbol: Option<String>,
        /// Asset class label, for example equity, crypto, future, fx, or option
        #[arg(long, default_value = "unknown")]
        asset_class: String,
        /// Adjustment policy, for example raw or split_and_dividend
        #[arg(long, default_value = "raw")]
        adjustment: String,
        /// License/evidence tier label
        #[arg(long, default_value = "research")]
        license: String,
        /// Expected bars; defaults to observed bar count when omitted
        #[arg(long)]
        expected_bars: Option<u64>,
        /// Native provider interval/timeframe, for example 1D, 60, or 15
        #[arg(long, default_value = "unknown")]
        timeframe: String,
        /// Mark the dataset as using a native provider interval
        #[arg(long)]
        native_interval: bool,
        /// Mark the dataset as production eligible after external governance checks
        #[arg(long)]
        production_eligible: bool,
        /// Price basis used by the stored bars, for example raw or adjusted
        #[arg(long, default_value = "raw")]
        price_basis: String,
        /// Trading session covered by the bars, for example regular or 24x7
        #[arg(long, default_value = "provider_default")]
        session: String,
        /// Quality status label for validation provenance
        #[arg(long, default_value = "degraded")]
        quality_status: String,
        /// Missing bars in the known coverage window
        #[arg(long, default_value_t = 0)]
        missing_bars: u64,
        /// Mark dataset optional for promotion readiness
        #[arg(long)]
        optional: bool,
        /// Optional JSON output path for the stored dataset record
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// List stored market datasets
    List {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Render registry JSON to stdout
        #[arg(long)]
        json: bool,
        /// Optional JSON output path for registry contents
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Show one stored dataset record and metadata
    Show {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Stored dataset id
        #[arg(long)]
        dataset_id: String,
        /// Stored dataset version
        #[arg(long)]
        version: String,
        /// Optional JSON output path for dataset details
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Validate a stored OHLCV dataset and write validation.json
    #[command(alias = "validate-ohlcv")]
    Validate {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Stored dataset id
        #[arg(long)]
        dataset_id: String,
        /// Stored dataset version
        #[arg(long)]
        version: String,
        /// Optional JSON output path for validation report
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// List data providers supported by the capability interface
    Providers {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Render JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Check provider/symbol/timeframe native capability without full download
    Capability {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        symbol: String,
        #[arg(long)]
        timeframe: String,
        /// Render JSON to stdout
        #[arg(long)]
        json: bool,
    },
    /// Provider-native OHLCV fetch command shape; provider support fails closed when unavailable
    FetchNative {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Data provider, for example tradingview, polygon, openbb, stooq, or yfinance
        #[arg(long)]
        provider: String,
        /// Canonical trading symbol
        #[arg(long)]
        symbol: String,
        /// Exact provider-native timeframe: 1W, 1D, 240, 60, or 15
        #[arg(long)]
        timeframe: String,
        /// Requested start date/time as RFC3339 or YYYY-MM-DD
        #[arg(long)]
        start: String,
        /// Requested end date/time as RFC3339 or YYYY-MM-DD
        #[arg(long)]
        end: String,
        /// Stable dataset id for a successful native provider ingest
        #[arg(long)]
        dataset_id: String,
    },
    /// Generic current snapshot command shape; provider tasks implement fetch support
    Snapshot {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        symbol: String,
    },
    /// Generate required trading-core-v1 coverage matrix
    Coverage {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Required universe; v1 supports trading-core-v1
        #[arg(long, default_value = "trading-core-v1")]
        universe: String,
        /// Render JSON instead of readable text
        #[arg(long)]
        json: bool,
        /// Optional output path for coverage report
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Verify one pipeline-produced dataset artifact directory using typed contracts
    VerifyArtifact {
        /// Dataset version directory containing manifest.json
        dataset_dir: PathBuf,
    },
    /// Verify coverage, registry linkage, and every referenced dataset checksum chain
    VerifyCoverage {
        /// Coverage JSON artifact
        coverage: PathBuf,
        /// Dataset registry JSON artifact
        registry: PathBuf,
    },
    /// Export stored normalized OHLCV bars as JSON
    #[command(alias = "export-ohlcv")]
    Export {
        /// Project root containing .archon/trading-lab/data
        #[arg(long)]
        target: Option<PathBuf>,
        /// Stored dataset id
        #[arg(long)]
        dataset_id: String,
        /// Stored dataset version
        #[arg(long)]
        version: String,
        /// JSON output path
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliDatasetStatus {
    Healthy,
    Degraded,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliBacktestSource {
    NativeHarness,
    StrategyTester,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliOhlcvFormat {
    Csv,
    Json,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliOhlcvRule {
    CloseMomentum,
    SmaCross,
}
