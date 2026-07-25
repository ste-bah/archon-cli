use anyhow::{Result, anyhow};
use archon_trading::ohlcv::OhlcvBar;
use serde_json::Value;

pub(super) fn bars_from_openbb_response(body: &[u8]) -> Result<Vec<OhlcvBar>> {
    let value: Value = serde_json::from_slice(body)?;
    if is_yfinance_chart_envelope(&value) {
        return bars_from_yfinance_chart(&value);
    }
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenBB response missing results array"))?;
    let bars = results
        .iter()
        .map(bar_from_openbb_row)
        .collect::<Result<Vec<_>>>()?;
    if bars.is_empty() {
        return Err(anyhow!("OpenBB response contained zero OHLCV rows"));
    }
    Ok(bars)
}

/// Whether this body is a Yahoo Finance chart envelope rather than an OpenBB one.
///
/// Dispatching on `value.get("chart").is_some()` does NOT work: OpenBB emits a
/// top-level `chart` key on every response, set to null unless a chart was
/// requested, and serde_json returns `Some(Value::Null)` for a present-but-null
/// key. That routed every OpenBB response into the Yahoo parser, which then
/// failed on the missing `chart.result` and reported a Yahoo error for whatever
/// provider was actually asked -- while the real rows sat unread in `results`.
///
/// A genuine Yahoo envelope carries an OBJECT at `chart` holding `result` or
/// `error`, and has no OpenBB `results` array.
fn is_yfinance_chart_envelope(value: &Value) -> bool {
    let Some(chart) = value.get("chart").and_then(Value::as_object) else {
        return false;
    };
    chart.contains_key("result") || chart.contains_key("error")
}

fn bars_from_yfinance_chart(value: &Value) -> Result<Vec<OhlcvBar>> {
    if !value["chart"]["error"].is_null() {
        return Err(anyhow!(
            "Yahoo Finance chart API returned error: {}",
            value["chart"]["error"]
        ));
    }
    let result = value["chart"]["result"]
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("Yahoo Finance chart response missing result"))?;
    let timestamps = result["timestamp"]
        .as_array()
        .ok_or_else(|| anyhow!("Yahoo Finance chart response missing timestamps"))?;
    let quote = result["indicators"]["quote"]
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("Yahoo Finance chart response missing quote indicators"))?;
    let bars = timestamps
        .iter()
        .enumerate()
        .filter_map(|(index, timestamp)| yfinance_bar_at(timestamp, quote, index).transpose())
        .collect::<Result<Vec<_>>>()?;
    if bars.is_empty() {
        return Err(anyhow!(
            "Yahoo Finance chart response contained zero OHLCV rows"
        ));
    }
    Ok(bars)
}

fn yfinance_bar_at(timestamp: &Value, quote: &Value, index: usize) -> Result<Option<OhlcvBar>> {
    let Some(seconds) = timestamp.as_i64() else {
        return Ok(None);
    };
    let open = optional_number_at(quote, "open", index);
    let high = optional_number_at(quote, "high", index);
    let low = optional_number_at(quote, "low", index);
    let close = optional_number_at(quote, "close", index);
    let (Some(open), Some(high), Some(low), Some(close)) = (open, high, low, close) else {
        return Ok(None);
    };
    let timestamp = chrono::DateTime::from_timestamp(seconds, 0)
        .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| anyhow!("invalid Yahoo Finance unix timestamp"))?;
    Ok(Some(OhlcvBar {
        timestamp,
        open,
        high,
        low,
        close,
        volume: optional_number_at(quote, "volume", index).unwrap_or(0.0),
    }))
}

fn optional_number_at(row: &Value, field: &str, index: usize) -> Option<f64> {
    row.get(field)?.as_array()?.get(index)?.as_f64()
}

