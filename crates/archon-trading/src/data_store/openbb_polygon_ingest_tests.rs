use super::*;
use crate::data_lake::providers::openbb_polygon::{
    OpenbbClock, OpenbbEnvironment, OpenbbRequest, OpenbbResponse, OpenbbTransport,
    OpenbbTransportError,
};
use crate::data_store::inject_io_failure;
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

struct Transport {
    calls: Mutex<usize>,
    response: OpenbbResponse,
}
impl OpenbbTransport for Transport {
    fn execute(&self, _: &OpenbbRequest) -> Result<OpenbbResponse, OpenbbTransportError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.response.clone())
    }
}

fn response(expected: u64) -> OpenbbResponse {
    let body = serde_json::json!({
        "provider":"polygon","symbol":"AAPL","timeframe":"1d",
        "adjustment":"splits","corporate_action_source":"polygon",
        "exchange":"XNAS","timezone":"America/New_York",
        "session":"regular","calendar":"XNYS","asset_class":"equity",
        "license":"Polygon terms","requested_start":"2025-01-01T00:00:00Z",
        "requested_end":"2025-01-02T00:00:00Z","expected_bars":expected,
        "native":true,"derived":false,"response_schema":"openbb-equity-historical",
        "response_version":"1","results":[
            {"timestamp":"2025-01-01T00:00:00Z","open":10.0,"high":12.0,"low":9.0,"close":11.0,"volume":100.0},
            {"timestamp":"2025-01-02T00:00:00Z","open":20.0,"high":22.0,"low":19.0,"close":21.0,"volume":200.0}
        ]
    });
    OpenbbResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({"authorization":"planted"}),
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn adapter(expected: u64) -> (OpenbbPolygonAdapter, Arc<Transport>) {
    let env = Arc::new(Env(BTreeMap::from([(
        "POLYGON_API_KEY".into(),
        "runtime-only".into(),
    )])));
    let transport = Arc::new(Transport {
        calls: Mutex::new(0),
        response: response(expected),
    });
    (
        OpenbbPolygonAdapter::new(env, transport.clone(), Arc::new(Clock)),
        transport,
    )
}

fn request() -> OpenbbPolygonIngestRequest {
    OpenbbPolygonIngestRequest {
        fetch: NativeFetchRequest {
            provider: "polygon".into(),
            canonical_instrument: "AAPL".into(),
            provider_symbol: "AAPL".into(),
            timeframe: "1d".into(),
            start: "2025-01-01T00:00:00Z".into(),
            end: "2025-01-02T00:00:00Z".into(),
        },
        created_at: "2025-01-02T00:00:00Z".into(),
        price_basis: "raw".into(),
        optional: false,
    }
}

#[test]
fn openbb_polygon_ingest_publishes_redacted_artifacts_then_registry() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, transport) = adapter(2);
    let result = lake.ingest_openbb_polygon(&adapter, request()).unwrap();
    let OpenbbPolygonIngestResult::Published(record) = result else {
        panic!("expected published result")
    };
    assert_eq!(*transport.calls.lock().unwrap(), 1);
    assert_eq!(record.provider, "polygon");
    assert!(lake.registry_path().exists());
    for path in [
        &record.raw_request_path,
        &record.redacted_headers_path,
        &record.provider_notes_path,
        &record.normalized_path,
        &record.metadata_path,
        &record.validation_path,
        &record.manifest_path,
    ] {
        assert!(temp.path().join(path).is_file());
    }
    let tree = std::fs::read_to_string(temp.path().join(&record.raw_request_path)).unwrap();
    assert!(tree.contains("native_lineage_evidence"));
    assert!(!tree.contains("runtime-only"));
}

#[test]
fn openbb_polygon_ingest_partial_never_publishes() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, _) = adapter(3);
    assert_eq!(
        lake.ingest_openbb_polygon(&adapter, request()).unwrap(),
        OpenbbPolygonIngestResult::Partial {
            observed_bars: 2,
            expected_bars: 3,
        }
    );
    assert!(!lake.registry_path().exists());
}

#[test]
fn openbb_polygon_ingest_failure_preserves_registry_and_removes_partial_dataset() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let (adapter, _) = adapter(2);
    let prepared = adapter.fetch_for_ingest(&request().fetch).unwrap();
    let raw_checksum = bytes_checksum(&serde_json::to_vec(&prepared.raw_response).unwrap());
    let metadata = metadata(&request(), &prepared.evidence);
    let dataset_id = crate::data_lake::dataset_id(&metadata).unwrap();
    let version = raw_bound_version(&request().created_at, &raw_checksum).unwrap();
    let publication_dir = lake.dataset_dir(&dataset_id, &version);
    let prior_registry = std::fs::read(lake.registry_path()).ok();

    inject_io_failure(Some("file.rename"));
    let result = lake.ingest_openbb_polygon(&adapter, request());
    inject_io_failure(None);

    assert!(result.is_err());
    assert!(!publication_dir.exists());
    assert_eq!(std::fs::read(lake.registry_path()).ok(), prior_registry);
    assert!(lake.load_registry().unwrap().datasets.is_empty());
}
