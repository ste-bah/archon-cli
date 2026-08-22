use crate::data_lake::{
    CapabilityEvidence, CapabilityRequest, MAX_PROBE_CANDLES, MAX_PROBE_DURATION,
    MAX_PROBE_RESPONSE_BYTES, NativeFetchRequest, NativeHistory, NativeProvenance, ProbeBudget,
    ProviderAdapter, ProviderDispatcher, UnavailableReason,
};
pub use crate::data_lake::{
    OpenbbClock, OpenbbEnvironment, OpenbbPolygonEvidence, OpenbbRequest, OpenbbResponse,
    OpenbbTransport, OpenbbTransportError, PreparedOpenbbPolygonHistory, ProcessEnvironment,
    SystemOpenbbClock,
};
use crate::ohlcv::{OhlcvBar, validate_bars};
use serde::Deserialize;
use std::sync::Arc;

mod validation;

pub const OPENBB_POLYGON_ROUTE: &str = "/api/v1/equity/price/historical";
pub const OPENBB_DEFAULT_BASE_URL: &str = "http://127.0.0.1:6900";
pub const POLYGON_CREDENTIAL_KEY: &str = "POLYGON_API_KEY";
pub const MAX_INGEST_RESPONSE_BYTES: usize = MAX_PROBE_RESPONSE_BYTES * 64;

pub struct OpenbbPolygonAdapter {
    environment: Arc<dyn OpenbbEnvironment>,
    transport: Arc<dyn OpenbbTransport>,
    clock: Arc<dyn OpenbbClock>,
}

impl OpenbbPolygonAdapter {
    pub fn new(
        environment: Arc<dyn OpenbbEnvironment>,
        transport: Arc<dyn OpenbbTransport>,
        clock: Arc<dyn OpenbbClock>,
    ) -> Self {
        Self {
            environment,
            transport,
            clock,
        }
    }

    pub fn process(transport: Arc<dyn OpenbbTransport>) -> Self {
        Self::new(
            Arc::new(ProcessEnvironment),
            transport,
            Arc::new(SystemOpenbbClock),
        )
    }

