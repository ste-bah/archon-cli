use anyhow::{Result, anyhow};

const TRADINGVIEW_MCP_MAX_BARS_PER_CALL: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TradingViewRequestSpan {
    pub(super) requested_bars: usize,
    pub(super) per_call_limit: usize,
}

pub(super) fn requested_span(
    start: &str,
    end: &str,
    timeframe: &str,
) -> Result<TradingViewRequestSpan> {
    let start_ts = parse_boundary(start, true)?;
    let end_ts = parse_boundary(end, false)?;
    if end_ts < start_ts {
        return Err(anyhow!("TradingView requested span end is before start"));
    }
    let step = timeframe_seconds(timeframe)?;
    let duration = end_ts - start_ts;
    let requested = (duration / step) + 1;
    let requested_bars = usize::try_from(requested)
        .map_err(|_| anyhow!("TradingView requested span is too large"))?;
    Ok(TradingViewRequestSpan {
        requested_bars: requested_bars.max(1),
        per_call_limit: requested_bars.clamp(1, TRADINGVIEW_MCP_MAX_BARS_PER_CALL),
    })
}

fn parse_boundary(value: &str, start_of_day: bool) -> Result<i64> {
    let text = value.trim();
    if text.is_empty() {
        return Err(anyhow!("TradingView requested span boundary is empty"));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let time = if start_of_day {
            chrono::NaiveTime::MIN
        } else {
            chrono::NaiveTime::from_hms_opt(23, 59, 59).expect("valid end-of-day time")
        };
        return Ok(date.and_time(time).and_utc().timestamp());
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|parsed| parsed.timestamp())
        .map_err(|err| anyhow!("invalid TradingView requested span boundary `{text}`: {err}"))
}

fn timeframe_seconds(timeframe: &str) -> Result<i64> {
    match timeframe.trim() {
        "15" => Ok(15 * 60),
        "60" => Ok(60 * 60),
        "240" => Ok(240 * 60),
        "1D" => Ok(24 * 60 * 60),
        "1W" => Ok(7 * 24 * 60 * 60),
        value => Err(anyhow!(
            "TradingView exact native timeframe `{value}` is unsupported"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::requested_span;

    #[test]
    fn derives_daily_count_from_inclusive_date_span() {
        let span = requested_span("2024-01-01", "2024-01-05", "1D").unwrap();
        assert_eq!(span.requested_bars, 5);
        assert_eq!(span.per_call_limit, 5);
    }

    #[test]
    fn caps_per_call_without_lowering_requested_span() {
        let span = requested_span("2024-01-01", "2026-01-01", "1D").unwrap();
        assert!(span.requested_bars > 500);
        assert_eq!(span.per_call_limit, 500);
    }
}
