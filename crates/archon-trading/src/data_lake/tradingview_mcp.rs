use crate::data_lake::CurrentSnapshot;
use crate::ohlcv::{OhlcvBar, validate_bars};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const TV_HEALTH_CHECK: &str = "tv_health_check";
pub const TV_CHART_GET_STATE: &str = "chart_get_state";
pub const TV_DATA_GET_OHLCV: &str = "data_get_ohlcv";
pub const TV_QUOTE_GET: &str = "quote_get";
pub const TRADINGVIEW_MAX_BARS_PER_CALL: u64 = 500;
pub const TRADINGVIEW_NATIVE_TIMEFRAMES: &[&str] = &["1W", "1D", "240", "60", "15"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingViewHistoryRequest {
    pub canonical_instrument: String,
    pub provider_symbol: String,
    pub timeframe: String,
    pub start: String,
    pub end: String,
    pub expected_bars: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingViewNativeHistory {
    pub request: TradingViewHistoryRequest,
    pub bars: Vec<OhlcvBar>,
    pub raw_response: Value,
    pub returned_bars: u64,
    pub action_counts: TradingViewMcpActionCounts,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingViewMcpActionCounts {
    pub health_check: u8,
    pub chart_get_state: u8,
    pub data_get_ohlcv: u8,
    pub quote_get: u8,
}

impl TradingViewMcpActionCounts {
    pub fn exact_history_sequence() -> Self {
        Self {
            health_check: 1,
            chart_get_state: 1,
            data_get_ohlcv: 1,
            quote_get: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingViewMcpError {
    InvalidIdentity(String),
    UnsupportedTimeframe(String),
    InvalidBounds,
    UnboundedRequest(u64),
    Transport {
        action: &'static str,
        reason: String,
    },
    Unhealthy,
    ForeignIdentity,
    LimitedCoverage {
        expected: u64,
        returned: u64,
    },
    InvalidPayload(String),
}

pub trait TradingViewMcpTransport {
    fn call(&mut self, action: &'static str, arguments: Value) -> Result<Value, String>;
}

pub struct TradingViewMcpDataAdapter<T> {
    transport: T,
}

impl<T: TradingViewMcpTransport> TradingViewMcpDataAdapter<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn fetch_native_history(
        &mut self,
        request: TradingViewHistoryRequest,
    ) -> Result<TradingViewNativeHistory, TradingViewMcpError> {
        validate_request(&request)?;
        let health = self.call(TV_HEALTH_CHECK, json!({}))?;
        require_health(&health)?;
        let state = self.call(TV_CHART_GET_STATE, json!({}))?;
        require_chart_state(&state)?;
        let response = self.call(
            TV_DATA_GET_OHLCV,
            json!({
                "symbol": request.provider_symbol,
                "timeframe": request.timeframe,
                "count": request.expected_bars,
                "summary": false,
            }),
        )?;
        let mut history = parse_history(request, response)?;
        history.action_counts = TradingViewMcpActionCounts::exact_history_sequence();
        Ok(history)
    }

    pub fn fetch_quote_snapshot(
        &mut self,
        canonical_instrument: &str,
        provider_symbol: &str,
        captured_at_unix_seconds: i64,
    ) -> Result<CurrentSnapshot, TradingViewMcpError> {
        validate_symbol_identity(canonical_instrument, provider_symbol)?;
        let health = self.call(TV_HEALTH_CHECK, json!({}))?;
        require_health(&health)?;
        let state = self.call(TV_CHART_GET_STATE, json!({}))?;
        require_chart_identity(&state, provider_symbol, None)?;
        let payload = self.call(TV_QUOTE_GET, json!({ "symbol": provider_symbol }))?;
        reject_sensitive_provider_material(&payload)?;
        require_quote(&payload, provider_symbol)?;
        Ok(CurrentSnapshot {
            provider: "tradingview".into(),
            canonical_instrument: canonical_instrument.into(),
            provider_symbol: provider_symbol.into(),
            captured_at_unix_seconds,
            payload,
        })
    }

    fn call(
        &mut self,
        action: &'static str,
        arguments: Value,
    ) -> Result<Value, TradingViewMcpError> {
        self.transport
            .call(action, arguments)
            .map_err(|reason| TradingViewMcpError::Transport { action, reason })
    }
}

fn validate_request(request: &TradingViewHistoryRequest) -> Result<(), TradingViewMcpError> {
    validate_symbol_identity(&request.canonical_instrument, &request.provider_symbol)?;
    if !TRADINGVIEW_NATIVE_TIMEFRAMES.contains(&request.timeframe.as_str()) {
        return Err(TradingViewMcpError::UnsupportedTimeframe(
            request.timeframe.clone(),
        ));
    }
    if request.start.is_empty() || request.end.is_empty() || request.start > request.end {
        return Err(TradingViewMcpError::InvalidBounds);
    }
    if request.expected_bars == 0 || request.expected_bars > TRADINGVIEW_MAX_BARS_PER_CALL {
        return Err(TradingViewMcpError::UnboundedRequest(request.expected_bars));
    }
    Ok(())
}

fn validate_symbol_identity(
    canonical_instrument: &str,
    provider_symbol: &str,
) -> Result<(), TradingViewMcpError> {
    let expected = tradingview_symbol(canonical_instrument)
        .ok_or_else(|| TradingViewMcpError::InvalidIdentity(canonical_instrument.to_string()))?;
    if provider_symbol.as_bytes() != expected.as_bytes() {
        return Err(TradingViewMcpError::InvalidIdentity(
            provider_symbol.to_string(),
        ));
    }
    Ok(())
}

pub fn tradingview_symbol(canonical_instrument: &str) -> Option<&'static str> {
    match canonical_instrument {
        "ES" => Some("CME_MINI:ES1!"),
        "NQ" => Some("CME_MINI:NQ1!"),
        "SPY" => Some("SPY"),
        "QQQ" => Some("QQQ"),
        "BTCUSDT" => Some("BINANCE:BTCUSDT"),
        "ETHUSDT" => Some("BINANCE:ETHUSDT"),
        _ => None,
    }
}

fn require_health(value: &Value) -> Result<(), TradingViewMcpError> {
    let healthy = value.get("success").and_then(Value::as_bool) == Some(true)
        && value.get("cdp_connected").and_then(Value::as_bool) == Some(true)
        && value.get("api_available").and_then(Value::as_bool) == Some(true);
    if healthy {
        Ok(())
    } else {
        Err(TradingViewMcpError::Unhealthy)
    }
}

fn require_chart_state(value: &Value) -> Result<(), TradingViewMcpError> {
    require_chart_identity(value, "", None)
}

fn require_chart_identity(
    value: &Value,
    symbol: &str,
    timeframe: Option<&str>,
) -> Result<(), TradingViewMcpError> {
    let actual_symbol = value.get("symbol").and_then(Value::as_str);
    let actual_timeframe = value.get("resolution").and_then(Value::as_str);
    let available = value.get("success").and_then(Value::as_bool) == Some(true)
        && actual_symbol.is_some()
        && actual_timeframe.is_some();
    if !available {
        return Err(TradingViewMcpError::Unhealthy);
    }
    if (!symbol.is_empty() && actual_symbol != Some(symbol))
        || timeframe.is_some_and(|expected| actual_timeframe != Some(expected))
    {
        return Err(TradingViewMcpError::ForeignIdentity);
    }
    Ok(())
}

fn require_quote(value: &Value, symbol: &str) -> Result<(), TradingViewMcpError> {
    if value.get("success").and_then(Value::as_bool) != Some(true)
        || value.get("symbol").and_then(Value::as_str) != Some(symbol)
    {
        return Err(TradingViewMcpError::ForeignIdentity);
    }
    let valid_price = ["last", "open", "high", "low", "close"]
        .iter()
        .filter_map(|field| value.get(field).and_then(Value::as_f64))
        .any(|price| price.is_finite() && price > 0.0);
    valid_price.then_some(()).ok_or_else(|| {
        TradingViewMcpError::InvalidPayload("quote has no finite positive price".into())
    })
}

fn require_history_identity(
    value: &Value,
    request: &TradingViewHistoryRequest,
) -> Result<(), TradingViewMcpError> {
    let symbol = value.get("actual_symbol").and_then(Value::as_str);
    let timeframe = value.get("actual_timeframe").and_then(Value::as_str);
    if symbol == Some(request.provider_symbol.as_str())
        && timeframe == Some(request.timeframe.as_str())
    {
        Ok(())
    } else {
        Err(TradingViewMcpError::ForeignIdentity)
    }
}

fn parse_history(
    request: TradingViewHistoryRequest,
    response: Value,
) -> Result<TradingViewNativeHistory, TradingViewMcpError> {
    reject_sensitive_provider_material(&response)?;
    let success = response.get("success").and_then(Value::as_bool) == Some(true);
    if !success {
        return Err(TradingViewMcpError::InvalidPayload(
            "OHLCV action did not report success".into(),
        ));
    }
    require_history_identity(&response, &request)?;
    let bars_value = response
        .get("bars")
        .cloned()
        .ok_or_else(|| TradingViewMcpError::InvalidPayload("missing bars".into()))?;
    let native_bars: Vec<TradingViewBar> = serde_json::from_value(bars_value)
        .map_err(|error| TradingViewMcpError::InvalidPayload(error.to_string()))?;
    let bars = native_bars
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;
    validate_bars(&bars)
        .map_err(|error| TradingViewMcpError::InvalidPayload(format!("{error:?}")))?;
    let returned = bars.len() as u64;
    let reported = response.get("bar_count").and_then(Value::as_u64);
    if returned != request.expected_bars || reported != Some(returned) {
        return Err(TradingViewMcpError::LimitedCoverage {
            expected: request.expected_bars,
            returned,
        });
    }
    require_requested_bounds(&bars, &request)?;
    Ok(TradingViewNativeHistory {
        request,
        bars,
        raw_response: response,
        returned_bars: returned,
        action_counts: TradingViewMcpActionCounts::default(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TradingViewBar {
    time: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl TryFrom<TradingViewBar> for OhlcvBar {
    type Error = TradingViewMcpError;

    fn try_from(value: TradingViewBar) -> Result<Self, Self::Error> {
        let timestamp = DateTime::<Utc>::from_timestamp(value.time, 0)
            .ok_or_else(|| TradingViewMcpError::InvalidPayload("invalid bar time".into()))?
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        Ok(Self {
            timestamp,
            open: value.open,
            high: value.high,
            low: value.low,
            close: value.close,
            volume: value.volume,
        })
    }
}

fn require_requested_bounds(
    bars: &[OhlcvBar],
    request: &TradingViewHistoryRequest,
) -> Result<(), TradingViewMcpError> {
    let bounds = bars
        .first()
        .zip(bars.last())
        .map(|(first, last)| (first.timestamp.as_str(), last.timestamp.as_str()));
    (bounds == Some((request.start.as_str(), request.end.as_str())))
        .then_some(())
        .ok_or(TradingViewMcpError::ForeignIdentity)
}

fn reject_sensitive_provider_material(value: &Value) -> Result<(), TradingViewMcpError> {
    let text = value.to_string().to_ascii_lowercase();
    let forbidden = [
        "authorization",
        "bearer ",
        "cookie",
        "session",
        "target_id",
        "targetid",
        "cdp",
        "websocketdebuggerurl",
        "api_key",
        "apikey",
        "password",
        "secret",
        "token",
        "file://",
        "/users/",
        "/home/",
    ];
    if forbidden.iter().any(|needle| text.contains(needle)) {
        Err(TradingViewMcpError::InvalidPayload(
            "sensitive provider material rejected".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "tradingview_mcp_tests.rs"]
mod tests;
