use archon_trading::data_lake::providers::yfinance::{
    YfinanceAdapter, YfinanceClock, YfinanceRequest, YfinanceResponse, YfinanceTransport,
    YfinanceTransportError,
};
use archon_trading::data_lake::{DatasetStatus, NativeFetchRequest, UnavailableReason};
use archon_trading::data_store::{TradingDataLake, YfinanceIngestRequest, YfinanceIngestResult};
use std::sync::{Arc, Mutex};

struct Clock;
impl YfinanceClock for Clock {
    fn now_rfc3339(&self) -> String {
        "2025-01-03T12:00:00Z".into()
    }
}

struct Transport {
    calls: Mutex<Vec<YfinanceRequest>>,
    response: YfinanceResponse,
}
impl YfinanceTransport for Transport {
    fn execute(
        &self,
        request: &YfinanceRequest,
    ) -> Result<YfinanceResponse, YfinanceTransportError> {
        self.calls.lock().unwrap().push(request.clone());
        Ok(self.response.clone())
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
        headers: serde_json::json!({
            "server":"yahoo", "authorization":"must-not-persist", "cookie":"must-not-persist"
        }),
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

fn adapter(response: YfinanceResponse) -> (YfinanceAdapter, Arc<Transport>) {
    let transport = Arc::new(Transport {
        calls: Mutex::new(Vec::new()),
        response,
    });
    (
        YfinanceAdapter::new(transport.clone(), Arc::new(Clock)),
        transport,
    )
}

fn publish() -> (
    tempfile::TempDir,
    TradingDataLake,
    Box<archon_trading::data_store::StoredDatasetRecord>,
) {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, transport) = adapter(response());
    let result = lake.ingest_yfinance(&adapter, request()).unwrap();
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
    let YfinanceIngestResult::PublishedDegradedDiagnostic(record) = result else {
        panic!("expected diagnostic publication")
    };
    (temp, lake, record)
}

#[test]
fn complete_response_publishes_degraded_diagnostic_artifacts() {
    let (temp, _, record) = publish();
    assert_eq!(record.provider, "yfinance");
    assert_eq!(record.status, DatasetStatus::Degraded);
    assert!(!record.production_eligible);
    for path in [
        &record.raw_response_path,
        &record.raw_request_path,
        &record.redacted_headers_path,
        &record.provider_notes_path,
        &record.normalized_path,
        &record.metadata_path,
        &record.validation_path,
        &record.manifest_path,
    ] {
        assert!(temp.path().join(path).is_file(), "missing {path}");
    }
    let persisted = directory_text(temp.path());
    assert!(!persisted.contains("must-not-persist"));
    assert!(persisted.contains("degraded diagnostic fallback only"));
    assert!(persisted.contains("No relabeling, derivation, aggregation, resampling"));
    assert!(persisted.contains("native-yahoo-chart-v1"));
    assert!(persisted.contains("interval-dependent"));
    assert!(persisted.contains("provider_reported"));
    assert!(persisted.contains("provider_default"));
    assert!(persisted.contains("UTC"));
    assert!(persisted.contains("Yahoo Finance chart API"));
}

#[test]
fn production_and_promotion_remain_denied() {
    let (_, lake, record) = publish();
    assert!(!record.production_eligible);
    assert_eq!(record.status, DatasetStatus::Degraded);
    assert!(
        lake.backtest_data_gate(&record.dataset_id, &record.version, false)
            .is_err()
    );
    let diagnostic = lake
        .backtest_data_gate(&record.dataset_id, &record.version, true)
        .unwrap();
    assert!(diagnostic.diagnostic);
    assert!(!diagnostic.promotion_eligible);
    assert!(!diagnostic.issues.is_empty());
}

#[test]
fn unsupported_four_hour_aliases_make_zero_calls_and_publish_nothing() {
    for alias in ["240", "4H", "4h", "04h"] {
        let temp = tempfile::tempdir().unwrap();
        let lake = TradingDataLake::new(temp.path());
        let (adapter, transport) = adapter(response());
        let mut ingest = request();
        ingest.fetch.timeframe = alias.into();

        let result = lake.ingest_yfinance(&adapter, ingest).unwrap();
        assert_eq!(
            result,
            YfinanceIngestResult::Unavailable {
                reason: UnavailableReason::ExactNativeIntervalUnsupported,
            }
        );
        assert!(transport.calls.lock().unwrap().is_empty());
        assert!(!lake.data_root().exists());
    }
}

#[test]
fn foreign_response_symbol_fails_closed_without_persisting_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut malformed = response();
    let mut payload: serde_json::Value = serde_json::from_slice(&malformed.body).unwrap();
    payload["chart"]["result"][0]["meta"]["symbol"] = serde_json::json!("QQQ");
    malformed.body = serde_json::to_vec(&payload).unwrap();
    let (adapter, transport) = adapter(malformed);

    let result = lake.ingest_yfinance(&adapter, request()).unwrap();
    assert_eq!(
        result,
        YfinanceIngestResult::Unavailable {
            reason: UnavailableReason::MalformedResponse,
        }
    );
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
    assert!(!lake.data_root().exists());
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
