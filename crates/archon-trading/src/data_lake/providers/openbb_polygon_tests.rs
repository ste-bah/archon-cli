use super::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

struct TestEnvironment(BTreeMap<String, String>);

impl OpenbbEnvironment for TestEnvironment {
    fn value(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

struct TestClock;

impl OpenbbClock for TestClock {
    fn now_rfc3339(&self) -> String {
        "2025-01-02T00:00:00Z".into()
    }
}

struct SpyTransport {
    requests: Mutex<Vec<OpenbbRequest>>,
    response: OpenbbResponse,
}

impl OpenbbTransport for SpyTransport {
    fn execute(&self, request: &OpenbbRequest) -> Result<OpenbbResponse, OpenbbTransportError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(self.response.clone())
    }
}

fn environment(key: Option<&str>) -> Arc<TestEnvironment> {
    let mut values = BTreeMap::new();
    if let Some(key) = key {
        values.insert(POLYGON_CREDENTIAL_KEY.into(), key.into());
    }
    Arc::new(TestEnvironment(values))
}

fn response(provider: &str, bars: serde_json::Value, expected: u64) -> OpenbbResponse {
    let body = serde_json::json!({
        "provider": provider,
        "symbol": "AAPL",
        "timeframe": "1d",
        "adjustment": "splits",
        "corporate_action_source": "polygon",
        "exchange": "XNAS",
        "timezone": "America/New_York",
        "session": "regular",
        "calendar": "XNYS",
        "asset_class": "equity",
        "license": "Polygon terms",
        "requested_start": "2025-01-01T00:00:00Z",
        "requested_end": "2025-01-02T00:00:00Z",
        "expected_bars": expected,
        "native": true,
        "derived": false,
        "response_schema": "openbb-equity-historical",
        "response_version": "1",
        "results": bars,
    });
    OpenbbResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({"x-source": "polygon", "authorization": "planted"}),
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn response_from_body(body: serde_json::Value) -> OpenbbResponse {
    OpenbbResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({}),
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn bars() -> serde_json::Value {
    serde_json::json!([
        {"timestamp":"2025-01-01T00:00:00Z","open":10.0,"high":12.0,"low":9.0,"close":11.0,"volume":100.0},
        {"timestamp":"2025-01-02T00:00:00Z","open":20.0,"high":22.0,"low":19.0,"close":21.0,"volume":200.0}
    ])
}

fn request() -> NativeFetchRequest {
    NativeFetchRequest {
        provider: "polygon".into(),
        canonical_instrument: "AAPL".into(),
        provider_symbol: "AAPL".into(),
        timeframe: "1d".into(),
        start: "2025-01-01T00:00:00Z".into(),
        end: "2025-01-02T00:00:00Z".into(),
    }
}

fn adapter(
    env: Arc<TestEnvironment>,
    response: OpenbbResponse,
) -> (OpenbbPolygonAdapter, Arc<SpyTransport>) {
    let transport = Arc::new(SpyTransport {
        requests: Mutex::new(Vec::new()),
        response,
    });
    (
        OpenbbPolygonAdapter::new(env, transport.clone(), Arc::new(TestClock)),
        transport,
    )
}

#[test]
fn openbb_polygon_provider_credential_first_and_address_contract() {
    let (adapter, transport) = adapter(environment(None), response("polygon", bars(), 2));
    assert_eq!(
        adapter.fetch_native_history(&request()),
        Err(UnavailableReason::MissingCredentials)
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(
        resolve_openbb_base_url(environment(Some("x")).as_ref()).unwrap(),
        OPENBB_DEFAULT_BASE_URL
    );
    let blank = TestEnvironment(BTreeMap::from([("OPENBB_HOST".into(), " ".into())]));
    assert_eq!(
        resolve_openbb_base_url(&blank),
        Err(UnavailableReason::InvalidRequest)
    );
}

#[test]
fn openbb_polygon_provider_exact_request_and_one_to_one_bars() {
    let (adapter, transport) = adapter(environment(Some("key")), response("polygon", bars(), 2));
    let prepared = adapter.fetch_for_ingest(&request()).unwrap();
    assert_eq!(prepared.history.bars[0].open, 10.0);
    assert_eq!(prepared.history.bars[1].open, 20.0);
    assert_eq!(prepared.evidence.transport, "openbb");
    let sent = transport.requests.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].path, OPENBB_POLYGON_ROUTE);
    assert!(
        sent[0]
            .query
            .contains(&("provider".into(), "polygon".into()))
    );
    assert!(!prepared.redacted_headers.to_string().contains("planted"));
}

#[test]
fn openbb_polygon_provider_rejects_transport_as_provider() {
    let (adapter, transport) = adapter(environment(Some("key")), response("openbb", bars(), 2));
    assert_eq!(
        adapter.fetch_native_history(&request()),
        Err(UnavailableReason::MalformedResponse)
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn openbb_polygon_provider_probe_is_strictly_bounded() {
    let (adapter, transport) = adapter(environment(Some("key")), response("polygon", bars(), 2));
    let capability = CapabilityRequest {
        provider: "polygon".into(),
        canonical_instrument: "AAPL".into(),
        provider_symbol: "AAPL".into(),
        timeframe: "1d".into(),
        checked_at: "2025-01-02T00:00:00Z".into(),
    };
    let evidence = adapter
        .probe_capability(&capability, ProbeBudget::default())
        .unwrap();
    assert_eq!(evidence.candles_examined, 2);
    let sent = transport.requests.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].query.contains(&("limit".into(), "2".into())));
}

#[test]
fn openbb_polygon_provider_rejects_session_calendar_and_corporate_action_mismatch() {
    for (field, value) in [
        ("timezone", "UTC"),
        ("session", "extended"),
        ("calendar", "24x7"),
        ("corporate_action_source", "openbb"),
    ] {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&response("polygon", bars(), 2).body).unwrap();
        payload[field] = value.into();
        let (adapter, _) = adapter(environment(Some("key")), response_from_body(payload));
        assert_eq!(
            adapter.fetch_native_history(&request()),
            Err(UnavailableReason::MalformedResponse),
            "field {field}"
        );
    }
}

#[test]
fn openbb_polygon_provider_rejects_bars_outside_requested_bounds() {
    for timestamp in ["2024-12-31T23:59:59Z", "2025-01-02T00:00:01Z"] {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&response("polygon", bars(), 2).body).unwrap();
        payload["results"][0]["timestamp"] = timestamp.into();
        if timestamp > "2025-01-02T00:00:00Z" {
            payload["results"].as_array_mut().unwrap().swap(0, 1);
        }
        let (adapter, _) = adapter(environment(Some("key")), response_from_body(payload));
        assert_eq!(
            adapter.fetch_native_history(&request()),
            Err(UnavailableReason::MalformedResponse)
        );
    }
}

#[test]
fn openbb_polygon_provider_rejects_malformed_and_oversized_payloads() {
    let malformed = OpenbbResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({}),
        body: b"{".to_vec(),
    };
    let (malformed_adapter, _) = adapter(environment(Some("key")), malformed);
    assert_eq!(
        malformed_adapter.fetch_native_history(&request()),
        Err(UnavailableReason::MalformedResponse)
    );

    let oversized = OpenbbResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({}),
        body: vec![b' '; MAX_INGEST_RESPONSE_BYTES + 1],
    };
    let (adapter, _) = adapter(environment(Some("key")), oversized);
    assert_eq!(
        adapter.fetch_native_history(&request()),
        Err(UnavailableReason::ProbeLimitExceeded)
    );
}
