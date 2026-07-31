pub(super) fn request() -> StoreOhlcvRequest {
    StoreOhlcvRequest {
        metadata: DatasetMetadata {
            schema_version: "archon-trading-dataset-v2".into(),
            dataset_id: "manual-BTCUSD-1D-raw".into(),
            version: "20260101-fixture".into(),
            canonical_instrument: "BTCUSD".into(),
            asset_class: "crypto".into(),
            provider: "manual".into(),
            provider_symbol: "BTCUSD".into(),
            timeframe: "1D".into(),
            native_interval: true,
            production_eligible: true,
            price_basis: "raw".into(),
            session: "24x7".into(),
            data_type: DataType::Ohlcv,
            symbol_map: BTreeMap::from([("BTCUSD".into(), "BTCUSD".into())]),
            timezone: "UTC".into(),
            adjustment: "raw".into(),
            license: "research".into(),
            coverage: CoverageWindow {
                start: String::new(),
                end: String::new(),
                expected_bars: 2,
                observed_bars: 0,
            },
            gaps: GapSummary {
                missing_bars: 0,
                expected_bars: 2,
            },
            checksum: String::new(),
            checksums: DatasetChecksums::default(),
            paths: DatasetArtifactPaths::default(),
            source: DatasetSourceMetadata::default(),
            quality_status: "passed".into(),
            created_at: String::new(),
            optional: false,
        },
        bars: vec![
            bar("2026-01-01T00:00:00Z", 10.0),
            bar("2026-01-02T00:00:00Z", 11.0),
        ],
        raw_body: b"raw".to_vec(),
        raw_format: OhlcvFormat::Csv,
        raw_request: serde_json::json!({"source":"test"}),
        redacted_headers: serde_json::json!({}),
        provider_notes: "test fixture".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn spy_request() -> StoreOhlcvRequest {
    core_coverage_request("SPY", "1D")
}

fn core_coverage_request(instrument: &str, timeframe: &str) -> StoreOhlcvRequest {
    let mut request = request();
    request.metadata.provider = "tradingview".into();
    request.metadata.canonical_instrument = instrument.into();
    request.metadata.provider_symbol = provider_symbol(instrument, "tradingview");
    request.metadata.dataset_id = format!("tradingview-{instrument}-{timeframe}-raw");
    request.metadata.asset_class = if instrument.ends_with("USDT") {
        "crypto"
    } else {
        "equity"
    }
    .into();
    request.metadata.timeframe = timeframe.into();
    request.metadata.symbol_map = BTreeMap::from([(
        instrument.into(),
        provider_symbol(instrument, "tradingview"),
    )]);
    // Production backtest history requires twice the coverage minimum, so a
    // fixture sized at `COVERAGE_MINIMUM_ROWS` is rejected by the AHDM gate.
    // The 28-day months roll past a year at this size; roll the year with them.
    request.metadata.coverage.expected_bars = AHDM_BACKTEST_MINIMUM_ROWS as u64;
    request.metadata.gaps.expected_bars = AHDM_BACKTEST_MINIMUM_ROWS as u64;
    // A constant close delta trips `bars_have_linear_shape`, which rejects the
    // dataset as placeholder evidence. Modulate the series the same way the
    // AHDM fixtures in `data_store_ahdm_tests` do.
    request.bars = (0..AHDM_BACKTEST_MINIMUM_ROWS)
        .map(|index| {
            let year = 2026 + (index / 336);
            let month = ((index / 28) % 12) + 1;
            let day = (index % 28) + 1;
            let cycle = index as f64;
            bar(
                &format!("{year}-{month:02}-{day:02}T00:00:00Z"),
                100.0 + (cycle / 7.0).sin() * 3.0 + cycle * 0.03,
            )
        })
        .collect();
    // The live-fetch provenance gate wants "live", "fetch" and "provider" in
    // each captured artefact; the raw response body is checked too.
    request.raw_request = serde_json::json!({"source":"live provider fetch test capture"});
    request.raw_body = serde_json::to_vec(&serde_json::json!({
        "source": "captured live provider fetch",
        "provider": "tradingview",
        "bar_count": AHDM_BACKTEST_MINIMUM_ROWS,
    }))
    .unwrap();
    request.provider_notes = "captured live provider fetch response".into();
    request
}

pub(super) fn store_complete_trading_core_coverage(lake: &TradingDataLake) {
    for instrument in trading_core_instruments() {
        for timeframe in trading_core_timeframes() {
            lake.store_ohlcv(core_coverage_request(&instrument, &timeframe))
                .unwrap();
        }
    }
}

fn persist_trading_core_snapshots(lake: &TradingDataLake, captured_at_unix_seconds: i64) {
    for instrument in trading_core_instruments() {
        persist_snapshot(lake, &instrument, captured_at_unix_seconds);
    }
}

fn persist_snapshot(lake: &TradingDataLake, instrument: &str, captured_at_unix_seconds: i64) {
    lake.persist_snapshot(
        CurrentSnapshot {
            provider: "tradingview".into(),
            canonical_instrument: instrument.into(),
            provider_symbol: provider_symbol(instrument, "tradingview"),
            captured_at_unix_seconds,
            payload: serde_json::json!({"last": 500.0}),
        },
        1_781_049_600,
    )
    .unwrap();
}

fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: close * 1_000.0,
    }
}
