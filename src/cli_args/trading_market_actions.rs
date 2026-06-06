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
}

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
        /// Adjustment policy, for example raw or split_and_dividend
        #[arg(long, default_value = "raw")]
        adjustment: String,
        /// License/evidence tier label
        #[arg(long, default_value = "research")]
        license: String,
        /// Expected bars; defaults to observed bar count when omitted
        #[arg(long)]
        expected_bars: Option<u64>,
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
    /// Export stored normalized OHLCV bars as JSON
    ExportOhlcv {
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
