use crate::data_lake::{
    CapabilityEvidence, CapabilityRequest, NativeFetchRequest, NativeHistory, NativeProvenance,
    ProbeBudget, ProviderAdapter, ProviderDispatcher, UnavailableReason,
};
use crate::ohlcv::{OhlcvBar, validate_bars};
use serde::Deserialize;
use std::sync::Arc;

pub const YFINANCE_PROVIDER: &str = "yfinance";
pub const YFINANCE_CHART_BASE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
pub const YFINANCE_NATIVE_INTERVALS: &[&str] = &[
    "1m", "2m", "5m", "15m", "30m", "60m", "90m", "1h", "1d", "5d", "1wk", "1mo", "3mo",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YfinanceRequest {
    pub method: String,
    pub url: String,
    pub query: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YfinanceResponse {
    pub status: u16,
    pub content_type: String,
    pub headers: serde_json::Value,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YfinanceTransportError {
    Connection,
    Timeout,
}

pub trait YfinanceTransport: Send + Sync {
    fn execute(
        &self,
        request: &YfinanceRequest,
    ) -> Result<YfinanceResponse, YfinanceTransportError>;
}

pub trait YfinanceClock: Send + Sync {
    fn now_rfc3339(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemYfinanceClock;

impl YfinanceClock for SystemYfinanceClock {
    fn now_rfc3339(&self) -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedYfinanceHistory {
    pub history: NativeHistory,
    pub raw_body: Vec<u8>,
    pub request_provenance: serde_json::Value,
    pub redacted_headers: serde_json::Value,
    pub retrieved_at: String,
}

pub struct YfinanceAdapter {
    transport: Arc<dyn YfinanceTransport>,
    clock: Arc<dyn YfinanceClock>,
}

impl YfinanceAdapter {
    pub fn new(transport: Arc<dyn YfinanceTransport>, clock: Arc<dyn YfinanceClock>) -> Self {
        Self { transport, clock }
    }

    pub fn process(transport: Arc<dyn YfinanceTransport>) -> Self {
        Self::new(transport, Arc::new(SystemYfinanceClock))
    }

    pub fn fetch_for_ingest(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<PreparedYfinanceHistory, UnavailableReason> {
        validate_request(request)?;
        let http_request = history_request(request)?;
        let response = self.send(&http_request)?;
        self.prepare(request, http_request, response)
    }

    fn send(&self, request: &YfinanceRequest) -> Result<YfinanceResponse, UnavailableReason> {
        let response = self
            .transport
            .execute(request)
            .map_err(|_| UnavailableReason::ProbeInconclusive)?;
        classify_status(response.status)?;
        if !response.content_type.to_ascii_lowercase().contains("json")
            || starts_like_html(&response.body)
        {
            return Err(UnavailableReason::ProviderVerificationBlock);
        }
        Ok(response)
    }

    fn prepare(
        &self,
        request: &NativeFetchRequest,
        http_request: YfinanceRequest,
        response: YfinanceResponse,
    ) -> Result<PreparedYfinanceHistory, UnavailableReason> {
        let raw_value: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|_| UnavailableReason::MalformedResponse)?;
        if contains_sensitive_field(&raw_value) {
            return Err(UnavailableReason::ProviderVerificationBlock);
        }
        let payload: ChartEnvelope =
            serde_json::from_value(raw_value).map_err(|_| UnavailableReason::MalformedResponse)?;
        let history = payload.into_history(request)?;
        let retrieved_at = self.clock.now_rfc3339();
        if chrono::DateTime::parse_from_rfc3339(&retrieved_at).is_err() {
            return Err(UnavailableReason::MalformedResponse);
        }
        Ok(PreparedYfinanceHistory {
            history,
            raw_body: response.body,
            request_provenance: serde_json::json!({
                "method": http_request.method,
                "url": http_request.url,
                "query": http_request.query,
                "provider": YFINANCE_PROVIDER,
                "retrieved_at": retrieved_at,
                "direct_native": true,
                "derived": false,
                "resampled": false,
            }),
            redacted_headers: redact_json(response.headers),
            retrieved_at,
        })
    }
}

impl ProviderAdapter for YfinanceAdapter {
    fn provider_id(&self) -> &str {
        YFINANCE_PROVIDER
    }

    fn probe_capability(
        &self,
        request: &CapabilityRequest,
        _budget: ProbeBudget,
    ) -> Result<CapabilityEvidence, UnavailableReason> {
        validate_capability_request(request)?;
        supported_interval(&request.timeframe)?;
        Ok(CapabilityEvidence {
            provider: YFINANCE_PROVIDER.into(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            native_interval: true,
            historical_supported: true,
            current_snapshot_supported: false,
            requires_credentials: false,
            credential_available: true,
            history_horizon: None,
            candles_examined: 0,
            response_bytes: 0,
        })
    }

    fn fetch_native_history(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<NativeHistory, UnavailableReason> {
        self.fetch_for_ingest(request)
            .map(|prepared| prepared.history)
    }
}

pub fn register_yfinance(dispatcher: &mut ProviderDispatcher, adapter: YfinanceAdapter) -> bool {
    dispatcher.register(adapter)
}

fn validate_request(request: &NativeFetchRequest) -> Result<(), UnavailableReason> {
    if request.provider != YFINANCE_PROVIDER
        || request.canonical_instrument.trim().is_empty()
        || request.provider_symbol.trim().is_empty()
        || request.provider_symbol != request.provider_symbol.trim()
        || request.start >= request.end
    {
        return Err(UnavailableReason::InvalidRequest);
    }
    parse_unix(&request.start)?;
    parse_unix(&request.end)?;
    supported_interval(&request.timeframe)
}

fn validate_capability_request(request: &CapabilityRequest) -> Result<(), UnavailableReason> {
    if request.provider != YFINANCE_PROVIDER
        || request.canonical_instrument.trim().is_empty()
        || request.provider_symbol.trim().is_empty()
        || request.provider_symbol != request.provider_symbol.trim()
        || chrono::DateTime::parse_from_rfc3339(&request.checked_at).is_err()
    {
        return Err(UnavailableReason::InvalidRequest);
    }
    Ok(())
}

fn supported_interval(value: &str) -> Result<(), UnavailableReason> {
    YFINANCE_NATIVE_INTERVALS
        .contains(&value)
        .then_some(())
        .ok_or(UnavailableReason::ExactNativeIntervalUnsupported)
}

fn history_request(request: &NativeFetchRequest) -> Result<YfinanceRequest, UnavailableReason> {
    supported_interval(&request.timeframe)?;
    Ok(YfinanceRequest {
        method: "GET".into(),
        url: format!("{YFINANCE_CHART_BASE_URL}/{}", request.provider_symbol),
        query: vec![
            ("period1".into(), parse_unix(&request.start)?.to_string()),
            ("period2".into(), parse_unix(&request.end)?.to_string()),
            ("interval".into(), request.timeframe.clone()),
            ("events".into(), "history".into()),
            ("includeAdjustedClose".into(), "false".into()),
        ],
    })
}

fn parse_unix(value: &str) -> Result<i64, UnavailableReason> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.timestamp())
        .map_err(|_| UnavailableReason::InvalidRequest)
}

fn classify_status(status: u16) -> Result<(), UnavailableReason> {
    match status {
        200..=299 => Ok(()),
        401 => Err(UnavailableReason::Unauthorized401),
        403 => Err(UnavailableReason::ProviderBlocked403),
        404 => Err(UnavailableReason::NotFound404),
        _ => Err(UnavailableReason::HttpStatusError),
    }
}

#[derive(Deserialize)]
struct ChartEnvelope {
    chart: Chart,
}

#[derive(Deserialize)]
struct Chart {
    result: Option<Vec<ChartResult>>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ChartResult {
    meta: ChartMeta,
    timestamp: Vec<i64>,
    indicators: Indicators,
}

#[derive(Deserialize)]
struct ChartMeta {
    symbol: String,
    #[serde(rename = "dataGranularity")]
    data_granularity: String,
}

#[derive(Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Deserialize)]
struct Quote {
    open: Vec<Option<f64>>,
    high: Vec<Option<f64>>,
    low: Vec<Option<f64>>,
    close: Vec<Option<f64>>,
    volume: Vec<Option<f64>>,
}

impl ChartEnvelope {
    fn into_history(
        self,
        request: &NativeFetchRequest,
    ) -> Result<NativeHistory, UnavailableReason> {
        if self.chart.error.is_some() {
            return Err(UnavailableReason::ProbeInconclusive);
        }
        let mut results = self
            .chart
            .result
            .ok_or(UnavailableReason::MalformedResponse)?;
        if results.len() != 1 {
            return Err(UnavailableReason::MalformedResponse);
        }
        let result = results.pop().ok_or(UnavailableReason::MalformedResponse)?;
        if result.meta.symbol != request.provider_symbol
            || result.meta.data_granularity != request.timeframe
            || result.indicators.quote.len() != 1
        {
            return Err(UnavailableReason::MalformedResponse);
        }
        let bars = result.into_bars()?;
        validate_bars(&bars).map_err(|_| UnavailableReason::MalformedResponse)?;
        let coverage_start = bars
            .first()
            .map(|bar| bar.timestamp.clone())
            .ok_or(UnavailableReason::MalformedResponse)?;
        let coverage_end = bars
            .last()
            .map(|bar| bar.timestamp.clone())
            .ok_or(UnavailableReason::MalformedResponse)?;
        Ok(NativeHistory {
            provider: YFINANCE_PROVIDER.into(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            requested_start: request.start.clone(),
            requested_end: request.end.clone(),
            coverage_start,
            coverage_end,
            provenance: NativeProvenance {
                provider: YFINANCE_PROVIDER.into(),
                provider_symbol: request.provider_symbol.clone(),
                native_timeframe: request.timeframe.clone(),
                resampled: false,
            },
            bars,
        })
    }
}

impl ChartResult {
    fn into_bars(self) -> Result<Vec<OhlcvBar>, UnavailableReason> {
        let quote = self
            .indicators
            .quote
            .into_iter()
            .next()
            .ok_or(UnavailableReason::MalformedResponse)?;
        let len = self.timestamp.len();
        if len == 0
            || [
                quote.open.len(),
                quote.high.len(),
                quote.low.len(),
                quote.close.len(),
                quote.volume.len(),
            ]
            .iter()
            .any(|value| *value != len)
        {
            return Err(UnavailableReason::MalformedResponse);
        }
        (0..len)
            .map(|index| {
                let timestamp = chrono::DateTime::from_timestamp(self.timestamp[index], 0)
                    .ok_or(UnavailableReason::MalformedResponse)?
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                Ok(OhlcvBar {
                    timestamp,
                    open: quote.open[index].ok_or(UnavailableReason::MalformedResponse)?,
                    high: quote.high[index].ok_or(UnavailableReason::MalformedResponse)?,
                    low: quote.low[index].ok_or(UnavailableReason::MalformedResponse)?,
                    close: quote.close[index].ok_or(UnavailableReason::MalformedResponse)?,
                    volume: quote.volume[index].ok_or(UnavailableReason::MalformedResponse)?,
                })
            })
            .collect()
    }
}

fn starts_like_html(body: &[u8]) -> bool {
    std::str::from_utf8(body).is_ok_and(|text| {
        let text = text.trim_start().to_ascii_lowercase();
        text.starts_with("<!doctype html") || text.starts_with("<html")
    })
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
    let key: String = key
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
    .any(|marker| key.contains(marker))
}

fn redact_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .filter(|(key, _)| !sensitive_key(key))
                .map(|(key, value)| (key, redact_json(value)))
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(redact_json).collect())
        }
        other => other,
    }
}

#[cfg(test)]
#[path = "yfinance_tests.rs"]
mod yfinance_tests;
