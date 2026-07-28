use anyhow::{Context, Result, anyhow};
use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, GapSummary,
};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat};
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::project_root;

use super::data_error;

pub(super) fn fetch_native(
    target: Option<&PathBuf>,
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
) -> Result<String> {
    if !provider.trim().eq_ignore_ascii_case("yfinance") {
        return Err(anyhow!(
            "native yfinance handler received provider {provider}"
        ));
    }
    let root = project_root(target)?;
    let interval = match yahoo_interval(timeframe) {
        Ok(interval) => interval,
        Err(err) => {
            return write_or_render(
                &json!({
                    "provider": "yfinance",
                    "symbol": symbol,
                    "timeframe": timeframe,
                    "dataset_id": dataset_id,
                    "can_fetch": false,
                    "quality_status": "degraded_fallback",
                    "production_eligible": false,
                    "provider_blocked_or_unavailable": true,
                    "unavailable_reason": err.to_string(),
                    "fail_closed_behavior": "unsupported yfinance intervals do not write dataset artifacts or registry entries"
                }),
                None,
            );
        }
    };
    let (period1, period2) = yahoo_periods(start, end)?;
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={period1}&period2={period2}&interval={interval}&events=history&includeAdjustedClose=true",
        symbol.trim()
    );
    let fetched_at = Utc::now().to_rfc3339();
    let (status, raw_body) = fetch_chart_response(&url)
        .with_context(|| format!("live yfinance request failed for {symbol}"))?;
    if status >= 400 {
        return Err(anyhow!(
            "live yfinance request failed closed with HTTP {status}"
        ));
    }
    let raw_json: serde_json::Value = serde_json::from_slice(&raw_body)
        .context("live yfinance response was not valid chart JSON")?;
    let bars = parse_chart_bars(&raw_json)?;
    let version = format!("{}-native_yfinance_{}", start.replace('-', ""), timeframe);
    let metadata = metadata(
        dataset_id,
        &version,
        symbol,
        timeframe,
        start,
        end,
        bars.len() as u64,
    );
    let record = TradingDataLake::new(root).store_ohlcv(StoreOhlcvRequest {
        metadata,
        bars,
        raw_body,
        raw_format: OhlcvFormat::Json,
        raw_request: json!({
            "url": url,
            "method": "GET",
            "params": {
                "symbol": symbol,
                "start_date": start,
                "end_date": end,
                "period1": period1,
                "period2": period2,
                "interval": interval,
                "events": "history",
                "includeAdjustedClose": true
            },
            "response_status": status,
            "fetched_at": fetched_at
        }),
        redacted_headers: json!({"user-agent": "archon-cli yfinance fallback diagnostic ingest"}),
        provider_notes: provider_notes(symbol, timeframe, start, end, status),
        created_at: fetched_at,
    })
    .map_err(data_error)?;
    write_or_render(&record, None)
}

fn fetch_chart_response(url: &str) -> Result<(u16, Vec<u8>)> {
    let response = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            r#"import sys, urllib.request
url = sys.argv[1]
request = urllib.request.Request(url, headers={'User-Agent': 'archon-cli yfinance fallback diagnostic ingest'})
try:
    with urllib.request.urlopen(request, timeout=30) as response:
        sys.stderr.write(str(response.status))
        sys.stdout.buffer.write(response.read())
except urllib.error.HTTPError as error:
    sys.stderr.write(str(error.code))
    sys.stdout.buffer.write(error.read())
"#,
        )
        .arg(url)
        .output()
        .context("failed to execute python3 yfinance HTTP helper")?;
    let status_text = String::from_utf8_lossy(&response.stderr);
    let status = status_text.trim().parse::<u16>().unwrap_or(599);
    if !response.status.success() && response.stdout.is_empty() {
        return Err(anyhow!("yfinance HTTP helper failed with status {status}"));
    }
    Ok((status, response.stdout))
}

