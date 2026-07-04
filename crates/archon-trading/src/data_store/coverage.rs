use super::*;
pub(super) fn trading_core_instruments() -> Vec<String> {
    ["ES", "NQ", "SPY", "QQQ", "BTCUSDT", "ETHUSDT"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(super) fn trading_core_timeframes() -> Vec<String> {
    ["1W", "1D", "240", "60", "15"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(super) fn coverage_cell(
    lake: &TradingDataLake,
    registry: &PersistentDatasetRegistry,
    instrument: &str,
    timeframe: &str,
    checked_at: &str,
) -> CoverageCell {
    let mut rejected_reasons = Vec::new();
    for record in registry.datasets.values() {
        if !coverage_record_candidate(record, instrument, timeframe) {
            continue;
        }
        match coverage_record_issues(lake, record, instrument, timeframe) {
            Ok(()) => return available_coverage_cell(record, instrument, timeframe),
            Err(issues) => rejected_reasons.push(format!(
                "{}:{} rejected: {}",
                record.dataset_id,
                record.version,
                issues.join("; ")
            )),
        }
    }

    let selected = provider_order()
        .into_iter()
        .map(|provider| can_fetch_symbol_timeframe(provider, instrument, timeframe, checked_at))
        .find(|capability| capability.native_interval)
        .unwrap_or_else(|| {
            can_fetch_symbol_timeframe("tradingview", instrument, timeframe, checked_at)
        });
    let fallback_reason = if rejected_reasons.is_empty() {
        selected
            .unavailable_reason
            .unwrap_or_else(|| "no registered production-eligible native dataset".into())
    } else {
        rejected_reasons.join(" | ")
    };

    CoverageCell {
        canonical_instrument: instrument.into(),
        timeframe: selected.timeframe.clone(),
        selected_provider: selected.provider.clone(),
        provider_symbol: provider_symbol(instrument, &selected.provider),
        dataset_id: None,
        version: None,
        available: false,
        native_interval: selected.native_interval,
        production_eligible: false,
        quality_status: provider_quality(&selected.provider).into(),
        row_count: 0,
        coverage_start: String::new(),
        coverage_end: String::new(),
        fallback_reason: Some(fallback_reason),
    }
}

fn available_coverage_cell(
    record: &StoredDatasetRecord,
    instrument: &str,
    timeframe: &str,
) -> CoverageCell {
    CoverageCell {
        canonical_instrument: instrument.into(),
        timeframe: timeframe.into(),
        selected_provider: record.provider.clone(),
        provider_symbol: provider_symbol(instrument, &record.provider),
        dataset_id: Some(record.dataset_id.clone()),
        version: Some(record.version.clone()),
        available: true,
        native_interval: true,
        production_eligible: true,
        quality_status: "passed".into(),
        row_count: record.bars as u64,
        coverage_start: record.coverage_start.clone(),
        coverage_end: record.coverage_end.clone(),
        fallback_reason: None,
    }
}

pub(super) fn coverage_markdown(matrix: &CoverageMatrix) -> String {
    let mut text = format!(
        "# Trading Coverage Matrix\n\n- schema_version: `{}`\n- generated_at: `{}`\n- instruments: `{}`\n- timeframes: `{}`\n- gaps: `{}`\n\n| Instrument | Timeframe | Provider | Symbol | Available | Native | Production | Quality | Rows | Dataset | Fallback reason |\n|---|---|---|---|---:|---:|---:|---|---:|---|---|\n",
        matrix.schema_version,
        matrix.generated_at,
        matrix.instruments.join(", "),
        matrix.timeframes.join(", "),
        matrix.gaps.len()
    );
    for cell in &matrix.cells {
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            cell.canonical_instrument,
            cell.timeframe,
            cell.selected_provider,
            cell.provider_symbol,
            cell.available,
            cell.native_interval,
            cell.production_eligible,
            cell.quality_status,
            cell.row_count,
            cell.dataset_id.as_deref().unwrap_or(""),
            cell.fallback_reason.as_deref().unwrap_or("")
        ));
    }
    text.push_str("\n## Gaps\n\n");
    for gap in &matrix.gaps {
        text.push_str(&format!(
            "- `{}` `{}`: {}\n",
            gap.canonical_instrument, gap.timeframe, gap.reason
        ));
    }
    text
}

fn coverage_record_candidate(
    record: &StoredDatasetRecord,
    instrument: &str,
    timeframe: &str,
) -> bool {
    record.dataset_id.contains(instrument) && record.dataset_id.contains(timeframe)
}

fn coverage_record_issues(
    lake: &TradingDataLake,
    record: &StoredDatasetRecord,
    instrument: &str,
    timeframe: &str,
) -> Result<(), Vec<String>> {
    let mut issues = Vec::new();
    append_missing_artifact_issues(&lake.root, record, &mut issues);
    if issues
        .iter()
        .any(|issue| issue.contains("missing artifact"))
    {
        return Err(issues);
    }
    match lake.load_ohlcv(&record.dataset_id, &record.version) {
        Ok(dataset) => {
            append_dataset_gate_issues(&lake.root, record, &dataset, &mut issues);
            append_coverage_identity_issues(&dataset.metadata, instrument, timeframe, &mut issues);
        }
        Err(error) => issues.push(format!("dataset unreadable: {error:?}")),
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn append_coverage_identity_issues(
    metadata: &DatasetMetadata,
    instrument: &str,
    timeframe: &str,
    issues: &mut Vec<String>,
) {
    if metadata.canonical_instrument != instrument {
        issues.push(format!(
            "metadata canonical instrument mismatch: {}",
            metadata.canonical_instrument
        ));
    }
    if metadata.timeframe != timeframe {
        issues.push(format!(
            "metadata timeframe mismatch: {}",
            metadata.timeframe
        ));
    }
}

pub(super) fn provider_order() -> [&'static str; 5] {
    ["tradingview", "openbb", "polygon", "stooq", "yfinance"]
}

pub(super) fn provider_symbol(instrument: &str, provider: &str) -> String {
    match (instrument, provider) {
        ("ES", "tradingview") => "CME_MINI:ES1!".into(),
        ("NQ", "tradingview") => "CME_MINI:NQ1!".into(),
        ("BTCUSDT", "tradingview") => "BINANCE:BTCUSDT".into(),
        ("ETHUSDT", "tradingview") => "BINANCE:ETHUSDT".into(),
        ("SPY", "stooq") => "SPY.US".into(),
        ("QQQ", "stooq") => "QQQ.US".into(),
        _ => instrument.into(),
    }
}

pub(super) fn provider_quality(provider: &str) -> &'static str {
    match provider {
        "stooq" => "baseline_unavailable",
        "yfinance" => "degraded_fallback",
        _ => "unavailable",
    }
}
