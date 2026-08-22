use super::{
    CapabilityEvidence, CapabilityRequest, NativeFetchRequest, NativeHistory, NativeProvenance,
    ProbeBudget, ProviderAdapter, ProviderDispatcher, UnavailableReason,
};
use crate::ohlcv::{OhlcvBar, validate_bars};
use std::sync::Arc;

pub const STOOQ_DOWNLOAD_URL: &str = "https://stooq.com/q/d/l/";
pub const STOOQ_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StooqRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
}

impl StooqRequest {
    pub fn fingerprint(&self) -> String {
        let canonical = format!("{}?{}", self.url, encode_query(&self.query));
        blake3::hash(canonical.as_bytes()).to_hex().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StooqResponse {
    pub status: u16,
    pub content_type: String,
    pub headers: serde_json::Value,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StooqTransportError {
    Connection,
    Timeout,
}

pub trait StooqTransport: Send + Sync {
    fn execute(&self, request: &StooqRequest) -> Result<StooqResponse, StooqTransportError>;
}

pub trait StooqClock: Send + Sync {
    fn now_rfc3339(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemStooqClock;

impl StooqClock for SystemStooqClock {
    fn now_rfc3339(&self) -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedStooqHistory {
    pub history: NativeHistory,
    pub raw_body: Vec<u8>,
    pub request_provenance: serde_json::Value,
    pub redacted_headers: serde_json::Value,
    pub evidence: StooqNativeEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StooqNativeEvidence {
    pub request_fingerprint: String,
    pub native_interval: String,
    pub retrieved_at: String,
    pub expected_bars: u64,
    pub observed_bars: u64,
}

pub struct StooqAdapter {
    transport: Arc<dyn StooqTransport>,
    clock: Arc<dyn StooqClock>,
}

impl StooqAdapter {
    pub fn new(transport: Arc<dyn StooqTransport>, clock: Arc<dyn StooqClock>) -> Self {
        Self { transport, clock }
    }

    pub fn process(transport: Arc<dyn StooqTransport>) -> Self {
        Self::new(transport, Arc::new(SystemStooqClock))
    }

    pub fn fetch_for_ingest(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<PreparedStooqHistory, UnavailableReason> {
        validate_request(request)?;
        let http_request = history_request(request)?;
        let response = self
            .transport
            .execute(&http_request)
            .map_err(|_| UnavailableReason::ProbeInconclusive)?;
        classify_response(&response)?;
        if response.body.len() > STOOQ_MAX_RESPONSE_BYTES {
            return Err(UnavailableReason::ProbeLimitExceeded);
        }
        let bars = parse_stooq_csv(&response.body)?;
        validate_exact_history(request, &bars)?;
        let retrieved_at = self.clock.now_rfc3339();
        if chrono::DateTime::parse_from_rfc3339(&retrieved_at).is_err() {
            return Err(UnavailableReason::MalformedResponse);
        }
        let expected_bars = bars.len() as u64;
        Ok(PreparedStooqHistory {
            history: native_history(request, bars),
            raw_body: response.body,
            request_provenance: request_provenance(&http_request, &retrieved_at),
            redacted_headers: redact_headers(response.headers),
            evidence: StooqNativeEvidence {
                request_fingerprint: http_request.fingerprint(),
                native_interval: request.timeframe.clone(),
                retrieved_at,
                expected_bars,
                observed_bars: expected_bars,
            },
        })
    }
}

impl ProviderAdapter for StooqAdapter {
    fn provider_id(&self) -> &str {
        "stooq"
    }

    fn probe_capability(
        &self,
        request: &CapabilityRequest,
        budget: ProbeBudget,
    ) -> Result<CapabilityEvidence, UnavailableReason> {
        if budget.max_candles == 0
            || budget.max_duration.is_zero()
            || budget.max_response_bytes == 0
        {
            return Err(UnavailableReason::ProbeLimitExceeded);
        }
        if request.provider != "stooq"
            || request.canonical_instrument.trim().is_empty()
            || request.provider_symbol.trim().is_empty()
            || request.provider_symbol != request.provider_symbol.trim()
            || chrono::DateTime::parse_from_rfc3339(&request.checked_at).is_err()
        {
            return Err(UnavailableReason::InvalidRequest);
        }
        let fetch = NativeFetchRequest {
            provider: request.provider.clone(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            start: request.checked_at.clone(),
            end: request.checked_at.clone(),
        };
        validate_identity(&fetch)?;
        interval_code(&request.timeframe)?;
        Ok(CapabilityEvidence {
            provider: "stooq".into(),
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

pub fn register_stooq(dispatcher: &mut ProviderDispatcher, adapter: StooqAdapter) -> bool {
    dispatcher.register(adapter)
}

fn validate_request(request: &NativeFetchRequest) -> Result<(), UnavailableReason> {
    validate_identity(request)?;
    interval_code(&request.timeframe)?;
    let start = parse_boundary(&request.start)?;
    let end = parse_boundary(&request.end)?;
    (start <= end)
        .then_some(())
        .ok_or(UnavailableReason::InvalidRequest)
}

fn validate_identity(request: &NativeFetchRequest) -> Result<(), UnavailableReason> {
    (request.provider == "stooq"
        && !request.canonical_instrument.trim().is_empty()
        && !request.provider_symbol.trim().is_empty()
        && request.provider_symbol == request.provider_symbol.trim())
    .then_some(())
    .ok_or(UnavailableReason::InvalidRequest)
}

fn history_request(request: &NativeFetchRequest) -> Result<StooqRequest, UnavailableReason> {
    let start = parse_boundary(&request.start)?;
    let end = parse_boundary(&request.end)?;
    Ok(StooqRequest {
        url: STOOQ_DOWNLOAD_URL.into(),
        query: vec![
            ("s".into(), request.provider_symbol.to_ascii_lowercase()),
            ("d1".into(), start.format("%Y%m%d").to_string()),
            ("d2".into(), end.format("%Y%m%d").to_string()),
            ("i".into(), interval_code(&request.timeframe)?.into()),
        ],
    })
}

fn interval_code(timeframe: &str) -> Result<&'static str, UnavailableReason> {
    match timeframe {
        "1d" => Ok("d"),
        "1w" => Ok("w"),
        "1mo" => Ok("m"),
        _ => Err(UnavailableReason::ExactNativeIntervalUnsupported),
    }
}

fn parse_boundary(value: &str) -> Result<chrono::NaiveDate, UnavailableReason> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.date_naive())
        .or_else(|_| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .map_err(|_| UnavailableReason::InvalidRequest)
}

fn classify_response(response: &StooqResponse) -> Result<(), UnavailableReason> {
    match response.status {
        200..=299 => {}
        401 => return Err(UnavailableReason::Unauthorized401),
        403 => return Err(UnavailableReason::ProviderBlocked403),
        404 => return Err(UnavailableReason::NotFound404),
        _ => return Err(UnavailableReason::HttpStatusError),
    }
    let content_type = response.content_type.to_ascii_lowercase();
    if content_type.contains("html") || access_denial_body(&response.body) {
        return Err(UnavailableReason::ProviderVerificationBlock);
    }
    if !matches!(
        content_type.split(';').next().map(str::trim),
        Some("text/csv" | "application/csv" | "text/plain" | "application/octet-stream")
    ) {
        return Err(UnavailableReason::MalformedResponse);
    }
    Ok(())
}

fn access_denial_body(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body)
        .trim_start()
        .to_ascii_lowercase();
    [
        "<!doctype html",
        "<html",
        "access denied",
        "verification required",
        "captcha",
        "cloudflare",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn parse_stooq_csv(body: &[u8]) -> Result<Vec<OhlcvBar>, UnavailableReason> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(body);
    let headers = reader
        .headers()
        .map_err(|_| UnavailableReason::MalformedResponse)?;
    if headers.iter().collect::<Vec<_>>() != ["Date", "Open", "High", "Low", "Close", "Volume"] {
        return Err(UnavailableReason::MalformedResponse);
    }
    let mut bars = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|_| UnavailableReason::MalformedResponse)?;
        if row.len() != 6 {
            return Err(UnavailableReason::MalformedResponse);
        }
        let date = chrono::NaiveDate::parse_from_str(&row[0], "%Y-%m-%d")
            .map_err(|_| UnavailableReason::MalformedResponse)?;
        bars.push(OhlcvBar {
            timestamp: format!("{date}T00:00:00Z"),
            open: parse_number(&row[1])?,
            high: parse_number(&row[2])?,
            low: parse_number(&row[3])?,
            close: parse_number(&row[4])?,
            volume: parse_number(&row[5])?,
        });
    }
    validate_bars(&bars).map_err(|_| UnavailableReason::MalformedResponse)?;
    Ok(bars)
}

fn parse_number(value: &str) -> Result<f64, UnavailableReason> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(UnavailableReason::MalformedResponse)
}

fn validate_exact_history(
    request: &NativeFetchRequest,
    bars: &[OhlcvBar],
) -> Result<(), UnavailableReason> {
    let first = bars.first().ok_or(UnavailableReason::MalformedResponse)?;
    let last = bars.last().ok_or(UnavailableReason::MalformedResponse)?;
    let start = parse_boundary(&request.start)?;
    let end = parse_boundary(&request.end)?;
    if bar_date(first)? != start || bar_date(last)? != end {
        return Err(UnavailableReason::MalformedResponse);
    }
    for pair in bars.windows(2) {
        if interval_gap(&request.timeframe, bar_date(&pair[0])?, bar_date(&pair[1])?) {
            return Err(UnavailableReason::MalformedResponse);
        }
    }
    Ok(())
}

fn bar_date(bar: &OhlcvBar) -> Result<chrono::NaiveDate, UnavailableReason> {
    chrono::DateTime::parse_from_rfc3339(&bar.timestamp)
        .map(|value| value.date_naive())
        .map_err(|_| UnavailableReason::MalformedResponse)
}

fn interval_gap(timeframe: &str, previous: chrono::NaiveDate, next: chrono::NaiveDate) -> bool {
    match interval_code(timeframe) {
        Ok("d") => weekday_distance(previous, next) != 1,
        Ok("w") => (next - previous).num_days() != 7,
        Ok("m") => !next_month(previous, next),
        _ => true,
    }
}

fn weekday_distance(mut previous: chrono::NaiveDate, next: chrono::NaiveDate) -> u32 {
    use chrono::Datelike;
    let mut count = 0;
    while previous < next {
        previous += chrono::Duration::days(1);
        if previous.weekday().number_from_monday() <= 5 {
            count += 1;
        }
    }
    count
}

fn next_month(previous: chrono::NaiveDate, next: chrono::NaiveDate) -> bool {
    use chrono::Datelike;
    let (year, month) = if previous.month() == 12 {
        (previous.year() + 1, 1)
    } else {
        (previous.year(), previous.month() + 1)
    };
    next.year() == year && next.month() == month
}

fn native_history(request: &NativeFetchRequest, bars: Vec<OhlcvBar>) -> NativeHistory {
    NativeHistory {
        provider: "stooq".into(),
        canonical_instrument: request.canonical_instrument.clone(),
        provider_symbol: request.provider_symbol.clone(),
        timeframe: request.timeframe.clone(),
        requested_start: request.start.clone(),
        requested_end: request.end.clone(),
        coverage_start: request.start.clone(),
        coverage_end: request.end.clone(),
        provenance: NativeProvenance {
            provider: "stooq".into(),
            provider_symbol: request.provider_symbol.clone(),
            native_timeframe: request.timeframe.clone(),
            resampled: false,
        },
        bars,
    }
}

fn request_provenance(request: &StooqRequest, retrieved_at: &str) -> serde_json::Value {
    serde_json::json!({
        "method": "GET",
        "url": request.url,
        "query": request.query,
        "request_fingerprint": request.fingerprint(),
        "retrieved_at": retrieved_at,
        "retry_count": 0,
    })
}

fn redact_headers(headers: serde_json::Value) -> serde_json::Value {
    let Some(headers) = headers.as_object() else {
        return serde_json::json!({});
    };
    serde_json::Value::Object(
        headers
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "authorization" | "cookie" | "set-cookie" | "x-api-key"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn encode_query(query: &[(String, String)]) -> String {
    query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}
