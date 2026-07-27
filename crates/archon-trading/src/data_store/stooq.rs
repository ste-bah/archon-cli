use crate::data_lake::DatasetMetadata;
use chrono::Datelike;

pub(super) fn fail_closed_stooq_short_span_metadata(
    metadata: &mut DatasetMetadata,
    raw_request: &serde_json::Value,
) {
    if !metadata.production_eligible
        || !metadata.provider.eq_ignore_ascii_case("stooq")
        || metadata.timeframe != "1D"
    {
        return;
    }
    let Some(expected_bars) = stooq_requested_weekday_bars(raw_request) else {
        return;
    };
    let observed = metadata.coverage.observed_bars;
    metadata.coverage.expected_bars = expected_bars;
    metadata.gaps.expected_bars = expected_bars;
    metadata.gaps.missing_bars = expected_bars.saturating_sub(observed);
    if observed.saturating_mul(100) < expected_bars.saturating_mul(90) {
        metadata.production_eligible = false;
        metadata.quality_status = "degraded".into();
    }
}

fn stooq_requested_weekday_bars(raw_request: &serde_json::Value) -> Option<u64> {
    let start = parse_stooq_request_date(raw_request.get("start")?.as_str()?)?;
    let end = parse_stooq_request_date(raw_request.get("end")?.as_str()?)?;
    if end < start {
        return None;
    }
    let mut day = start;
    let mut count = 0;
    while day <= end {
        if day.weekday().number_from_monday() <= 5 {
            count += 1;
        }
        day = day.succ_opt()?;
    }
    Some(count)
}

fn parse_stooq_request_date(value: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(&value[..value.len().min(10)], "%Y-%m-%d").ok()
}
