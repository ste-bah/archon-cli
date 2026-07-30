use anyhow::Result;
use serde_json::json;
use std::path::Path;

use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetChecksums, DatasetMetadata, DatasetSourceMetadata, GapSummary,
};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat, parse_ohlcv};

use crate::command::trading_data::data_error;
use crate::command::trading_data_provider::unavailable_provider_report;
use crate::command::trading_io::write_or_render;

pub(super) fn fetch_stooq_native(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
) -> Result<String> {
    if normalize_timeframe(timeframe) != "1D" {
        return stooq_unavailable_report(
            root,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            "exact native Stooq interval is unavailable; resampling is refused",
        );
    }
    let provider_symbol = stooq_provider_symbol(symbol);
    let source = stooq_source(&provider_symbol, start, end);
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let (headers, body) = match fetch_stooq_response(&source.url) {
        Ok(parts) => parts,
        Err(reason) => {
            return stooq_unavailable_report(
                root, symbol, timeframe, start, end, dataset_id, &reason,
            );
        }
    };
    let bars = match parse_stooq_csv(&body) {
        Ok(bars) => bars,
        Err(reason) => {
            return stooq_unavailable_report(
                root,
                symbol,
                timeframe,
                start,
                end,
                dataset_id,
                &format!("Stooq CSV parse failed: {reason:?}"),
            );
        }
    };
    store_stooq_dataset(
        root,
        StooqStoreInput {
            symbol,
            provider_symbol: &provider_symbol,
            start,
            end,
            dataset_id,
            url: &source.url,
            source_is_live_stooq: source.is_live_stooq,
            headers,
            body,
            bars,
            fetched_at,
        },
    )
}

struct StooqStoreInput<'a> {
    symbol: &'a str,
    provider_symbol: &'a str,
    start: &'a str,
    end: &'a str,
    dataset_id: &'a str,
    url: &'a str,
    source_is_live_stooq: bool,
    headers: serde_json::Value,
    body: Vec<u8>,
    bars: Vec<OhlcvBar>,
    fetched_at: String,
}

fn fetch_stooq_response(url: &str) -> Result<(serde_json::Value, Vec<u8>), String> {
    let url = url.to_string();
    std::thread::spawn(move || fetch_stooq_response_blocking(&url))
        .join()
        .map_err(|_| "Stooq fetch worker panicked".to_string())?
}

fn fetch_stooq_response_blocking(url: &str) -> Result<(serde_json::Value, Vec<u8>), String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            "archon-cli trading-data stooq-native",
        )
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let headers = stooq_redacted_headers(response.headers());
    let body = response
        .bytes()
        .map_err(|error| error.to_string())?
        .to_vec();
    if !status.is_success() || stooq_body_is_non_data(&body) {
        return Err(format!(
            "provider returned non-data response status {status}"
        ));
    }
    Ok((headers, body))
}

fn store_stooq_dataset(root: &Path, input: StooqStoreInput<'_>) -> Result<String> {
    let record = TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata: stooq_metadata(
                input.dataset_id,
                input.symbol,
                input.provider_symbol,
                &input.bars,
                input.source_is_live_stooq,
            ),
            bars: input.bars,
            raw_body: input.body,
            raw_format: OhlcvFormat::Csv,
            raw_request: json!({ "provider": "stooq", "url": input.url, "symbol": input.symbol, "provider_symbol": input.provider_symbol, "timeframe": "1D", "start": input.start, "end": input.end, "source_is_live_stooq": input.source_is_live_stooq }),
            redacted_headers: input.headers,
            provider_notes: "Stooq exact native daily CSV; interval=1D; no bot-detection bypass; no resampling; credentials not required.".into(),
            created_at: input.fetched_at,
        })
        .map_err(data_error)?;
    persist_stooq_capability(
        root,
        input.symbol,
        input.provider_symbol,
        record.production_eligible,
    )?;
    write_or_render(
        &json!({
            "provider": "stooq", "symbol": input.symbol, "timeframe": "1D", "start": input.start, "end": input.end,
            "dataset_id": input.dataset_id, "can_fetch": record.production_eligible, "native_interval": true,
            "quality_status": record.status, "production_eligible": record.production_eligible, "stored_ohlcv": record,
            "source_is_live_stooq": input.source_is_live_stooq,
            "fail_closed_behavior": "dataset artifacts are written after CSV validation; ARCHON_STOOQ_CSV_URL override output is explicitly non-production and cannot satisfy production deliverables"
        }),
        None,
    )
}

