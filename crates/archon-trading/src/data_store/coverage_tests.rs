use super::coverage_methods::coverage_markdown;
use super::*;

fn test_coverage_cell(instrument: &str, timeframe: &str, production: bool) -> CoverageCell {
    CoverageCell {
        canonical_instrument: instrument.into(),
        timeframe: timeframe.into(),
        selected_provider: "tradingview".into(),
        provider_symbol: provider_symbol(instrument, "tradingview"),
        dataset_id: production.then(|| format!("tradingview-{instrument}-{timeframe}-raw")),
        version: production.then(|| "20260610-abcdef".into()),
        dataset_checksum: production.then(|| "checksum".into()),
        available: production,
        native_interval: production,
        production_eligible: production,
        quality_status: if production { "passed" } else { "unavailable" }.into(),
        row_count: if production {
            COVERAGE_MINIMUM_ROWS as u64
        } else {
            0
        },
        coverage_start: production.then(|| "2025-01-01T00:00:00Z".into()),
        coverage_end: production.then(|| "2026-06-10T00:00:00Z".into()),
        fallback_reason: (!production).then(|| "missing qualified evidence".into()),
    }
}

fn exact_matrix(production: bool) -> CoverageMatrix {
    let instruments = trading_core_instruments();
    let timeframes = trading_core_timeframes();
    let cells = instruments
        .iter()
        .flat_map(|instrument| {
            timeframes
                .iter()
                .map(move |timeframe| test_coverage_cell(instrument, timeframe, production))
        })
        .collect::<Vec<_>>();
    let gaps = cells
        .iter()
        .filter_map(|cell| {
            cell.fallback_reason.as_ref().map(|reason| CoverageGap {
                canonical_instrument: cell.canonical_instrument.clone(),
                timeframe: cell.timeframe.clone(),
                reason: reason.clone(),
            })
        })
        .collect();
    CoverageMatrix {
        schema_version: "archon-trading-coverage-v1".into(),
        generated_at: "2026-06-10T00:00:00Z".into(),
        instruments,
        timeframes,
        cells,
        gaps,
    }
}

fn record(provider: &str, created_at: &str) -> StoredDatasetRecord {
    StoredDatasetRecord {
        dataset_id: format!("{provider}-ES-1D-raw"),
        version: created_at.replace([':', '-'], ""),
        schema_version: registry_contract_schema(),
        dataset_path: "dataset".into(),
        metadata_checksum: "metadata".into(),
        raw_checksum: "raw".into(),
        validation_checksum: "validation".into(),
        raw_response_path: "raw/response.json".into(),
        raw_request_path: "raw/request.json".into(),
        redacted_headers_path: "raw/headers.redacted.json".into(),
        provider_notes_path: "raw/provider-notes.md".into(),
        provider: provider.into(),
        data_type: "Ohlcv".into(),
        symbol: "ES".into(),
        timeframe: "1D".into(),
        native_interval: true,
        production_eligible: true,
        status: DatasetStatus::Healthy,
        checksum: "checksum".into(),
        bars: COVERAGE_MINIMUM_ROWS,
        coverage_start: "2025-01-01T00:00:00Z".into(),
        coverage_end: "2026-06-10T00:00:00Z".into(),
        metadata_path: "metadata.json".into(),
        normalized_path: "ohlcv.jsonl".into(),
        raw_path: "raw/response.json".into(),
        validation_path: "validation.json".into(),
        manifest_path: "manifest.json".into(),
        created_at: created_at.into(),
    }
}

#[test]
fn coverage_matrix_has_exact_trading_core_v1_cells() {
    let temp = tempfile::tempdir().unwrap();
    let matrix = TradingDataLake::new(temp.path())
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    assert_eq!(matrix.cells.len(), 30);
    assert_eq!(matrix.gaps.len(), 30);
    validate_coverage_shape(&matrix).unwrap();
}

#[test]
fn coverage_rejects_identity_order_and_artifact_mutations() {
    let mut matrix = exact_matrix(true);
    matrix.cells.swap(0, 1);
    assert!(validate_coverage_shape(&matrix).is_err());
    matrix = exact_matrix(true);
    matrix.cells[0].canonical_instrument = "UNKNOWN".into();
    assert!(validate_coverage_shape(&matrix).is_err());
    matrix = exact_matrix(true);
    matrix.cells.pop();
    assert!(validate_coverage_shape(&matrix).is_err());
}

#[test]
fn coverage_qualifies_before_provider_priority() {
    let mut high_priority = record("tradingview", "2026-06-01T00:00:00Z");
    high_priority.production_eligible = false;
    high_priority.status = DatasetStatus::Degraded;
    let lower_priority = record("polygon", "2026-06-10T00:00:00Z");
    let mut registry = PersistentDatasetRegistry::default();
    registry.datasets.insert("high".into(), high_priority);
    registry.datasets.insert("low".into(), lower_priority);
    let cell = super::coverage_cell(
        &TradingDataLake::new(tempfile::tempdir().unwrap().path()),
        &registry,
        "ES",
        "1D",
        "2026-06-10T00:00:00Z",
    );
    assert!(!cell.available);
    assert!(
        cell.fallback_reason
            .unwrap()
            .contains("production_eligible=false")
    );
}

