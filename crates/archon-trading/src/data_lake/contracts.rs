use super::{normalize_timeframe, provider_supports_native_timeframe, unavailable_reason};
use crate::ohlcv::OhlcvBar;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    Passed,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationCheck {
    pub id: String,
    pub status: ValidationStatus,
    pub severity: ValidationSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub row_count: u64,
    pub duplicate_timestamp_count: u64,
    pub gap_count: u64,
    pub bad_ohlc_count: u64,
    pub missing_volume_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub schema_version: String,
    pub dataset_id: String,
    pub version: String,
    pub status: ValidationStatus,
    pub native_interval: bool,
    pub production_eligible: bool,
    pub checks: Vec<ValidationCheck>,
    pub summary: ValidationSummary,
    pub validated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityResult {
    pub provider: String,
    pub symbol: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    #[serde(default)]
    pub native_interval: bool,
    #[serde(default)]
    pub production_eligible: bool,
    #[serde(default)]
    pub can_fetch: bool,
    #[serde(default)]
    pub current_snapshot_supported: bool,
    #[serde(default)]
    pub historical_supported: bool,
    #[serde(default)]
    pub requires_credentials: bool,
    #[serde(default)]
    pub missing_credentials: bool,
    #[serde(default)]
    pub provider_blocked: bool,
    #[serde(default)]
    pub unsupported: bool,
    #[serde(default)]
    pub credential_state: String,
    pub unavailable_reason: Option<String>,
    pub checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentSnapshot {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub captured_at_unix_seconds: i64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageMatrix {
    pub schema_version: String,
    pub generated_at: String,
    pub instruments: Vec<String>,
    pub timeframes: Vec<String>,
    pub cells: Vec<CoverageCell>,
    pub gaps: Vec<CoverageGap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageCell {
    pub canonical_instrument: String,
    pub timeframe: String,
    pub selected_provider: String,
    pub provider_symbol: String,
    pub dataset_id: Option<String>,
    pub version: Option<String>,
    pub available: bool,
    pub native_interval: bool,
    pub production_eligible: bool,
    pub quality_status: String,
    pub row_count: u64,
    pub coverage_start: String,
    pub coverage_end: String,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGap {
    pub canonical_instrument: String,
    pub timeframe: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacktestDataGateReport {
    pub dataset_id: String,
    pub version: String,
    pub diagnostic: bool,
    pub promotion_eligible: bool,
    pub issues: Vec<String>,
    pub overridden_issues: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotFreshness {
    Fresh,
    Stale,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFetchError {
    pub provider: String,
    pub action: &'static str,
    pub reason: String,
}

pub trait NativeOhlcvProvider {
    fn can_fetch_symbol_timeframe(
        &self,
        symbol: &str,
        timeframe: &str,
        checked_at: &str,
    ) -> ProviderCapabilityResult;

    fn fetch_ohlcv_native(
        &self,
        symbol: &str,
        timeframe: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<OhlcvBar>, ProviderFetchError>;

    fn fetch_current_snapshot(&self, symbol: &str) -> Result<CurrentSnapshot, ProviderFetchError>;
}

pub fn can_fetch_symbol_timeframe(
    provider: &str,
    symbol: &str,
    timeframe: &str,
    checked_at: &str,
) -> ProviderCapabilityResult {
    let normalized_provider = provider.trim().to_ascii_lowercase();
    let normalized_timeframe = normalize_timeframe(timeframe.trim());
    let trimmed_symbol = symbol.trim();
    let missing_input = normalized_provider.is_empty()
        || trimmed_symbol.is_empty()
        || normalized_timeframe.is_empty();
    let supported_provider = matches!(
        normalized_provider.as_str(),
        "tradingview" | "openbb" | "polygon" | "stooq" | "yfinance"
    );
    let exact_native_interval =
        matches!(
            normalized_timeframe.as_str(),
            "1W" | "1D" | "240" | "60" | "15"
        ) && provider_supports_native_timeframe(&normalized_provider, &normalized_timeframe);
    let requires_credentials = matches!(normalized_provider.as_str(), "openbb" | "polygon");
    let missing_credentials =
        requires_credentials && !has_provider_credentials(&normalized_provider);
    let unsupported = missing_input || !supported_provider || !exact_native_interval;
    let provider_blocked = provider_blocked(trimmed_symbol);
    let production_eligible = exact_native_interval
        && supported_provider
        && !missing_input
        && !missing_credentials
        && !provider_blocked
        && normalized_provider != "yfinance";
    let can_fetch = false;
    let unavailable_reason = Some(capability_unavailable_reason(
        provider,
        trimmed_symbol,
        &normalized_timeframe,
        (supported_provider, exact_native_interval),
        (missing_credentials, provider_blocked),
        &normalized_provider,
        production_eligible,
    ));
    ProviderCapabilityResult {
        provider: normalized_provider,
        symbol: trimmed_symbol.to_string(),
        canonical_instrument: trimmed_symbol.to_string(),
        provider_symbol: trimmed_symbol.to_string(),
        timeframe: normalized_timeframe.clone(),
        native_interval: exact_native_interval && supported_provider && !missing_input,
        production_eligible,
        can_fetch,
        current_snapshot_supported: supported_provider && !missing_input,
        historical_supported: supported_provider && exact_native_interval && !missing_input,
        requires_credentials,
        missing_credentials,
        provider_blocked,
        unsupported,
        credential_state: credential_state_label(requires_credentials, missing_credentials),
        unavailable_reason,
        checked_at: checked_at.to_string(),
    }
}

fn capability_unavailable_reason(
    provider: &str,
    symbol: &str,
    timeframe: &str,
    (supported_provider, native_interval): (bool, bool),
    (missing_credentials, provider_blocked): (bool, bool),
    normalized_provider: &str,
    provider_implemented: bool,
) -> String {
    if let Some(reason) = http_status_unavailable_reason(symbol) {
        reason.into()
    } else if provider_blocked {
        "provider blocked access".into()
    } else if missing_credentials {
        "missing provider credentials".into()
    } else if normalized_provider == "yfinance" && native_interval {
        "yfinance fallback is degraded and ineligible for promotion".into()
    } else if provider_implemented {
        "capability record only; downstream provider fetch implementation proof is required before can_fetch=true"
            .into()
    } else {
        unavailable_reason(
            provider,
            symbol,
            timeframe,
            supported_provider,
            native_interval,
        )
    }
}

fn has_provider_credentials(provider: &str) -> bool {
    let keys: &[&str] = match provider {
        "openbb" | "polygon" => &["POLYGON_API_KEY"],
        _ => &[],
    };
    keys.is_empty() || keys.iter().any(|key| std::env::var_os(key).is_some())
}

fn http_status_unavailable_reason(symbol: &str) -> Option<&'static str> {
    match symbol.trim().parse::<u16>().ok()? {
        401 => Some("missing or invalid provider credentials"),
        403 => Some("provider blocked access"),
        404 => Some("provider symbol or endpoint not found"),
        _ => None,
    }
}

fn provider_blocked(symbol: &str) -> bool {
    matches!(symbol.trim().parse::<u16>(), Ok(403))
}

fn credential_state_label(requires_credentials: bool, missing_credentials: bool) -> String {
    match (requires_credentials, missing_credentials) {
        (false, _) => "not_required".into(),
        (true, true) => "missing".into(),
        (true, false) => "available".into(),
    }
}

pub fn snapshot_is_fresh(captured_at_unix_seconds: i64, now_unix_seconds: i64) -> bool {
    let age = now_unix_seconds.saturating_sub(captured_at_unix_seconds);
    (0..=300).contains(&age)
}

pub fn snapshot_freshness(
    captured_at_unix_seconds: Option<i64>,
    now_unix_seconds: i64,
) -> SnapshotFreshness {
    captured_at_unix_seconds.map_or(SnapshotFreshness::Missing, |captured_at| {
        if snapshot_is_fresh(captured_at, now_unix_seconds) {
            SnapshotFreshness::Fresh
        } else {
            SnapshotFreshness::Stale
        }
    })
}
