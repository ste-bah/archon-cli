use archon_trading::data_lake::providers::openbb_polygon::{
    OpenbbClock, OpenbbEnvironment, OpenbbPolygonAdapter, OpenbbRequest, OpenbbResponse,
    OpenbbTransport, OpenbbTransportError, resolve_openbb_base_url,
};
use archon_trading::data_lake::{
    CapabilityRequest, NativeFetchRequest, ProbeBudget, ProviderAdapter, UnavailableReason,
};
use archon_trading::data_store::{
    OpenbbPolygonIngestRequest, OpenbbPolygonIngestResult, TradingDataLake,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

struct Env(BTreeMap<String, String>);
impl OpenbbEnvironment for Env {
    fn value(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

struct Clock;
impl OpenbbClock for Clock {
    fn now_rfc3339(&self) -> String {
        "2025-01-02T00:00:00Z".into()
    }
}

struct Spy {
    requests: Mutex<Vec<OpenbbRequest>>,
    response: OpenbbResponse,
}
impl OpenbbTransport for Spy {
    fn execute(&self, request: &OpenbbRequest) -> Result<OpenbbResponse, OpenbbTransportError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(self.response.clone())
    }
}

fn env(key: Option<&str>) -> Arc<Env> {
    let mut values = BTreeMap::new();
    if let Some(key) = key {
        values.insert("POLYGON_API_KEY".into(), key.into());
    }
    Arc::new(Env(values))
}

fn body(provider: &str, expected: u64) -> serde_json::Value {
    serde_json::json!({
        "provider":provider,"symbol":"AAPL","timeframe":"1d","adjustment":"splits",
        "corporate_action_source":"polygon",
        "exchange":"XNAS","timezone":"America/New_York","session":"regular",
        "calendar":"XNYS","asset_class":"equity","license":"Polygon terms",
        "requested_start":"2025-01-01T00:00:00Z","requested_end":"2025-01-02T00:00:00Z",
        "expected_bars":expected,"native":true,"derived":false,
        "response_schema":"openbb-equity-historical","response_version":"1","results":[
            {"timestamp":"2025-01-01T00:00:00Z","open":10.0,"high":12.0,"low":9.0,"close":11.0,"volume":100.0},
            {"timestamp":"2025-01-02T00:00:00Z","open":20.0,"high":22.0,"low":19.0,"close":21.0,"volume":200.0}
        ]
    })
}

fn response(provider: &str, expected: u64, status: u16) -> OpenbbResponse {
    OpenbbResponse {
        status,
        content_type: "application/json".into(),
        headers: serde_json::json!({
            "x-provider":"polygon",
            "authorization":"planted-authorization",
            "nested":{"cookie":"planted-cookie"}
        }),
        body: serde_json::to_vec(&body(provider, expected)).unwrap(),
    }
}

fn adapter(environment: Arc<Env>, response: OpenbbResponse) -> (OpenbbPolygonAdapter, Arc<Spy>) {
    let spy = Arc::new(Spy {
        requests: Mutex::new(Vec::new()),
        response,
    });
    (
        OpenbbPolygonAdapter::new(environment, spy.clone(), Arc::new(Clock)),
        spy,
    )
}

fn fetch() -> NativeFetchRequest {
    NativeFetchRequest {
        provider: "polygon".into(),
        canonical_instrument: "AAPL".into(),
        provider_symbol: "AAPL".into(),
        timeframe: "1d".into(),
        start: "2025-01-01T00:00:00Z".into(),
        end: "2025-01-02T00:00:00Z".into(),
    }
}

fn ingest() -> OpenbbPolygonIngestRequest {
    OpenbbPolygonIngestRequest {
        fetch: fetch(),
        created_at: "2025-01-02T00:00:00Z".into(),
        price_basis: "raw".into(),
        optional: false,
    }
}

#[test]
fn credential_preflight_stops_before_transport() {
    for value in [None, Some(""), Some("  ")] {
        let (adapter, spy) = adapter(env(value), response("polygon", 2, 200));
        assert_eq!(
            adapter.fetch_native_history(&fetch()),
            Err(UnavailableReason::MissingCredentials)
        );
        assert!(spy.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn service_address_contract_is_exact() {
    assert_eq!(
        resolve_openbb_base_url(env(Some("key")).as_ref()).unwrap(),
        "http://127.0.0.1:6900"
    );
    let configured = Env(BTreeMap::from([
        ("OPENBB_HOST".into(), "openbb.internal".into()),
        ("OPENBB_PORT".into(), "7000".into()),
        ("OPENBB_API_URL".into(), "https://ignored.example".into()),
    ]));
    assert_eq!(
        resolve_openbb_base_url(&configured).unwrap(),
        "http://openbb.internal:7000"
    );
}

#[test]
fn capability_probe_is_bounded_and_non_publishing() {
    let (adapter, spy) = adapter(env(Some("key")), response("polygon", 2, 200));
    let request = CapabilityRequest {
        provider: "polygon".into(),
        canonical_instrument: "AAPL".into(),
        provider_symbol: "AAPL".into(),
        timeframe: "1d".into(),
        checked_at: "2025-01-02T00:00:00Z".into(),
    };
    let evidence = adapter
        .probe_capability(&request, ProbeBudget::default())
        .unwrap();
    assert_eq!(evidence.candles_examined, 2);
    let requests = spy.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].query.contains(&("limit".into(), "2".into())));
}

#[test]
fn exact_polygon_identity_and_native_interval() {
    let (adapter, _) = adapter(env(Some("key")), response("polygon", 2, 200));
    let history = adapter.fetch_native_history(&fetch()).unwrap();
    assert_eq!(history.provider, "polygon");
    assert_eq!(history.timeframe, "1d");
    assert_eq!(history.provenance.provider, "polygon");
    assert!(!history.provenance.resampled);
}

#[test]
fn bars_are_one_to_one_without_transformation() {
    let (adapter, _) = adapter(env(Some("key")), response("polygon", 2, 200));
    let history = adapter.fetch_native_history(&fetch()).unwrap();
    assert_eq!(history.bars.len(), 2);
    assert_eq!(history.bars[0].open, 10.0);
    assert_eq!(history.bars[1].open, 20.0);
}

#[test]
fn incomplete_coverage_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, _) = adapter(env(Some("key")), response("polygon", 3, 200));
    assert_eq!(
        lake.ingest_openbb_polygon(&adapter, ingest()).unwrap(),
        OpenbbPolygonIngestResult::Partial {
            observed_bars: 2,
            expected_bars: 3
        }
    );
    assert!(!lake.registry_path().exists());
}

#[test]
fn unavailable_states_do_not_retry_or_fallback() {
    for (status, reason) in [
        (401, UnavailableReason::Unauthorized401),
        (403, UnavailableReason::ProviderBlocked403),
        (404, UnavailableReason::NotFound404),
        (500, UnavailableReason::HttpStatusError),
    ] {
        let (adapter, spy) = adapter(env(Some("key")), response("polygon", 2, status));
        assert_eq!(adapter.fetch_native_history(&fetch()), Err(reason));
        assert_eq!(spy.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn redacted_artifacts_precede_registry_publish() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, _) = adapter(env(Some("key")), response("polygon", 2, 200));
    let OpenbbPolygonIngestResult::Published(record) =
        lake.ingest_openbb_polygon(&adapter, ingest()).unwrap()
    else {
        panic!("expected publication")
    };
    assert!(temp.path().join(record.raw_request_path).is_file());
    assert!(temp.path().join(record.redacted_headers_path).is_file());
    assert!(temp.path().join(record.validation_path).is_file());
    assert!(temp.path().join(record.manifest_path).is_file());
    assert!(lake.registry_path().is_file());
}

#[test]
fn secrets_are_absent_from_all_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, _) = adapter(env(Some("planted-api-value")), response("polygon", 2, 200));
    lake.ingest_openbb_polygon(&adapter, ingest()).unwrap();
    let mut stack = vec![temp.path().to_path_buf()];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            stack.extend(
                std::fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else {
            let bytes = std::fs::read(path).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("planted-api-value"));
            assert!(!text.contains("planted-authorization"));
            assert!(!text.contains("planted-cookie"));
        }
    }
}
