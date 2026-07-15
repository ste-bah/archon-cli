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
    for provider in provider_order() {
        let capability = can_fetch_symbol_timeframe(provider, instrument, timeframe, checked_at);
        if !capability.native_interval {
            continue;
        }
        let records = coverage_record_candidates(registry, provider, instrument, timeframe);
        if records.is_empty() {
            append_rejected_provider_records(
                registry,
                provider,
                instrument,
                timeframe,
                &mut rejected_reasons,
            );
            rejected_reasons.push(provider_unavailable_reason(&capability));
            continue;
        }
        let freshness = snapshot_freshness_for(registry, provider, instrument, checked_at);
        if freshness != SnapshotFreshness::Fresh {
            rejected_reasons.push(snapshot_gap_reason(provider, instrument, freshness));
            continue;
        }
        for record in records {
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

pub(super) fn validate_coverage_matrix_complete(
    matrix: &CoverageMatrix,
) -> Result<(), DataStoreError> {
    let missing = matrix
        .cells
        .iter()
        .filter(|cell| {
            !cell.available
                || cell.dataset_id.is_none()
                || !cell.production_eligible
                || cell.provider_symbol.trim().is_empty()
                || cell.timeframe.trim().is_empty()
        })
        .map(|cell| {
            format!(
                "{}:{}: {}",
                cell.canonical_instrument,
                cell.timeframe,
                cell.fallback_reason
                    .as_deref()
                    .unwrap_or("no provider-native validated registry dataset")
            )
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(DataStoreError::InvalidMetadata(format!(
            "coverage matrix incomplete for trading-core-v1: {}",
            missing.join("; ")
        )))
    }
}

fn coverage_record_candidates<'a>(
    registry: &'a PersistentDatasetRegistry,
    provider: &str,
    instrument: &str,
    timeframe: &str,
) -> Vec<&'a StoredDatasetRecord> {
    registry
        .datasets
        .values()
        .filter(|record| {
            record.provider == provider
                && record_matches_coverage_cell(record, instrument, timeframe)
        })
        .collect()
}

fn append_rejected_provider_records(
    registry: &PersistentDatasetRegistry,
    provider: &str,
    instrument: &str,
    timeframe: &str,
    rejected_reasons: &mut Vec<String>,
) {
    for record in registry.datasets.values().filter(|record| {
        record.provider == provider && record.symbol == instrument && record.timeframe == timeframe
    }) {
        rejected_reasons.push(format!(
            "{}:{} rejected: registry native_interval={} production_eligible={} status={:?}",
            record.dataset_id,
            record.version,
            record.native_interval,
            record.production_eligible,
            record.status
        ));
    }
}

fn record_matches_coverage_cell(
    record: &StoredDatasetRecord,
    instrument: &str,
    timeframe: &str,
) -> bool {
    record.symbol == instrument
        && record.timeframe == timeframe
        && record.native_interval
        && record.production_eligible
        && record.status == DatasetStatus::Healthy
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

fn snapshot_freshness_for(
    registry: &PersistentDatasetRegistry,
    provider: &str,
    instrument: &str,
    generated_at: &str,
) -> SnapshotFreshness {
    let generated_at = unix_seconds(generated_at).unwrap_or(i64::MAX);
    snapshot_freshness(
        snapshot_captured_at(registry, provider, instrument),
        generated_at,
    )
}

fn snapshot_captured_at(
    registry: &PersistentDatasetRegistry,
    provider: &str,
    instrument: &str,
) -> Option<i64> {
    registry
        .snapshots
        .values()
        .filter_map(|artifact| matching_snapshot_captured_at(artifact, provider, instrument))
        .max()
}

fn matching_snapshot_captured_at(
    artifact: &serde_json::Value,
    provider: &str,
    instrument: &str,
) -> Option<i64> {
    let snapshot = artifact.get("snapshot").unwrap_or(artifact);
    let artifact_provider = snapshot.get("provider")?.as_str()?;
    let artifact_instrument = snapshot.get("canonical_instrument")?.as_str()?;
    if !artifact_provider.eq_ignore_ascii_case(provider) || artifact_instrument != instrument {
        return None;
    }
    snapshot.get("captured_at_unix_seconds")?.as_i64()
}

fn unix_seconds(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp())
        .ok()
}

fn snapshot_gap_reason(provider: &str, instrument: &str, freshness: SnapshotFreshness) -> String {
    format!(
        "{provider}:{instrument} current snapshot freshness is {freshness:?}; snapshots older than 5 minutes are stale"
    )
}

fn provider_unavailable_reason(capability: &ProviderCapabilityResult) -> String {
    format!(
        "{}:{} no provider-native validated registry dataset; capability reason: {}",
        capability.provider,
        capability.canonical_instrument,
        capability
            .unavailable_reason
            .as_deref()
            .unwrap_or("unavailable")
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible_cell(provider_symbol: &str, timeframe: &str) -> CoverageCell {
        CoverageCell {
            canonical_instrument: "ES".into(),
            timeframe: timeframe.into(),
            selected_provider: "tradingview".into(),
            provider_symbol: provider_symbol.into(),
            dataset_id: Some("tradingview-ES-1D-raw".into()),
            version: Some("fixture".into()),
            available: true,
            native_interval: true,
            production_eligible: true,
            quality_status: "passed".into(),
            row_count: 100,
            coverage_start: "2026-01-01T00:00:00Z".into(),
            coverage_end: "2026-04-10T00:00:00Z".into(),
            fallback_reason: None,
        }
    }

    fn matrix(cell: CoverageCell) -> CoverageMatrix {
        CoverageMatrix {
            schema_version: "archon-trading-coverage-v1".into(),
            generated_at: "2026-07-15T00:00:00Z".into(),
            instruments: vec!["ES".into()],
            timeframes: vec!["1D".into()],
            cells: vec![cell],
            gaps: Vec::new(),
        }
    }

    #[test]
    fn d47_production_eligible_coverage_requires_symbol_and_interval() {
        assert!(validate_coverage_matrix_complete(&matrix(eligible_cell("", "1D"))).is_err());
        assert!(validate_coverage_matrix_complete(&matrix(eligible_cell("ES1!", ""))).is_err());
        assert!(validate_coverage_matrix_complete(&matrix(eligible_cell("ES1!", "1D"))).is_ok());
    }
}
