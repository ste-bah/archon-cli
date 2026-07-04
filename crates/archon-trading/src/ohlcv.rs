use crate::backtest::EvidenceSource;
use crate::data_lake::DatasetStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvBar {
    #[serde(alias = "ts")]
    pub timestamp: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OhlcvFormat {
    Csv,
    Json,
    Txt,
    Zip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OhlcvBacktestRule {
    CloseMomentum,
    SmaCross,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvDatasetRef {
    pub dataset_id: String,
    pub version: String,
    pub checksum: String,
    pub status: DatasetStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvBacktestRequest {
    pub dataset: OhlcvDatasetRef,
    pub rule: OhlcvBacktestRule,
    pub quantity: f64,
    pub exploratory: bool,
    pub source: EvidenceSource,
    pub fast_len: usize,
    pub slow_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OhlcvError {
    Empty,
    InvalidBar(&'static str),
    DuplicateTimestamp(String),
    InvalidTimestamp(String),
    UnsortedTimestamp(String),
    Csv(String),
    Json(String),
    Zip(String),
}

pub fn parse_ohlcv(input: &[u8], format: OhlcvFormat) -> Result<Vec<OhlcvBar>, OhlcvError> {
    match format {
        OhlcvFormat::Csv => parse_csv(input),
        OhlcvFormat::Json => parse_json(input),
        OhlcvFormat::Txt => parse_json(input).or_else(|_| parse_csv(input)),
        OhlcvFormat::Zip => Err(OhlcvError::Zip(
            "zip OHLCV parsing is not implemented; raw zip artifacts are storage-only".into(),
        )),
    }
}

pub fn validate_bars(bars: &[OhlcvBar]) -> Result<(), OhlcvError> {
    if bars.is_empty() {
        return Err(OhlcvError::Empty);
    }
    let mut seen = BTreeSet::new();
    let mut previous = "";
    for bar in bars {
        validate_bar(bar)?;
        if !seen.insert(bar.timestamp.clone()) {
            return Err(OhlcvError::DuplicateTimestamp(bar.timestamp.clone()));
        }
        if !previous.is_empty() && bar.timestamp.as_str() < previous {
            return Err(OhlcvError::UnsortedTimestamp(bar.timestamp.clone()));
        }
        previous = &bar.timestamp;
    }
    Ok(())
}

pub fn bars_checksum(bars: &[OhlcvBar]) -> String {
    let bytes = serde_json::to_vec(bars).unwrap_or_default();
    bytes_checksum(&bytes)
}

pub fn bytes_checksum(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn coverage_bounds(bars: &[OhlcvBar]) -> Option<(String, String)> {
    Some((
        bars.first()?.timestamp.clone(),
        bars.last()?.timestamp.clone(),
    ))
}

fn parse_csv(input: &[u8]) -> Result<Vec<OhlcvBar>, OhlcvError> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(input);
    let mut bars = Vec::new();
    for row in reader.deserialize::<RawOhlcvBar>() {
        bars.push(row.map_err(|err| OhlcvError::Csv(err.to_string()))?.into());
    }
    validate_bars(&bars)?;
    Ok(bars)
}

fn parse_json(input: &[u8]) -> Result<Vec<OhlcvBar>, OhlcvError> {
    let value: serde_json::Value = serde_json::from_slice(input).or_else(|_| parse_jsonl(input))?;
    let bars: Vec<OhlcvBar> = if let Some(items) = value.get("bars") {
        serde_json::from_value::<Vec<OhlcvBar>>(items.clone())
    } else if value.is_object() {
        serde_json::from_value::<OhlcvBar>(value).map(|bar| vec![bar])
    } else {
        serde_json::from_value::<Vec<OhlcvBar>>(value)
    }
    .map_err(|err| OhlcvError::Json(err.to_string()))?;
    validate_bars(&bars)?;
    Ok(bars)
}

fn parse_jsonl(input: &[u8]) -> Result<serde_json::Value, OhlcvError> {
    let text = std::str::from_utf8(input).map_err(|err| OhlcvError::Json(err.to_string()))?;
    let mut bars = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        bars.push(
            serde_json::from_str::<OhlcvBar>(line)
                .map_err(|err| OhlcvError::Json(err.to_string()))?,
        );
    }
    serde_json::to_value(bars).map_err(|err| OhlcvError::Json(err.to_string()))
}

fn validate_bar(bar: &OhlcvBar) -> Result<(), OhlcvError> {
    if bar.timestamp.trim().is_empty() {
        return Err(OhlcvError::InvalidBar("timestamp"));
    }
    if !is_rfc3339_timestamp(&bar.timestamp) {
        return Err(OhlcvError::InvalidTimestamp(bar.timestamp.clone()));
    }
    positive(bar.open, "open")?;
    positive(bar.high, "high")?;
    positive(bar.low, "low")?;
    positive(bar.close, "close")?;
    if !bar.volume.is_finite() || bar.volume < 0.0 {
        return Err(OhlcvError::InvalidBar("volume"));
    }
    if bar.high < bar.low || bar.high < bar.open || bar.high < bar.close {
        return Err(OhlcvError::InvalidBar("high"));
    }
    if bar.low > bar.open || bar.low > bar.close {
        return Err(OhlcvError::InvalidBar("low"));
    }
    Ok(())
}

fn positive(value: f64, field: &'static str) -> Result<(), OhlcvError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(OhlcvError::InvalidBar(field))
    }
}

fn is_rfc3339_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < "2000-01-01T00:00:00Z".len() {
        return false;
    }
    let fixed = [4, 7, 10, 13, 16];
    if fixed.iter().any(|&index| index >= bytes.len()) {
        return false;
    }
    let separators_ok = bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':';
    if !separators_ok {
        return false;
    }
    let digits_ok = bytes[..19]
        .iter()
        .enumerate()
        .filter(|(index, _)| !fixed.contains(index))
        .all(|(_, byte)| byte.is_ascii_digit());
    let timezone_ok = value.ends_with('Z')
        || value
            .rsplit_once(['+', '-'])
            .is_some_and(|(_, offset)| offset.len() == 5 && offset.as_bytes()[2] == b':');
    digits_ok && timezone_ok
}

#[derive(Debug, Deserialize)]
struct RawOhlcvBar {
    #[serde(alias = "ts", alias = "time", alias = "date", alias = "datetime")]
    timestamp: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    #[serde(alias = "vol")]
    volume: f64,
}

impl From<RawOhlcvBar> for OhlcvBar {
    fn from(value: RawOhlcvBar) -> Self {
        Self {
            timestamp: value.timestamp,
            open: value.open,
            high: value.high,
            low: value.low,
            close: value.close,
            volume: value.volume,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv_with_time_alias() {
        let csv = b"time,open,high,low,close,volume\n2026-01-01T00:00:00Z,10,12,9,11,100\n";
        let bars = parse_ohlcv(csv, OhlcvFormat::Csv).unwrap();
        assert_eq!(bars[0].timestamp, "2026-01-01T00:00:00Z");
        assert_eq!(bars[0].close, 11.0);
    }

    #[test]
    fn parses_jsonl_with_normalized_ts_field() {
        let jsonl = br#"{"ts":"2026-01-01T00:00:00Z","open":10.0,"high":12.0,"low":9.0,"close":11.0,"volume":100.0}
{"ts":"2026-01-02T00:00:00Z","open":11.0,"high":13.0,"low":10.0,"close":12.0,"volume":200.0}
"#;
        let bars = parse_ohlcv(jsonl, OhlcvFormat::Json).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[1].timestamp, "2026-01-02T00:00:00Z");
    }

    #[test]
    fn rejects_invalid_candle_range() {
        let bars = vec![OhlcvBar {
            timestamp: "2026-01-01T00:00:00Z".into(),
            open: 10.0,
            high: 9.0,
            low: 8.0,
            close: 10.0,
            volume: 1.0,
        }];
        assert_eq!(validate_bars(&bars), Err(OhlcvError::InvalidBar("high")));
    }

    #[test]
    fn rejects_non_rfc3339_timestamps() {
        let bars = vec![OhlcvBar {
            timestamp: "2026-01-01".into(),
            open: 10.0,
            high: 11.0,
            low: 9.0,
            close: 10.0,
            volume: 1.0,
        }];
        assert_eq!(
            validate_bars(&bars),
            Err(OhlcvError::InvalidTimestamp("2026-01-01".into()))
        );
    }
}
