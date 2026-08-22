use super::*;
use crate::data_lake::DatasetStatus;
use crate::data_lake::providers::yfinance::{
    YfinanceClock, YfinanceRequest, YfinanceResponse, YfinanceTransport, YfinanceTransportError,
};
use std::sync::{Arc, Mutex};

struct Clock;
impl YfinanceClock for Clock {
    fn now_rfc3339(&self) -> String {
        "2025-01-03T12:00:00Z".into()
    }
}

struct Transport {
    calls: Mutex<usize>,
}
impl YfinanceTransport for Transport {
    fn execute(
        &self,
        _request: &YfinanceRequest,
    ) -> Result<YfinanceResponse, YfinanceTransportError> {
        *self.calls.lock().unwrap() += 1;
        Ok(response())
    }
}

fn response() -> YfinanceResponse {
    let value = serde_json::json!({"chart":{"result":[{
        "meta":{"symbol":"SPY","dataGranularity":"1h"},
        "timestamp":[1735828200,1735831800],
        "indicators":{"quote":[{"open":[10.0,11.0],"high":[12.0,13.0],
        "low":[9.0,10.0],"close":[11.0,12.0],"volume":[100.0,200.0]}]}
    }],"error":null}});
    YfinanceResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({"cookie":"redact-me", "server":"yahoo"}),
        body: serde_json::to_vec(&value).unwrap(),
    }
}

fn request() -> YfinanceIngestRequest {
    YfinanceIngestRequest {
        fetch: NativeFetchRequest {
            provider: "yfinance".into(),
            canonical_instrument: "SPY".into(),
            provider_symbol: "SPY".into(),
            timeframe: "1h".into(),
            start: "2025-01-02T14:30:00Z".into(),
            end: "2025-01-02T15:30:00Z".into(),
        },
        created_at: "2025-01-03T12:00:00Z".into(),
        price_basis: "raw".into(),
        optional: true,
    }
}

#[test]
fn yfinance_ingest_is_degraded_diagnostic_only() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let transport = Arc::new(Transport {
        calls: Mutex::new(0),
    });
    let adapter = YfinanceAdapter::new(transport, Arc::new(Clock));
    let result = lake.ingest_yfinance(&adapter, request()).unwrap();
    let YfinanceIngestResult::PublishedDegradedDiagnostic(record) = result else {
        panic!("expected degraded diagnostic publication")
    };
    assert_eq!(record.provider, "yfinance");
    assert_eq!(record.status, DatasetStatus::Degraded);
    assert!(record.native_interval);
    assert!(!record.production_eligible);
    let metadata: DatasetMetadata =
        serde_json::from_slice(&std::fs::read(temp.path().join(&record.metadata_path)).unwrap())
            .unwrap();
    assert_eq!(metadata.quality_status, "degraded");
    assert!(!metadata.production_eligible);
    let gate = lake
        .backtest_data_gate(&record.dataset_id, &record.version, true)
        .unwrap();
    assert!(gate.diagnostic);
    assert!(!gate.promotion_eligible);
    let persisted = directory_text(temp.path());
    assert!(!persisted.contains("redact-me"));
    assert!(persisted.contains("diagnostic_only"));
    assert!(persisted.contains("promotion_eligible"));
    assert!(persisted.contains("native-yahoo-chart-v1"));
    assert!(persisted.contains("interval-dependent"));
    assert!(persisted.contains("provider_reported"));
    assert!(persisted.contains("provider_default"));
    assert!(persisted.contains("UTC"));
    assert!(persisted.contains("unofficial degraded diagnostic fallback only"));
    assert!(persisted.contains("Yahoo Finance chart API"));
}

fn directory_text(root: &std::path::Path) -> String {
    fn visit(path: &std::path::Path, output: &mut String) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, output);
            } else if let Ok(text) = std::fs::read_to_string(path) {
                output.push_str(&text);
            }
        }
    }
    let mut output = String::new();
    visit(root, &mut output);
    output
}
