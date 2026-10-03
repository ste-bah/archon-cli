//! Argument structs for `TradingCliDataAction` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

use super::*;

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataStatusArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataIngestOhlcvArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Source CSV/JSON file
    #[arg(long)]
    pub source: PathBuf,
    /// Source format
    #[arg(long, value_enum)]
    pub format: TradingCliOhlcvFormat,
    /// Stable dataset id referenced by StrategySpec SPEC-F04
    #[arg(long)]
    pub dataset_id: String,
    /// Immutable dataset version, for example v1 or 2026-06-06
    #[arg(long)]
    pub version: String,
    /// Data provider/source name
    #[arg(long)]
    pub provider: String,
    /// Canonical trading symbol
    #[arg(long)]
    pub symbol: String,
    /// Dataset timezone
    #[arg(long, default_value = "UTC")]
    pub timezone: String,
    /// Provider-native symbol when it differs from the canonical symbol
    #[arg(long)]
    pub provider_symbol: Option<String>,
    /// Asset class label, for example equity, crypto, future, fx, or option
    #[arg(long, default_value = "unknown")]
    pub asset_class: String,
    /// Adjustment policy, for example raw or split_and_dividend
    #[arg(long, default_value = "raw")]
    pub adjustment: String,
    /// License/evidence tier label
    #[arg(long, default_value = "research")]
    pub license: String,
    /// Expected bars; defaults to observed bar count when omitted
    #[arg(long)]
    pub expected_bars: Option<u64>,
    /// Native provider interval/timeframe, for example 1D, 60, or 15
    #[arg(long, default_value = "unknown")]
    pub timeframe: String,
    /// Mark the dataset as using a native provider interval
    #[arg(long)]
    pub native_interval: bool,
    /// Mark the dataset as production eligible after external governance checks
    #[arg(long)]
    pub production_eligible: bool,
    /// Price basis used by the stored bars, for example raw or adjusted
    #[arg(long, default_value = "raw")]
    pub price_basis: String,
    /// Trading session covered by the bars, for example regular or 24x7
    #[arg(long, default_value = "provider_default")]
    pub session: String,
    /// Quality status label for validation provenance
    #[arg(long, default_value = "degraded")]
    pub quality_status: String,
    /// Missing bars in the known coverage window
    #[arg(long, default_value_t = 0)]
    pub missing_bars: u64,
    /// Mark dataset optional for promotion readiness
    #[arg(long)]
    pub optional: bool,
    /// Optional JSON output path for the stored dataset record
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataListArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Render registry JSON to stdout
    #[arg(long)]
    pub json: bool,
    /// Optional JSON output path for registry contents
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataShowArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Stored dataset id
    #[arg(long)]
    pub dataset_id: String,
    /// Stored dataset version
    #[arg(long)]
    pub version: String,
    /// Optional JSON output path for dataset details
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataValidateArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Stored dataset id
    #[arg(long)]
    pub dataset_id: String,
    /// Stored dataset version
    #[arg(long)]
    pub version: String,
    /// Optional JSON output path for validation report
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataProvidersArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Render JSON to stdout
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataCapabilityArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    #[arg(long)]
    pub provider: String,
    #[arg(long)]
    pub symbol: String,
    #[arg(long)]
    pub timeframe: String,
    /// Render JSON to stdout
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataFetchNativeArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Data provider, for example tradingview, polygon, openbb, stooq, or yfinance
    #[arg(long)]
    pub provider: String,
    /// Canonical trading symbol
    #[arg(long)]
    pub symbol: String,
    /// Exact provider-native timeframe: 1W, 1D, 240, 60, or 15
    #[arg(long)]
    pub timeframe: String,
    /// Requested start date/time as RFC3339 or YYYY-MM-DD
    #[arg(long)]
    pub start: String,
    /// Requested end date/time as RFC3339 or YYYY-MM-DD
    #[arg(long)]
    pub end: String,
    /// Stable dataset id for a successful native provider ingest
    #[arg(long)]
    pub dataset_id: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataSnapshotArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    #[arg(long)]
    pub provider: String,
    #[arg(long)]
    pub symbol: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataCoverageArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Required universe; v1 supports trading-core-v1
    #[arg(long, default_value = "trading-core-v1")]
    pub universe: String,
    /// Render JSON instead of readable text
    #[arg(long)]
    pub json: bool,
    /// Optional output path for coverage report
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataVerifyArtifactArgs {
    /// Dataset version directory containing manifest.json
    pub dataset_dir: PathBuf,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataVerifyCoverageArgs {
    /// Coverage JSON artifact
    pub coverage: PathBuf,
    /// Dataset registry JSON artifact
    pub registry: PathBuf,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TradingCliDataExportArgs {
    /// Project root containing .archon/trading-lab/data
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Stored dataset id
    #[arg(long)]
    pub dataset_id: String,
    /// Stored dataset version
    #[arg(long)]
    pub version: String,
    /// JSON output path
    #[arg(long)]
    pub out: PathBuf,
}
