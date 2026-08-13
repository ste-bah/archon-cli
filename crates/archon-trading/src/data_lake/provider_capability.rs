use super::{CurrentSnapshot, ProviderCapabilityResult, ProviderHistoryHorizon};
use crate::ohlcv::{OhlcvBar, validate_bars};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub const MAX_PROBE_CANDLES: usize = 2;
pub const MAX_PROBE_DURATION: Duration = Duration::from_secs(5);
pub const MAX_PROBE_RESPONSE_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeFetchRequest {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRequest {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeBudget {
    pub max_candles: usize,
    pub max_duration: Duration,
    pub max_response_bytes: usize,
}

impl Default for ProbeBudget {
    fn default() -> Self {
        Self {
            max_candles: MAX_PROBE_CANDLES,
            max_duration: MAX_PROBE_DURATION,
            max_response_bytes: MAX_PROBE_RESPONSE_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityEvidence {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub native_interval: bool,
    pub historical_supported: bool,
    pub current_snapshot_supported: bool,
    pub requires_credentials: bool,
    pub credential_available: bool,
    pub history_horizon: Option<ProviderHistoryHorizon>,
    pub candles_examined: usize,
    pub response_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeHistory {
    pub provider: String,
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub requested_start: String,
    pub requested_end: String,
    pub coverage_start: String,
    pub coverage_end: String,
    pub provenance: NativeProvenance,
    pub bars: Vec<OhlcvBar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeProvenance {
    pub provider: String,
    pub provider_symbol: String,
    pub native_timeframe: String,
    pub resampled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NativeFetchResult {
    Complete { history: NativeHistory },
    Partial { history: NativeHistory },
    Unavailable { reason: UnavailableReason },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SnapshotResult {
    Available { snapshot: CurrentSnapshot },
    Unavailable { reason: UnavailableReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    InvalidRequest,
    ProviderNotRegistered,
    MissingCredentials,
    Unauthorized401,
    ProviderBlocked403,
    NotFound404,
    HttpStatusError,
    ProviderVerificationBlock,
    ExactNativeIntervalUnsupported,
    HistoricalUnsupported,
    CurrentSnapshotUnsupported,
    ProbeLimitExceeded,
    MalformedResponse,
    ProbeInconclusive,
}

impl UnavailableReason {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::ProviderNotRegistered => "provider_not_registered",
            Self::MissingCredentials => "missing_credentials",
            Self::Unauthorized401 => "unauthorized_401",
            Self::ProviderBlocked403 => "provider_blocked_403",
            Self::NotFound404 => "not_found_404",
            Self::HttpStatusError => "http_status_error",
            Self::ProviderVerificationBlock => "provider_verification_block",
            Self::ExactNativeIntervalUnsupported => "exact_native_interval_unsupported",
            Self::HistoricalUnsupported => "historical_unsupported",
            Self::CurrentSnapshotUnsupported => "current_snapshot_unsupported",
            Self::ProbeLimitExceeded => "probe_limit_exceeded",
            Self::MalformedResponse => "malformed_response",
            Self::ProbeInconclusive => "probe_inconclusive",
        }
    }
}

pub trait ProviderAdapter: Send + Sync {
    fn provider_id(&self) -> &str;

    /// Performs only the adapter's bounded metadata/small-sample probe.
    fn probe_capability(
        &self,
        request: &CapabilityRequest,
        budget: ProbeBudget,
    ) -> Result<CapabilityEvidence, UnavailableReason>;

    fn fetch_native_history(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<NativeHistory, UnavailableReason>;

    fn supports_current_snapshot(&self) -> bool {
        false
    }

    fn fetch_current_snapshot(
        &self,
        _request: &SnapshotRequest,
    ) -> Result<CurrentSnapshot, UnavailableReason> {
        Err(UnavailableReason::CurrentSnapshotUnsupported)
    }
}

#[derive(Default)]
pub struct ProviderDispatcher {
    adapters: BTreeMap<String, Box<dyn ProviderAdapter>>,
}

impl ProviderDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<A: ProviderAdapter + 'static>(&mut self, adapter: A) -> bool {
        let id = normalize_provider(adapter.provider_id());
        if id.is_empty() {
            return false;
        }
        self.adapters.insert(id, Box::new(adapter)).is_none()
    }

    pub fn can_fetch_symbol_timeframe(
        &self,
        request: &CapabilityRequest,
    ) -> ProviderCapabilityResult {
        let normalized = normalize_capability_request(request);
        let mut result = unavailable_capability(&normalized, UnavailableReason::InvalidRequest);
        if !valid_capability_request(&normalized) {
            return result;
        }
        let Some(adapter) = self.adapters.get(&normalized.provider) else {
            result.unavailable_reason =
                Some(UnavailableReason::ProviderNotRegistered.code().into());
            return result;
        };
        let started = Instant::now();
        let evidence = adapter.probe_capability(&normalized, ProbeBudget::default());
        match evidence {
            Ok(evidence) if started.elapsed() <= MAX_PROBE_DURATION => {
                capability_from_evidence(&normalized, evidence)
            }
            Ok(_) => unavailable_capability(&normalized, UnavailableReason::ProbeLimitExceeded),
            Err(reason) => unavailable_capability(&normalized, reason),
        }
    }

    pub fn fetch_ohlcv_native(&self, request: &NativeFetchRequest) -> NativeFetchResult {
        let request = normalize_native_request(request);
        if !valid_native_request(&request) {
            return native_unavailable(UnavailableReason::InvalidRequest);
        }
        let Some(adapter) = self.adapters.get(&request.provider) else {
            return native_unavailable(UnavailableReason::ProviderNotRegistered);
        };
        match adapter.fetch_native_history(&request) {
            Ok(history) => classify_native_history(&request, history),
            Err(reason) => native_unavailable(reason),
        }
    }

    pub fn fetch_current_snapshot(&self, request: &SnapshotRequest) -> SnapshotResult {
        let request = normalize_snapshot_request(request);
        if !valid_snapshot_request(&request) {
            return snapshot_unavailable(UnavailableReason::InvalidRequest);
        }
        let Some(adapter) = self.adapters.get(&request.provider) else {
            return snapshot_unavailable(UnavailableReason::ProviderNotRegistered);
        };
        if !adapter.supports_current_snapshot() {
            return snapshot_unavailable(UnavailableReason::CurrentSnapshotUnsupported);
        }
        match adapter.fetch_current_snapshot(&request) {
            Ok(snapshot)
                if snapshot_identity_matches(&request, &snapshot)
                    && !contains_sensitive_field(&snapshot.payload) =>
            {
                SnapshotResult::Available { snapshot }
            }
            Ok(_) => snapshot_unavailable(UnavailableReason::MalformedResponse),
            Err(reason) => snapshot_unavailable(reason),
        }
    }
}

fn capability_from_evidence(
    request: &CapabilityRequest,
    evidence: CapabilityEvidence,
) -> ProviderCapabilityResult {
    if !capability_identity_matches(request, &evidence) {
        return unavailable_capability(request, UnavailableReason::MalformedResponse);
    }
    if evidence.candles_examined > MAX_PROBE_CANDLES
        || evidence.response_bytes > MAX_PROBE_RESPONSE_BYTES
    {
        return unavailable_capability(request, UnavailableReason::ProbeLimitExceeded);
    }
    if evidence.requires_credentials && !evidence.credential_available {
        return unavailable_capability(request, UnavailableReason::MissingCredentials);
    }
    if !evidence.historical_supported {
        return unavailable_capability(request, UnavailableReason::HistoricalUnsupported);
    }
    if !evidence.native_interval {
        return unavailable_capability(request, UnavailableReason::ExactNativeIntervalUnsupported);
    }
    ProviderCapabilityResult {
        provider: request.provider.clone(),
        symbol: request.canonical_instrument.clone(),
        canonical_instrument: request.canonical_instrument.clone(),
        provider_symbol: request.provider_symbol.clone(),
        timeframe: request.timeframe.clone(),
        native_interval: true,
        production_eligible: false,
        can_fetch: true,
        current_snapshot_supported: evidence.current_snapshot_supported,
        historical_supported: true,
        history_horizon: evidence.history_horizon,
        requires_credentials: evidence.requires_credentials,
        missing_credentials: false,
        provider_blocked: false,
        unsupported: false,
        credential_state: if evidence.requires_credentials {
            "present"
        } else {
            "not_required"
        }
        .into(),
        unavailable_reason: None,
        checked_at: request.checked_at.clone(),
    }
}

fn unavailable_capability(
    request: &CapabilityRequest,
    reason: UnavailableReason,
) -> ProviderCapabilityResult {
    ProviderCapabilityResult {
        provider: request.provider.clone(),
        symbol: request.canonical_instrument.clone(),
        canonical_instrument: request.canonical_instrument.clone(),
        provider_symbol: request.provider_symbol.clone(),
        timeframe: request.timeframe.clone(),
        native_interval: false,
        production_eligible: false,
        can_fetch: false,
        current_snapshot_supported: false,
        historical_supported: false,
        history_horizon: None,
        requires_credentials: false,
        missing_credentials: reason == UnavailableReason::MissingCredentials,
        provider_blocked: matches!(
            reason,
            UnavailableReason::ProviderBlocked403 | UnavailableReason::ProviderVerificationBlock
        ),
        unsupported: matches!(
            reason,
            UnavailableReason::ExactNativeIntervalUnsupported
                | UnavailableReason::HistoricalUnsupported
                | UnavailableReason::CurrentSnapshotUnsupported
        ),
        credential_state: if reason == UnavailableReason::MissingCredentials {
            "missing"
        } else {
            "unknown"
        }
        .into(),
        unavailable_reason: Some(reason.code().into()),
        checked_at: request.checked_at.clone(),
    }
}

fn classify_native_history(
    request: &NativeFetchRequest,
    history: NativeHistory,
) -> NativeFetchResult {
    if !native_identity_matches(request, &history)
        || history.provenance.resampled
        || validate_bars(&history.bars).is_err()
        || history.coverage_start > history.coverage_end
    {
        return native_unavailable(UnavailableReason::MalformedResponse);
    }
    if history.coverage_start == request.start && history.coverage_end == request.end {
        NativeFetchResult::Complete { history }
    } else if history.coverage_start >= request.start && history.coverage_end <= request.end {
        NativeFetchResult::Partial { history }
    } else {
        native_unavailable(UnavailableReason::MalformedResponse)
    }
}

fn native_identity_matches(request: &NativeFetchRequest, history: &NativeHistory) -> bool {
    history.provider == request.provider
        && history.canonical_instrument == request.canonical_instrument
        && history.provider_symbol == request.provider_symbol
        && history.timeframe == request.timeframe
        && history.requested_start == request.start
        && history.requested_end == request.end
        && history.provenance.provider == request.provider
        && history.provenance.provider_symbol == request.provider_symbol
        && history.provenance.native_timeframe == request.timeframe
}

fn capability_identity_matches(request: &CapabilityRequest, evidence: &CapabilityEvidence) -> bool {
    evidence.provider == request.provider
        && evidence.canonical_instrument == request.canonical_instrument
        && evidence.provider_symbol == request.provider_symbol
        && evidence.timeframe == request.timeframe
}

fn snapshot_identity_matches(request: &SnapshotRequest, snapshot: &CurrentSnapshot) -> bool {
    snapshot.provider == request.provider
        && snapshot.canonical_instrument == request.canonical_instrument
        && snapshot.provider_symbol == request.provider_symbol
}

fn native_unavailable(reason: UnavailableReason) -> NativeFetchResult {
    NativeFetchResult::Unavailable { reason }
}

fn snapshot_unavailable(reason: UnavailableReason) -> SnapshotResult {
    SnapshotResult::Unavailable { reason }
}

fn normalize_capability_request(request: &CapabilityRequest) -> CapabilityRequest {
    CapabilityRequest {
        provider: normalize_provider(&request.provider),
        canonical_instrument: request.canonical_instrument.trim().to_string(),
        provider_symbol: request.provider_symbol.trim().to_string(),
        timeframe: request.timeframe.trim().to_string(),
        checked_at: request.checked_at.trim().to_string(),
    }
}

fn normalize_native_request(request: &NativeFetchRequest) -> NativeFetchRequest {
    NativeFetchRequest {
        provider: normalize_provider(&request.provider),
        canonical_instrument: request.canonical_instrument.trim().to_string(),
        provider_symbol: request.provider_symbol.trim().to_string(),
        timeframe: request.timeframe.trim().to_string(),
        start: request.start.trim().to_string(),
        end: request.end.trim().to_string(),
    }
}

fn normalize_snapshot_request(request: &SnapshotRequest) -> SnapshotRequest {
    SnapshotRequest {
        provider: normalize_provider(&request.provider),
        canonical_instrument: request.canonical_instrument.trim().to_string(),
        provider_symbol: request.provider_symbol.trim().to_string(),
    }
}

fn normalize_provider(provider: &str) -> String {
    provider.trim().to_ascii_lowercase()
}

fn valid_capability_request(request: &CapabilityRequest) -> bool {
    !request.provider.is_empty()
        && !request.canonical_instrument.is_empty()
        && !request.provider_symbol.is_empty()
        && !request.timeframe.is_empty()
        && !request.checked_at.is_empty()
}

fn valid_native_request(request: &NativeFetchRequest) -> bool {
    !request.provider.is_empty()
        && !request.canonical_instrument.is_empty()
        && !request.provider_symbol.is_empty()
        && !request.timeframe.is_empty()
        && !request.start.is_empty()
        && request.start < request.end
}

fn valid_snapshot_request(request: &SnapshotRequest) -> bool {
    !request.provider.is_empty()
        && !request.canonical_instrument.is_empty()
        && !request.provider_symbol.is_empty()
}

fn contains_sensitive_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| sensitive_key(key) || contains_sensitive_field(value)),
        serde_json::Value::Array(values) => values.iter().any(contains_sensitive_field),
        _ => false,
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "apikey",
        "authorization",
        "cookie",
        "credential",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[cfg(test)]
#[path = "provider_capability_tests.rs"]
mod provider_capability_tests;