fn stooq_unavailable_report(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    reason: &str,
) -> Result<String> {
    let report =
        unavailable_provider_report(root, "stooq", symbol, timeframe, start, end, dataset_id)?;
    persist_stooq_unavailable_capability(root, symbol, timeframe, reason)?;
    let manifest_path =
        write_stooq_unavailable_manifest(root, symbol, timeframe, start, end, dataset_id, reason)?;
    Ok(stooq_report_with_fetch_error(
        report,
        reason,
        Some(manifest_path.display().to_string()),
    ))
}

fn write_stooq_unavailable_manifest(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    reason: &str,
) -> Result<std::path::PathBuf> {
    let checked_at = chrono::Utc::now().to_rfc3339();
    let version = format!("{}-unavailable", chrono::Utc::now().format("%Y%m%d"));
    let relative =
        format!(".archon/trading-lab/data/datasets/{dataset_id}/{version}/manifest.json");
    let manifest_path = root.join(&relative);
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = json!({
        "schema_version": "archon-trading-stooq-unavailable-v1",
        "dataset_id": dataset_id,
        "version": version,
        "provider": "stooq",
        "symbol": symbol,
        "timeframe": timeframe,
        "start": start,
        "end": end,
        "checked_at": checked_at,
        "status": "provider_blocked_or_unavailable",
        "quality_status": "provider_blocked_or_unavailable",
        "native_interval": normalize_timeframe(timeframe) == "1D",
        "production_eligible": false,
        "registered_healthy_dataset": false,
        "can_fetch": false,
        "unavailable_reason": format!(
            "provider_blocked_or_unavailable: {reason}; exact native Stooq data was not directly supplied and resampling is refused"
        ),
        "fail_closed_behavior": "unavailable Stooq proof omits ohlcv.jsonl and raw/response.csv and does not write a healthy registry entry",
        "omitted_artifacts": ["ohlcv.jsonl", "raw/response.csv"],
    });
    let text = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&manifest_path, text)?;
    Ok(manifest_path)
}

fn persist_stooq_capability(
    root: &Path,
    symbol: &str,
    provider_symbol: &str,
    production_eligible: bool,
) -> Result<()> {
    let checked_at = chrono::Utc::now().to_rfc3339();
    let mut capability =
        archon_trading::data_lake::can_fetch_symbol_timeframe("stooq", symbol, "1D", &checked_at);
    capability.can_fetch = production_eligible;
    capability.production_eligible = production_eligible;
    capability.provider_symbol = provider_symbol.into();
    capability.unavailable_reason = (!production_eligible).then(|| {
        "ARCHON_STOOQ_CSV_URL override is test/fixture evidence only; not production eligible"
            .into()
    });
    TradingDataLake::new(root)
        .persist_capability_result(capability)
        .map_err(data_error)?;
    Ok(())
}

fn persist_stooq_unavailable_capability(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    reason: &str,
) -> Result<()> {
    let checked_at = chrono::Utc::now().to_rfc3339();
    let normalized_timeframe = normalize_timeframe(timeframe);
    let mut capability = archon_trading::data_lake::can_fetch_symbol_timeframe(
        "stooq",
        symbol,
        &normalized_timeframe,
        &checked_at,
    );
    capability.provider_symbol = stooq_provider_symbol(symbol);
    capability.can_fetch = false;
    capability.production_eligible = false;
    capability.unavailable_reason = Some(format!(
        "provider_blocked_or_unavailable: {reason}; exact native Stooq data was not directly supplied and resampling is refused"
    ));
    TradingDataLake::new(root)
        .persist_capability_result(capability)
        .map_err(data_error)?;
    Ok(())
}

struct StooqSource {
    url: String,
    is_live_stooq: bool,
}

fn stooq_source(provider_symbol: &str, start: &str, end: &str) -> StooqSource {
    if let Ok(url) = std::env::var("ARCHON_STOOQ_CSV_URL") {
        return StooqSource {
            url,
            is_live_stooq: false,
        };
    }
    StooqSource {
        url: format!(
            "https://stooq.com/q/d/l/?s={provider_symbol}&d1={}&d2={}&i=d",
            stooq_date(start),
            stooq_date(end)
        ),
        is_live_stooq: true,
    }
}

