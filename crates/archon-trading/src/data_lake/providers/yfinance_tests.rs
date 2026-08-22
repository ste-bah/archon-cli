use super::*;
use std::sync::Mutex;

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

fn request(interval: &str) -> NativeFetchRequest {
    NativeFetchRequest {
        provider: YFINANCE_PROVIDER.into(),
        canonical_instrument: "SPY".into(),
        provider_symbol: "SPY".into(),
        timeframe: interval.into(),
        start: "2025-01-02T14:30:00Z".into(),
        end: "2025-01-02T15:30:00Z".into(),
    }
}

fn response(symbol: &str, interval: &str) -> YfinanceResponse {
    let payload = serde_json::json!({
        "chart": {
            "result": [{
                "meta": {"symbol": symbol, "dataGranularity": interval},
                "timestamp": [1735828200, 1735831800],
                "indicators": {"quote": [{
                    "open": [10.0, 11.0], "high": [12.0, 13.0],
                    "low": [9.0, 10.0], "close": [11.0, 12.0],
                    "volume": [100.0, 200.0]
                }]}
            }],
            "error": null
        }
    });
    YfinanceResponse {
        status: 200,
        content_type: "application/json".into(),
        headers: serde_json::json!({"server":"yahoo", "cookie":"must-not-persist"}),
        body: serde_json::to_vec(&payload).unwrap(),
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

#[test]
fn yfinance_provider_identity_is_immutable() {
    let (subject, transport) = adapter(response("SPY", "1h"));
    let prepared = subject.fetch_for_ingest(&request("1h")).unwrap();
    assert_eq!(subject.provider_id(), YFINANCE_PROVIDER);
    assert_eq!(prepared.history.provider, YFINANCE_PROVIDER);
    assert_eq!(prepared.history.provenance.provider, YFINANCE_PROVIDER);
    assert_eq!(prepared.history.provenance.native_timeframe, "1h");
    assert!(!prepared.history.provenance.resampled);
    assert!(
        !prepared
            .redacted_headers
            .to_string()
            .contains("must-not-persist")
    );
    assert_eq!(transport.calls.lock().unwrap().len(), 1);

    let (foreign, _) = adapter(response("SPY", "1h"));
    let mut wrong = request("1h");
    wrong.provider = "polygon".into();
    assert_eq!(
        foreign.fetch_for_ingest(&wrong),
        Err(UnavailableReason::InvalidRequest)
    );
}

#[test]
fn yfinance_rejects_240_before_transport() {
    for alias in ["240", "4H", "4h", "04h", "1H", "60"] {
        let (adapter, transport) = adapter(response("SPY", "1h"));
        assert_eq!(
            adapter.fetch_for_ingest(&request(alias)),
            Err(UnavailableReason::ExactNativeIntervalUnsupported),
            "alias {alias} must fail closed"
        );
        assert!(transport.calls.lock().unwrap().is_empty());
    }
}

struct OfflineTransport;

impl YfinanceTransport for OfflineTransport {
    fn execute(
        &self,
        _request: &YfinanceRequest,
    ) -> Result<YfinanceResponse, YfinanceTransportError> {
        Err(YfinanceTransportError::Connection)
    }
}

#[test]
fn network_failure_is_reported_without_leaking_transport_details() {
    let adapter = YfinanceAdapter::new(Arc::new(OfflineTransport), Arc::new(Clock));
    assert_eq!(
        adapter.fetch_for_ingest(&request("1h")),
        Err(UnavailableReason::ProbeInconclusive)
    );
}

#[test]
fn foreign_interval_and_malformed_bars_fail_closed() {
    let (foreign_interval, _) = adapter(response("SPY", "1d"));
    assert_eq!(
        foreign_interval.fetch_for_ingest(&request("1h")),
        Err(UnavailableReason::MalformedResponse)
    );

    let mut malformed = response("SPY", "1h");
    let mut payload: serde_json::Value = serde_json::from_slice(&malformed.body).unwrap();
    payload["chart"]["result"][0]["indicators"]["quote"][0]["low"][0] = serde_json::json!(13.0);
    malformed.body = serde_json::to_vec(&payload).unwrap();
    let (malformed_adapter, _) = adapter(malformed);
    assert_eq!(
        malformed_adapter.fetch_for_ingest(&request("1h")),
        Err(UnavailableReason::MalformedResponse)
    );
}

#[test]
fn malformed_foreign_and_secret_bearing_responses_fail_closed() {
    let (foreign, _) = adapter(response("QQQ", "1h"));
    assert_eq!(
        foreign.fetch_for_ingest(&request("1h")),
        Err(UnavailableReason::MalformedResponse)
    );

    let mut secret = response("SPY", "1h");
    let mut payload: serde_json::Value = serde_json::from_slice(&secret.body).unwrap();
    payload["api_key"] = serde_json::json!("planted");
    secret.body = serde_json::to_vec(&payload).unwrap();
    let (secret_adapter, _) = adapter(secret);
    assert_eq!(
        secret_adapter.fetch_for_ingest(&request("1h")),
        Err(UnavailableReason::ProviderVerificationBlock)
    );
}