fn yahoo_interval(timeframe: &str) -> Result<&'static str> {
    match timeframe.trim() {
        "1D" | "1d" => Ok("1d"),
        "1W" | "1w" => Ok("1wk"),
        "1H" | "1h" | "60" => Ok("1h"),
        "15m" | "15M" | "15" => Ok("15m"),
        "4H" | "4h" | "240" => Ok("1h"),
        other => Err(anyhow!(
            "unsupported native timeframe `{other}` for yfinance"
        )),
    }
}

fn yahoo_periods(start: &str, end: &str) -> Result<(i64, i64)> {
    let start = parse_date(start)?;
    let end_exclusive = parse_date(end)? + Duration::days(1);
    Ok((
        Utc.from_utc_datetime(&start.and_hms_opt(0, 0, 0).unwrap())
            .timestamp(),
        Utc.from_utc_datetime(&end_exclusive.and_hms_opt(0, 0, 0).unwrap())
            .timestamp(),
    ))
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value.get(..10).unwrap_or(value), "%Y-%m-%d")
        .with_context(|| format!("invalid yfinance date `{value}`"))
}

fn parse_chart_bars(raw: &serde_json::Value) -> Result<Vec<OhlcvBar>> {
    let result = raw["chart"]["result"][0]
        .as_object()
        .context("missing chart result")?;
    let timestamps = result["timestamp"]
        .as_array()
        .context("missing chart timestamps")?;
    let quote = &result["indicators"]["quote"][0];
    let mut bars = Vec::new();
    for (index, timestamp) in timestamps.iter().enumerate() {
        let Some(timestamp) = timestamp.as_i64() else {
            continue;
        };
        let Some(open) = quote["open"][index].as_f64() else {
            continue;
        };
        let Some(high) = quote["high"][index].as_f64() else {
            continue;
        };
        let Some(low) = quote["low"][index].as_f64() else {
            continue;
        };
        let Some(close) = quote["close"][index].as_f64() else {
            continue;
        };
        let volume = quote["volume"][index].as_f64().unwrap_or(0.0);
        bars.push(OhlcvBar {
            timestamp: Utc
                .timestamp_opt(timestamp, 0)
                .single()
                .unwrap()
                .to_rfc3339(),
            open,
            high,
            low,
            close,
            volume,
        });
    }
    if bars.is_empty() {
        return Err(anyhow!(
            "live yfinance response contained no complete OHLCV bars"
        ));
    }
    Ok(bars)
}

fn metadata(
    dataset_id: &str,
    version: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    observed: u64,
) -> DatasetMetadata {
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v2".into(),
        dataset_id: dataset_id.into(),
        version: version.into(),
        canonical_instrument: symbol.into(),
        asset_class: "equity".into(),
        provider: "yfinance".into(),
        provider_symbol: symbol.into(),
        timeframe: timeframe.into(),
        native_interval: true,
        production_eligible: false,
        price_basis: "raw".into(),
        session: "regular".into(),
        data_type: DataType::Ohlcv,
        symbol_map: BTreeMap::from([(symbol.into(), symbol.into())]),
        timezone: "UTC".into(),
        adjustment: "raw".into(),
        license: "Yahoo Finance terms; diagnostic fallback only".into(),
        coverage: CoverageWindow {
            start: start.into(),
            end: end.into(),
            expected_bars: observed,
            observed_bars: observed,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: observed,
        },
        checksum: String::new(),
        checksums: DatasetChecksums::default(),
        paths: DatasetArtifactPaths::default(),
        source: DatasetSourceMetadata {
            license_notes: "Yahoo Finance diagnostic fallback; not production eligible".into(),
            url_or_endpoint: "https://query1.finance.yahoo.com/v8/finance/chart".into(),
            retrieved_at: String::new(),
            credential_required: false,
        },
        quality_status: "degraded".into(),
        created_at: String::new(),
        optional: true,
    }
}

fn provider_notes(symbol: &str, timeframe: &str, start: &str, end: &str, status: u16) -> String {
    format!(
        "# yfinance fallback diagnostic ingest\n\nLive Yahoo Finance chart request for {symbol} {timeframe} from {start} through {end}. HTTP status {status}. Dataset is degraded, diagnostic-only, and ineligible for production promotion. No credentials were used or stored.\n"
    )
}