fn parse_stooq_csv(input: &[u8]) -> Result<Vec<OhlcvBar>, archon_trading::ohlcv::OhlcvError> {
    let text = std::str::from_utf8(input)
        .map_err(|error| archon_trading::ohlcv::OhlcvError::Csv(error.to_string()))?;
    parse_ohlcv(normalize_stooq_csv(text).as_bytes(), OhlcvFormat::Csv)
}

fn normalize_stooq_csv(text: &str) -> String {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return String::new();
    };
    let header = header
        .replace("Date,", "timestamp,")
        .replace(",Volume", ",volume")
        .to_ascii_lowercase();
    let rows = lines.map(|line| {
        let mut columns = line.split(',');
        let Some(date) = columns.next() else {
            return line.to_string();
        };
        format!(
            "{}T00:00:00Z,{}",
            date.trim(),
            columns.collect::<Vec<_>>().join(",")
        )
    });
    std::iter::once(header)
        .chain(rows)
        .collect::<Vec<_>>()
        .join("\n")
}

fn stooq_body_is_non_data(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    text.trim().is_empty()
        || text.contains("<html")
        || text.contains("<!doctype html")
        || text.contains("captcha")
        || text.contains("access denied")
        || text.contains("verification")
        || text.contains("blocked")
}

fn stooq_metadata(
    dataset_id: &str,
    symbol: &str,
    provider_symbol: &str,
    bars: &[OhlcvBar],
    production_eligible: bool,
) -> DatasetMetadata {
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id: dataset_id.into(),
        version: format!("{}-native_stooq_1D", version_date(bars)),
        canonical_instrument: symbol.trim().into(),
        asset_class: "equity".into(),
        provider: "stooq".into(),
        provider_symbol: provider_symbol.into(),
        timeframe: "1D".into(),
        native_interval: true,
        production_eligible,
        price_basis: "raw".into(),
        session: "regular".into(),
        data_type: DataType::Ohlcv,
        symbol_map: std::collections::BTreeMap::from([(
            symbol.trim().into(),
            provider_symbol.into(),
        )]),
        timezone: "America/New_York".into(),
        adjustment: "split_and_dividend".into(),
        license: "Stooq terms; verify redistribution before external use".into(),
        coverage: CoverageWindow {
            start: String::new(),
            end: String::new(),
            expected_bars: bars.len() as u64,
            observed_bars: bars.len() as u64,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: bars.len() as u64,
        },
        checksum: String::new(),
        checksums: DatasetChecksums::default(),
        paths: Default::default(),
        source: DatasetSourceMetadata::default(),
        quality_status: "passed".into(),
        created_at: String::new(),
        optional: false,
    }
}

fn stooq_provider_symbol(symbol: &str) -> String {
    let trimmed = symbol.trim().to_ascii_lowercase();
    if trimmed.contains('.') {
        trimmed
    } else {
        format!("{trimmed}.us")
    }
}

fn normalize_timeframe(timeframe: &str) -> String {
    match timeframe.trim().to_ascii_uppercase().as_str() {
        "D" | "1D" => "1D".into(),
        other => other.into(),
    }
}

fn stooq_date(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(8)
        .collect()
}

fn version_date(bars: &[OhlcvBar]) -> String {
    bars.first()
        .map(|bar| stooq_date(&bar.timestamp))
        .filter(|value| value.len() == 8)
        .unwrap_or_else(|| chrono::Utc::now().format("%Y%m%d").to_string())
}

fn stooq_redacted_headers(headers: &reqwest::header::HeaderMap) -> serde_json::Value {
    let headers = headers
        .iter()
        .map(|(name, value)| {
            let value = if name.as_str().eq_ignore_ascii_case("set-cookie") {
                "<redacted>".into()
            } else {
                value.to_str().unwrap_or("<non-utf8>").to_string()
            };
            (name.as_str().to_string(), serde_json::Value::String(value))
        })
        .collect::<serde_json::Map<_, _>>();
    serde_json::Value::Object(headers)
}

fn stooq_report_with_fetch_error(
    report: String,
    reason: &str,
    manifest_path: Option<String>,
) -> String {
    match serde_json::from_str::<serde_json::Value>(&report) {
        Ok(mut value) => {
            value["unavailable_reason"] = json!(format!(
                "provider_blocked_or_unavailable: {reason}; exact native Stooq data was not directly supplied and resampling is refused"
            ));
            if let Some(path) = manifest_path {
                value["unavailable_manifest_path"] = json!(path);
            }
            serde_json::to_string_pretty(&value).unwrap_or(report)
        }
        Err(_) => report,
    }
}
