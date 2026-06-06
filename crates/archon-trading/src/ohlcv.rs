use crate::backtest::EvidenceSource;
use crate::data_lake::DatasetStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvBar {
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
    UnsortedTimestamp(String),
    Csv(String),
    Json(String),
}

pub fn parse_ohlcv(input: &[u8], format: OhlcvFormat) -> Result<Vec<OhlcvBar>, OhlcvError> {
    match format {
        OhlcvFormat::Csv => parse_csv(input),
        OhlcvFormat::Json => parse_json(input),
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
    blake3::hash(&bytes).to_hex().to_string()
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
    let value: serde_json::Value =
        serde_json::from_slice(input).map_err(|err| OhlcvError::Json(err.to_string()))?;
    let bars: Vec<OhlcvBar> = if let Some(items) = value.get("bars") {
        serde_json::from_value::<Vec<OhlcvBar>>(items.clone())
    } else {
        serde_json::from_value::<Vec<OhlcvBar>>(value)
    }
    .map_err(|err| OhlcvError::Json(err.to_string()))?;
    validate_bars(&bars)?;
    Ok(bars)
}

fn validate_bar(bar: &OhlcvBar) -> Result<(), OhlcvError> {
    if bar.timestamp.trim().is_empty() {
        return Err(OhlcvError::InvalidBar("timestamp"));
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

#[derive(Debug, Deserialize)]
struct RawOhlcvBar {
    #[serde(alias = "time", alias = "date", alias = "datetime")]
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
        let csv = b"time,open,high,low,close,volume\n2026-01-01,10,12,9,11,100\n";
        let bars = parse_ohlcv(csv, OhlcvFormat::Csv).unwrap();
        assert_eq!(bars[0].timestamp, "2026-01-01");
        assert_eq!(bars[0].close, 11.0);
    }

    #[test]
    fn rejects_invalid_candle_range() {
        let bars = vec![OhlcvBar {
            timestamp: "2026-01-01".into(),
            open: 10.0,
            high: 9.0,
            low: 8.0,
            close: 10.0,
            volume: 1.0,
        }];
        assert_eq!(validate_bars(&bars), Err(OhlcvError::InvalidBar("high")));
    }
}
