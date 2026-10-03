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
    Status(TradingCliDataStatusArgs),
    /// Ingest OHLCV CSV or JSON into the persistent Trading Lab data lake
    IngestOhlcv(TradingCliDataIngestOhlcvArgs),
    /// List stored market datasets
    List(TradingCliDataListArgs),
    /// Show one stored dataset record and metadata
    Show(TradingCliDataShowArgs),
    /// Validate a stored OHLCV dataset and write validation.json
    #[command(alias = "validate-ohlcv")]
    Validate(TradingCliDataValidateArgs),
    /// List data providers supported by the capability interface
    Providers(TradingCliDataProvidersArgs),
    /// Check provider/symbol/timeframe native capability without full download
    Capability(TradingCliDataCapabilityArgs),
    /// Provider-native OHLCV fetch command shape; provider support fails closed when unavailable
    FetchNative(TradingCliDataFetchNativeArgs),
    /// Generic current snapshot command shape; provider tasks implement fetch support
    Snapshot(TradingCliDataSnapshotArgs),
    /// Generate required trading-core-v1 coverage matrix
    Coverage(TradingCliDataCoverageArgs),
    /// Verify one pipeline-produced dataset artifact directory using typed contracts
    VerifyArtifact(TradingCliDataVerifyArtifactArgs),
    /// Verify coverage, registry linkage, and every referenced dataset checksum chain
    VerifyCoverage(TradingCliDataVerifyCoverageArgs),
    /// Export stored normalized OHLCV bars as JSON
    #[command(alias = "export-ohlcv")]
    Export(TradingCliDataExportArgs),
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

#[path = "trading_market_actions_args.rs"]
pub(super) mod trading_market_actions_args;
pub use trading_market_actions_args::*;
