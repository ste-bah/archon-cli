use crate::data_lake::{NativeFetchRequest, UnavailableReason};
use crate::ohlcv::OhlcvBar;

pub(super) fn validate_bar_bounds(
    request: &NativeFetchRequest,
    bars: &[OhlcvBar],
) -> Result<(), UnavailableReason> {
    let start = parse_bound(&request.start)?;
    let end = parse_bound(&request.end)?;
    for bar in bars {
        let timestamp = chrono::DateTime::parse_from_rfc3339(&bar.timestamp)
            .map_err(|_| UnavailableReason::MalformedResponse)?;
        if timestamp < start || timestamp > end {
            return Err(UnavailableReason::MalformedResponse);
        }
    }
    Ok(())
}

fn parse_bound(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, UnavailableReason> {
    chrono::DateTime::parse_from_rfc3339(value).map_err(|_| UnavailableReason::MalformedResponse)
}
