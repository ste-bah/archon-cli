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
    ["apikey", "authorization", "cookie", "password", "secret", "session", "token"]
        .iter()
        .any(|marker| key.contains(marker))
}

