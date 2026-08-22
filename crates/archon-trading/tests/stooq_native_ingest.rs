use archon_trading::data_lake::providers::stooq::{
    StooqAdapter, StooqClock, StooqRequest, StooqResponse, StooqTransport, StooqTransportError,
};
use archon_trading::data_lake::{NativeFetchRequest, UnavailableReason};
use archon_trading::data_store::{StooqIngestRequest, StooqIngestResult, TradingDataLake};
use std::sync::{Arc, Mutex};

struct Clock;
impl StooqClock for Clock {
    fn now_rfc3339(&self) -> String {
        "2025-01-06T12:00:00Z".into()
    }
}

struct Transport {
    calls: Mutex<Vec<StooqRequest>>,
    response: StooqResponse,
}

impl StooqTransport for Transport {
    fn execute(&self, request: &StooqRequest) -> Result<StooqResponse, StooqTransportError> {
        self.calls.lock().unwrap().push(request.clone());
        Ok(self.response.clone())
    }
}

fn transport(status: u16, content_type: &str, body: &str) -> Arc<Transport> {
    Arc::new(Transport {
        calls: Mutex::new(Vec::new()),
        response: StooqResponse {
            status,
            content_type: content_type.into(),
            headers: serde_json::json!({
                "server": "stooq",
                "authorization": "must-not-persist",
                "cookie": "must-not-persist"
            }),
            body: body.as_bytes().to_vec(),
        },
    })
}

fn ingest_request() -> StooqIngestRequest {
    StooqIngestRequest {
        fetch: NativeFetchRequest {
            provider: "stooq".into(),
            canonical_instrument: "SPY".into(),
            provider_symbol: "SPY.US".into(),
            timeframe: "1d".into(),
            start: "2025-01-03".into(),
            end: "2025-01-06".into(),
        },
        created_at: "2025-01-06T12:00:00Z".into(),
        price_basis: "raw".into(),
        optional: false,
    }
}

#[test]
fn complete_exact_native_response_publishes_redacted_artifacts_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let transport = transport(
        200,
        "text/csv",
        "Date,Open,High,Low,Close,Volume\n2025-01-03,10,12,9,11,100\n2025-01-06,11,13,10,12,200\n",
    );
    let adapter = StooqAdapter::new(transport.clone(), Arc::new(Clock));
    let result = lake.ingest_stooq(&adapter, ingest_request()).unwrap();
    let StooqIngestResult::Published(record) = result else {
        panic!("expected published Stooq dataset")
    };
    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].query,
        [
            ("s", "spy.us"),
            ("d1", "20250103"),
            ("d2", "20250106"),
            ("i", "d")
        ]
        .map(|(key, value)| (key.into(), value.into()))
    );
    let fingerprint = calls[0].fingerprint();
    drop(calls);
    assert_eq!(record.provider, "stooq");
    assert!(record.native_interval);
    assert!(record.production_eligible);
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
    assert!(persisted.contains(&fingerprint));
    assert!(persisted.contains("retry_count"));
    assert!(persisted.contains("request_fingerprint"));
    assert!(lake.registry_path().is_file());
}

#[test]
fn blocked_or_partial_response_returns_unavailable_without_publication() {
    for (status, content_type, body, expected) in [
        (
            403,
            "text/html",
            "<html>access denied</html>",
            UnavailableReason::ProviderBlocked403,
        ),
        (
            200,
            "text/csv",
            "Date,Open,High,Low,Close,Volume\n2025-01-03,10,12,9,11,100\n",
            UnavailableReason::MalformedResponse,
        ),
        (
            200,
            "text/csv",
            "Date,Open,High,Low,Close,Volume\n2025-01-03,10,9,8,11,100\n2025-01-06,11,13,10,12,200\n",
            UnavailableReason::MalformedResponse,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let lake = TradingDataLake::new(temp.path());
        let transport = transport(status, content_type, body);
        let adapter = StooqAdapter::new(transport.clone(), Arc::new(Clock));
        assert_eq!(
            lake.ingest_stooq(&adapter, ingest_request()).unwrap(),
            StooqIngestResult::Unavailable { reason: expected }
        );
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
        assert!(!lake.registry_path().exists());
        assert!(!lake.data_root().join("datasets").exists());
    }
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
