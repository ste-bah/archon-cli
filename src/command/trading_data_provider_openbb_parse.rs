use anyhow::{Result, anyhow};
use archon_trading::ohlcv::OhlcvBar;
use serde_json::Value;

pub(super) fn bars_from_openbb_response(body: &[u8]) -> Result<Vec<OhlcvBar>> {
    let value: Value = serde_json::from_slice(body)?;
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