fn bar_from_openbb_row(row: &Value) -> Result<OhlcvBar> {
    Ok(OhlcvBar {
        timestamp: openbb_timestamp(row)?,
        open: number_field(row, "open")?,
        high: number_field(row, "high")?,
        low: number_field(row, "low")?,
        close: number_field(row, "close")?,
        volume: number_field(row, "volume").unwrap_or(0.0),
    })
}

fn openbb_timestamp(row: &Value) -> Result<String> {
    let raw = row
        .get("timestamp")
        .or_else(|| row.get("datetime"))
        .or_else(|| row.get("date"))
        .ok_or_else(|| anyhow!("OpenBB row missing timestamp/date"))?;
    if let Some(seconds) = raw.as_i64() {
        return chrono::DateTime::from_timestamp(seconds, 0)
            .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .ok_or_else(|| anyhow!("invalid OpenBB unix timestamp"));
    }
    let text = raw
        .as_str()
        .ok_or_else(|| anyhow!("OpenBB timestamp/date was not a string"))?
        .trim()
        .replace(' ', "T");
    if text.len() == 10 && text.as_bytes().get(4) == Some(&b'-') {
        return Ok(format!("{text}T00:00:00Z"));
    }
    if text.ends_with('Z') || has_timezone_offset(&text) {
        Ok(text)
    } else {
        Ok(format!("{text}Z"))
    }
}

fn number_field(row: &Value, field: &str) -> Result<f64> {
    row.get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("OpenBB row missing numeric `{field}`"))
}

fn has_timezone_offset(value: &str) -> bool {
    value
        .get(10..)
        .is_some_and(|tail| tail.contains('+') || tail.rfind('-').is_some())
}

#[cfg(test)]
mod openbb_envelope_tests {
    use super::*;

    /// The exact shape OpenBB returns for a provider fetch: a null `chart`
    /// alongside the real rows. Dispatching on key presence sent this to the
    /// Yahoo parser and lost 492 real bars behind a Yahoo error message.
    fn openbb_body(rows: &str) -> Vec<u8> {
        format!(
            r#"{{"id":"x","results":[{rows}],"provider":"polygon","warnings":null,"chart":null,"extra":{{}}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn a_null_chart_key_does_not_route_an_openbb_body_to_the_yahoo_parser() {
        let body = openbb_body(
            r#"{"date":"2024-08-01","open":552.57,"high":554.87,"low":539.43,"close":543.01,"volume":76428732.0}"#,
        );
        let bars = bars_from_openbb_response(&body).expect("openbb body must parse");
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].close, 543.01);
        assert_eq!(bars[0].timestamp, "2024-08-01T00:00:00Z");
    }

    #[test]
    fn a_real_yahoo_chart_envelope_still_routes_to_the_yahoo_parser() {
        let body = br#"{"chart":{"error":null,"result":[{"timestamp":[1722470400],
            "indicators":{"quote":[{"open":[552.57],"high":[554.87],"low":[539.43],
            "close":[543.01],"volume":[76428732]}]}}]}}"#;
        let bars = bars_from_openbb_response(body).expect("yahoo body must parse");
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].close, 543.01);
    }

    #[test]
    fn a_yahoo_error_envelope_is_reported_as_a_yahoo_error() {
        let body = br#"{"chart":{"error":{"code":"Not Found"},"result":null}}"#;
        let error = bars_from_openbb_response(body).expect_err("must fail");
        assert!(error.to_string().contains("Yahoo Finance"), "{error}");
    }

    #[test]
    fn an_openbb_failure_is_never_described_as_a_yahoo_failure() {
        // A polygon request that comes back shaped wrong must not be blamed on
        // Yahoo -- that misdirected every agent debugging the polygon path.
        let body = br#"{"id":"x","results":"not-an-array","provider":"polygon","chart":null}"#;
        let error = bars_from_openbb_response(body).expect_err("must fail");
        assert!(!error.to_string().contains("Yahoo"), "{error}");
        assert!(error.to_string().contains("results array"), "{error}");
    }
}