#[test]
fn coverage_freshness_edges_fail_closed() {
    let mut daily = record("tradingview", "2026-06-03T00:00:00Z");
    assert!(historical_record_is_fresh(&daily, "1D", "2026-06-10T00:00:00Z").is_ok());
    assert!(historical_record_is_fresh(&daily, "1D", "2026-06-10T00:00:01Z").is_err());
    daily.created_at = "2026-06-09T00:00:00Z".into();
    assert!(historical_record_is_fresh(&daily, "15", "2026-06-10T00:00:00Z").is_ok());
    assert!(historical_record_is_fresh(&daily, "15", "2026-06-10T00:00:01Z").is_err());
    daily.created_at = "malformed".into();
    assert!(historical_record_is_fresh(&daily, "1D", "2026-06-10T00:00:00Z").is_err());
    daily.created_at = "2026-06-11T00:00:00Z".into();
    assert!(historical_record_is_fresh(&daily, "1D", "2026-06-10T00:00:00Z").is_err());
    assert!(crate::data_lake::snapshot_is_fresh(1_000, 1_300));
    assert!(!crate::data_lake::snapshot_is_fresh(1_000, 1_301));
}

#[test]
fn coverage_non_production_cells_have_exact_gaps() {
    let mut matrix = exact_matrix(false);
    validate_coverage_shape(&matrix).unwrap();
    matrix.gaps[0].reason = "different".into();
    assert!(validate_coverage_shape(&matrix).is_err());
}

#[test]
fn coverage_latest_json_and_markdown_match() {
    let matrix = exact_matrix(true);
    let json = serde_json::to_value(&matrix).unwrap();
    let markdown = coverage_markdown(&matrix);
    assert_eq!(json["schema_version"], "archon-trading-coverage-v1");
    assert!(json.get("schema").is_none());
    for cell in &matrix.cells {
        assert!(markdown.contains(&format!(
            "| {} | {} | {} |",
            cell.canonical_instrument, cell.timeframe, cell.selected_provider
        )));
    }
    assert!(markdown.contains("| Dataset | Version | Checksum |"));
}

#[test]
fn coverage_pair_failure_restores_previous_pair() {
    let temp = tempfile::tempdir().unwrap();
    let json_path = temp.path().join("latest.json");
    let markdown_path = temp.path().join("latest.md");
    std::fs::write(&json_path, b"old-json").unwrap();
    std::fs::write(&markdown_path, b"old-markdown").unwrap();
    inject_io_failure(Some("transaction.replace.1"));
    let result = atomic_write_many(vec![
        (json_path.clone(), b"new-json".to_vec()),
        (markdown_path.clone(), b"new-markdown".to_vec()),
    ]);
    inject_io_failure(None);
    assert!(result.is_err());
    assert_eq!(std::fs::read(json_path).unwrap(), b"old-json");
    assert_eq!(std::fs::read(markdown_path).unwrap(), b"old-markdown");
}

#[test]
fn coverage_performs_no_fetch_or_resampling() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let before = lake.load_registry().unwrap();
    let matrix = lake
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let after = lake.load_registry().unwrap();
    assert_eq!(before, after);
    assert!(matrix.cells.iter().all(|cell| cell.dataset_id.is_none()));
}

#[test]
fn coverage_history_preserves_previous_json() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.write_coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let previous = std::fs::read(lake.coverage_dir().join("latest.json")).unwrap();

    lake.write_coverage_matrix("trading-core-v1", "2026-06-10T01:00:00Z".into())
        .unwrap();

    let history = std::fs::read_dir(lake.coverage_dir().join("history"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    assert_eq!(std::fs::read(history).unwrap(), previous);
    let latest: CoverageMatrix = read_json(&lake.coverage_dir().join("latest.json")).unwrap();
    assert_eq!(latest.generated_at, "2026-06-10T01:00:00Z");
}

#[test]
fn coverage_publication_accepts_explicit_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());

    let matrix = lake
        .write_coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();

    assert_eq!(matrix.gaps.len(), 30);
    let artifact: serde_json::Value = read_json(&lake.coverage_dir().join("latest.json")).unwrap();
    assert_eq!(artifact["schema_version"], "archon-trading-coverage-v1");
    assert!(artifact.get("schema").is_none());
}

#[test]
fn coverage_rejects_an_incomplete_existing_pair() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    std::fs::create_dir_all(lake.coverage_dir()).unwrap();
    std::fs::write(lake.coverage_dir().join("latest.json"), b"{}").unwrap();

    let result = lake.write_coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into());

    assert!(
        matches!(result, Err(DataStoreError::InvalidMetadata(message)) if message.contains("pair is incomplete"))
    );
}

#[test]
fn coverage_wire_contract_uses_null_bounds_for_unavailable_cells() {
    let value = serde_json::to_value(exact_matrix(false)).unwrap();
    assert_eq!(value["schema_version"], "archon-trading-coverage-v1");
    assert!(value["cells"][0]["coverage_start"].is_null());
    assert!(value["cells"][0]["coverage_end"].is_null());
}
