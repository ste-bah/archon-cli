use crate::command::trading_data_provider::{coverage, fetch_native};
use archon_trading::data_store::TradingDataLake;
use serde_json::json;

#[test]
fn coverage_refuses_to_publish_incomplete_required_universe() {
    let temp = tempfile::tempdir().unwrap();
    let error = coverage(
        Some(&temp.path().to_path_buf()),
        "trading-core-v1",
        true,
        None,
    )
    .expect_err("empty required universe must fail closed");
    assert!(error
        .to_string()
        .contains("coverage matrix incomplete for trading-core-v1"));
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/coverage/latest.json")
        .exists());
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/coverage/latest.md")
        .exists());
}

fn write_tradingview_fixture(
    root: &std::path::Path,
    symbol: &str,
    timeframe: &str,
    bar_count: usize,
    degraded_volume: bool,
) {
    let fixture = root.join(".archon/test-fixtures/tradingview/ohlcv.json");
    std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
    let mut bars = (0..bar_count)
        .map(|index| {
            let open = 100.0 + index as f64;
            json!({"time": 1_704_067_200_i64 + index as i64 * 86_400, "open": open,
                "high": open + 2.0, "low": open - 1.0, "close": open + 1.0,
                "volume": if degraded_volume { 0.0 } else { 1_000.0 + index as f64 }})
        })
        .collect::<Vec<_>>();
    if degraded_volume {
        bars[1].as_object_mut().unwrap().remove("volume");
    }
    let response = json!({"success": true, "symbol": symbol, "timeframe": timeframe,
            "requested_count": bar_count, "bar_count": bar_count, "bars": bars});
    std::fs::write(&fixture, serde_json::to_vec(&response).unwrap()).unwrap();
}

#[test]
fn fetch_native_tradingview_unavailable_writes_no_registry_dataset() {
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "CME_MINI:ES1!",
        "1D",
        "2024-01-01",
        "2024-01-03",
        "tradingview-ES-1D-raw",
    )
    .unwrap();
    assert!(text.contains("declared MCP tool invocation required"));
    assert!(text.contains("local TradingView CLI/Node shims are not accepted"));
    assert!(text.contains("no dataset registry entry is written"));
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn fetch_native_tradingview_ignores_local_cli_shim() {
    let temp = tempfile::tempdir().unwrap();
    let cli = temp
        .path()
        .join(".archon/tools/tradingview-mcp/src/cli/index.js");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(
        &cli,
        "console.log(JSON.stringify({success:true,symbol:'CME_MINI:ES1!',timeframe:'1D',bars:[{ts:1704153600,open:1,high:2,low:1,close:2,volume:1}]}));",
    )
    .unwrap();

    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "CME_MINI:ES1!",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "tradingview-ES-1D-raw",
    )
    .unwrap();
    assert!(text.contains("local TradingView CLI/Node shims are not accepted"));
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn fetch_native_tradingview_uses_span_derived_count() {
    let temp = tempfile::tempdir().unwrap();
    write_tradingview_fixture(temp.path(), "CME_MINI:ES1!", "1D", 5, false);
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "CME_MINI:ES1!",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "tradingview-ES-1D-raw",
    )
    .unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["can_fetch"], true);
    assert!(text.contains("20240101-tv_native_1D_5"));
}

#[test]
fn fetch_native_tradingview_stores_complete_artifact_contract() {
    let temp = tempfile::tempdir().unwrap();
    write_tradingview_fixture(temp.path(), "CME_MINI:ES1!", "1D", 5, false);
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "CME_MINI:ES1!",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "tradingview-ES-1D-raw",
    )
    .unwrap();
    let dataset_root = temp
        .path()
        .join(".archon/trading-lab/data/datasets/tradingview-ES-1D-raw/20240101-tv_native_1D_5");
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        report["can_fetch"], true,
        "unexpected TradingView response: {text}"
    );
    assert_eq!(report["production_eligible"], true);
    assert!(text.contains("20240101-tv_native_1D_5"));
    for artifact in [
        "metadata.json",
        "validation.json",
        "manifest.json",
        "ohlcv.jsonl",
        "raw/request.json",
        "raw/response.json",
        "raw/headers.redacted.json",
        "raw/provider-notes.md",
    ] {
        assert!(dataset_root.join(artifact).exists());
    }
    assert!(temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn fetch_native_tradingview_rejects_mismatched_chart_identity() {
    let temp = tempfile::tempdir().unwrap();
    write_tradingview_fixture(temp.path(), "NASDAQ:WRONG", "1D", 100, false);
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "CME_MINI:ES1!",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "tradingview-ES-1D-raw",
    )
    .unwrap();
    assert!(text.contains("response symbol mismatch"));
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["production_eligible"], false);
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn fetch_native_tradingview_rejects_row_shortfall() {
    let temp = tempfile::tempdir().unwrap();
    write_tradingview_fixture(temp.path(), "CME_MINI:ES1!", "1D", 4, false);
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "CME_MINI:ES1!",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "tradingview-ES-1D-raw",
    )
    .unwrap();
    assert!(text.contains("row shortfall"));
    assert!(text.contains("requested=5 actual=4"));
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn d44_tradingview_zero_or_missing_volume_stays_degraded() {
    let temp = tempfile::tempdir().unwrap();
    write_tradingview_fixture(temp.path(), "TVC:GOLD", "1D", 5, true);
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "tradingview",
        "TVC:GOLD",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "tradingview-GOLD-1D-raw",
    )
    .unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["production_eligible"], false);
    let lake = TradingDataLake::new(temp.path());
    let registry = lake.load_registry().unwrap();
    let record = registry
        .datasets
        .values()
        .next()
        .unwrap_or_else(|| panic!("expected degraded TradingView dataset: {text}"));
    assert_eq!(
        record.status,
        archon_trading::data_lake::DatasetStatus::Degraded
    );
    assert!(!record.production_eligible);
    let dataset = lake
        .load_ohlcv(&record.dataset_id, &record.version)
        .unwrap();
    assert_eq!(dataset.bars.len(), 5);
    assert!(dataset.bars.iter().all(|bar| bar.volume == 0.0));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join(&record.validation_path)).unwrap())
            .unwrap();
    assert_eq!(report["status"], "failed");
    assert_eq!(report["production_eligible"], false);
}