    pub fn fetch_for_ingest(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<PreparedOpenbbPolygonHistory, UnavailableReason> {
        self.require_credential()?;
        validate_native_request(request)?;
        let http_request = self.history_request(request)?;
        let response = self.send(&http_request, MAX_INGEST_RESPONSE_BYTES)?;
        self.prepare_history(request, &http_request, response)
    }

    fn require_credential(&self) -> Result<(), UnavailableReason> {
        self.environment
            .value(POLYGON_CREDENTIAL_KEY)
            .filter(|value| !value.trim().is_empty())
            .map(|_| ())
            .ok_or(UnavailableReason::MissingCredentials)
    }

    fn history_request(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<OpenbbRequest, UnavailableReason> {
        supported_interval(&request.timeframe)?;
        Ok(OpenbbRequest {
            method: "GET".into(),
            base_url: resolve_openbb_base_url(self.environment.as_ref())?,
            path: OPENBB_POLYGON_ROUTE.into(),
            query: history_query(request),
        })
    }

    fn send(
        &self,
        request: &OpenbbRequest,
        byte_limit: usize,
    ) -> Result<OpenbbResponse, UnavailableReason> {
        let response = self
            .transport
            .execute(request)
            .map_err(|_| UnavailableReason::ProbeInconclusive)?;
        classify_status(response.status)?;
        if response.body.len() > byte_limit {
            return Err(UnavailableReason::ProbeLimitExceeded);
        }
        if response.content_type.to_ascii_lowercase().contains("html")
            || starts_like_html(&response.body)
        {
            return Err(UnavailableReason::ProviderVerificationBlock);
        }
        Ok(response)
    }

    fn prepare_history(
        &self,
        request: &NativeFetchRequest,
        http_request: &OpenbbRequest,
        response: OpenbbResponse,
    ) -> Result<PreparedOpenbbPolygonHistory, UnavailableReason> {
        let payload: HistoricalPayload = serde_json::from_slice(&response.body)
            .map_err(|_| UnavailableReason::MalformedResponse)?;
        validate_payload(request, &payload)?;
        let bars = payload.results.clone();
        validate_bars(&bars).map_err(|_| UnavailableReason::MalformedResponse)?;
        validation::validate_bar_bounds(request, &bars)?;
        let history = native_history(request, bars)?;
        let retrieved_at = self.clock.now_rfc3339();
        if chrono::DateTime::parse_from_rfc3339(&retrieved_at).is_err() {
            return Err(UnavailableReason::MalformedResponse);
        }
        let raw_response = redact_json(
            serde_json::from_slice(&response.body)
                .map_err(|_| UnavailableReason::MalformedResponse)?,
        );
        Ok(PreparedOpenbbPolygonHistory {
            history,
            raw_response,
            request_provenance: request_provenance(http_request, &retrieved_at),
            redacted_headers: redact_json(response.headers),
            evidence: payload.evidence(retrieved_at),
        })
    }

    fn probe(
        &self,
        request: &CapabilityRequest,
        budget: ProbeBudget,
    ) -> Result<CapabilityEvidence, UnavailableReason> {
        self.require_credential()?;
        validate_probe_budget(budget)?;
        validate_capability_request(request)?;
        supported_interval(&request.timeframe)?;
        let http_request = probe_request(self.environment.as_ref(), request)?;
        let response = self.send(&http_request, budget.max_response_bytes)?;
        let payload: HistoricalPayload = serde_json::from_slice(&response.body)
            .map_err(|_| UnavailableReason::MalformedResponse)?;
        validate_probe_payload(request, &payload)?;
        if payload.results.is_empty() {
            return Err(UnavailableReason::ProbeInconclusive);
        }
        Ok(CapabilityEvidence {
            provider: "polygon".into(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            native_interval: true,
            historical_supported: true,
            current_snapshot_supported: false,
            requires_credentials: true,
            credential_available: true,
            history_horizon: None,
            candles_examined: payload.results.len(),
            response_bytes: response.body.len(),
        })
    }
}

impl ProviderAdapter for OpenbbPolygonAdapter {
    fn provider_id(&self) -> &str {
        "polygon"
    }

    fn probe_capability(
        &self,
        request: &CapabilityRequest,
        budget: ProbeBudget,
    ) -> Result<CapabilityEvidence, UnavailableReason> {
        self.probe(request, budget)
    }

    fn fetch_native_history(
        &self,
        request: &NativeFetchRequest,
    ) -> Result<NativeHistory, UnavailableReason> {
        self.fetch_for_ingest(request)
            .map(|prepared| prepared.history)
    }
}

pub fn register_openbb_polygon(
    dispatcher: &mut ProviderDispatcher,
    adapter: OpenbbPolygonAdapter,
) -> bool {
    dispatcher.register(adapter)
}

pub fn resolve_openbb_base_url(
    environment: &dyn OpenbbEnvironment,
) -> Result<String, UnavailableReason> {
    let host = environment
        .value("OPENBB_HOST")
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = environment
        .value("OPENBB_PORT")
        .unwrap_or_else(|| "6900".into());
    if !valid_host(&host) {
        return Err(UnavailableReason::InvalidRequest);
    }
    let port: u16 = port
        .parse()
        .ok()
        .filter(|port| *port > 0)
        .ok_or(UnavailableReason::InvalidRequest)?;
    Ok(format!("http://{host}:{port}"))
}

fn valid_host(host: &str) -> bool {
    !host.trim().is_empty()
        && host == host.trim()
        && !host.contains(['/', ':', '@'])
        && host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
}

fn supported_interval(value: &str) -> Result<(), UnavailableReason> {
    matches!(value, "1m" | "5m" | "15m" | "30m" | "1h" | "4h" | "1d")
        .then_some(())
        .ok_or(UnavailableReason::ExactNativeIntervalUnsupported)
}

fn validate_native_request(request: &NativeFetchRequest) -> Result<(), UnavailableReason> {
    if request.provider != "polygon"
        || request.canonical_instrument.trim().is_empty()
        || request.provider_symbol.trim().is_empty()
        || request.provider_symbol != request.provider_symbol.trim()
        || request.start >= request.end
        || chrono::DateTime::parse_from_rfc3339(&request.start).is_err()
        || chrono::DateTime::parse_from_rfc3339(&request.end).is_err()
    {
        return Err(UnavailableReason::InvalidRequest);
    }
    Ok(())
}

fn validate_capability_request(request: &CapabilityRequest) -> Result<(), UnavailableReason> {
    if request.provider != "polygon"
        || request.canonical_instrument.trim().is_empty()
        || request.provider_symbol.trim().is_empty()
        || request.provider_symbol != request.provider_symbol.trim()
        || chrono::DateTime::parse_from_rfc3339(&request.checked_at).is_err()
    {
        return Err(UnavailableReason::InvalidRequest);
    }
    Ok(())
}

fn validate_probe_budget(budget: ProbeBudget) -> Result<(), UnavailableReason> {
    (budget.max_candles > 0
        && budget.max_candles <= MAX_PROBE_CANDLES
        && budget.max_duration <= MAX_PROBE_DURATION
        && budget.max_response_bytes <= MAX_PROBE_RESPONSE_BYTES)
        .then_some(())
        .ok_or(UnavailableReason::ProbeLimitExceeded)
}

fn history_query(request: &NativeFetchRequest) -> Vec<(String, String)> {
    vec![
        ("provider".into(), "polygon".into()),
        ("symbol".into(), request.provider_symbol.clone()),
        ("interval".into(), request.timeframe.clone()),
        ("start_date".into(), request.start.clone()),
        ("end_date".into(), request.end.clone()),
        ("adjustment".into(), "splits".into()),
    ]
}

fn probe_request(
    environment: &dyn OpenbbEnvironment,
    request: &CapabilityRequest,
) -> Result<OpenbbRequest, UnavailableReason> {
    Ok(OpenbbRequest {
        method: "GET".into(),
        base_url: resolve_openbb_base_url(environment)?,
        path: OPENBB_POLYGON_ROUTE.into(),
        query: vec![
            ("provider".into(), "polygon".into()),
            ("symbol".into(), request.provider_symbol.clone()),
            ("interval".into(), request.timeframe.clone()),
            ("adjustment".into(), "splits".into()),
            ("limit".into(), MAX_PROBE_CANDLES.to_string()),
        ],
    })
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

fn starts_like_html(body: &[u8]) -> bool {
    std::str::from_utf8(body).is_ok_and(|text| {
        let text = text.trim_start().to_ascii_lowercase();
        text.starts_with("<!doctype html") || text.starts_with("<html")
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoricalPayload {
    provider: String,
    symbol: String,
    timeframe: String,
    adjustment: String,
    corporate_action_source: String,
    exchange: String,
    timezone: String,
    session: String,
    calendar: String,
    asset_class: String,
    license: String,
    requested_start: String,
    requested_end: String,
    expected_bars: u64,
    native: bool,
    derived: bool,
    response_schema: String,
    response_version: String,
    results: Vec<OhlcvBar>,
}

impl HistoricalPayload {
    fn evidence(&self, retrieved_at: String) -> OpenbbPolygonEvidence {
        OpenbbPolygonEvidence {
            transport: "openbb".into(),
            route: OPENBB_POLYGON_ROUTE.into(),
            adjustment: self.adjustment.clone(),
            corporate_action_source: self.corporate_action_source.clone(),
            exchange: self.exchange.clone(),
            timezone: self.timezone.clone(),
            session: self.session.clone(),
            calendar: self.calendar.clone(),
            asset_class: self.asset_class.clone(),
            license: self.license.clone(),
            retrieved_at,
            expected_bars: self.expected_bars,
            response_schema: self.response_schema.clone(),
            response_version: self.response_version.clone(),
        }
    }
}

fn validate_payload(
    request: &NativeFetchRequest,
    payload: &HistoricalPayload,
) -> Result<(), UnavailableReason> {
    validate_identity(&request.provider_symbol, &request.timeframe, payload)?;
    if payload.requested_start != request.start
        || payload.requested_end != request.end
        || payload.expected_bars == 0
        || payload.results.is_empty()
    {
        return Err(UnavailableReason::MalformedResponse);
    }
    Ok(())
}

fn validate_probe_payload(
    request: &CapabilityRequest,
    payload: &HistoricalPayload,
) -> Result<(), UnavailableReason> {
    validate_identity(&request.provider_symbol, &request.timeframe, payload)?;
    if payload.results.len() > MAX_PROBE_CANDLES {
        return Err(UnavailableReason::ProbeLimitExceeded);
    }
    Ok(())
}

fn validate_identity(
    symbol: &str,
    timeframe: &str,
    payload: &HistoricalPayload,
) -> Result<(), UnavailableReason> {
    if payload.provider != "polygon"
        || payload.symbol != symbol
        || payload.timeframe != timeframe
        || payload.adjustment != "splits"
        || payload.corporate_action_source != "polygon"
        || payload.asset_class != "equity"
        || payload.timezone != "America/New_York"
        || payload.session != "regular"
        || payload.calendar != "XNYS"
        || !payload.native
        || payload.derived
        || [
            &payload.exchange,
            &payload.timezone,
            &payload.session,
            &payload.calendar,
            &payload.license,
            &payload.response_schema,
            &payload.response_version,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(UnavailableReason::MalformedResponse);
    }
    Ok(())
}

fn native_history(
    request: &NativeFetchRequest,
    bars: Vec<OhlcvBar>,
) -> Result<NativeHistory, UnavailableReason> {
    let coverage_start = bars
        .first()
        .map(|bar| bar.timestamp.clone())
        .ok_or(UnavailableReason::MalformedResponse)?;
    let coverage_end = bars
        .last()
        .map(|bar| bar.timestamp.clone())
        .ok_or(UnavailableReason::MalformedResponse)?;
    Ok(NativeHistory {
        provider: "polygon".into(),
        canonical_instrument: request.canonical_instrument.clone(),
        provider_symbol: request.provider_symbol.clone(),
        timeframe: request.timeframe.clone(),
        requested_start: request.start.clone(),
        requested_end: request.end.clone(),
        coverage_start,
        coverage_end,
        provenance: NativeProvenance {
            provider: "polygon".into(),
            provider_symbol: request.provider_symbol.clone(),
            native_timeframe: request.timeframe.clone(),
            resampled: false,
        },
        bars,
    })
}

fn request_provenance(request: &OpenbbRequest, retrieved_at: &str) -> serde_json::Value {
    serde_json::json!({
        "transport": "openbb",
        "provider": "polygon",
        "method": request.method,
        "path": request.path,
        "query_fields": request.query.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        "query": request.query.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
        "retrieved_at": retrieved_at,
    })
}

pub(crate) fn redact_json(mut value: serde_json::Value) -> serde_json::Value {
    match &mut value {
        serde_json::Value::Object(fields) => {
            let safe = std::mem::take(fields)
                .into_iter()
                .filter(|(key, _)| !sensitive_key(key))
                .map(|(key, field)| (key, redact_json(field)))
                .collect();
            *fields = safe;
        }
        serde_json::Value::Array(values) => {
            for field in values {
                *field = redact_json(field.take());
            }
        }
        _ => {}
    }
    value
}

fn sensitive_key(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    [
        "apikey",
        "authorization",
        "cookie",
        "password",
        "secret",
        "session",
        "token",
    ]
    .iter()
    .any(|marker| key.contains(marker))
}

#[cfg(test)]
#[path = "openbb_polygon_tests.rs"]
mod openbb_polygon_tests;
