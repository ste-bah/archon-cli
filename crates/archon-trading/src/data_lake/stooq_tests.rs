use super::stooq::*;
use super::{
    CapabilityRequest, NativeFetchRequest, ProbeBudget, ProviderAdapter, UnavailableReason,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Clock;
impl StooqClock for Clock {
    fn now_rfc3339(&self) -> String {
        "2025-01-06T12:00:00Z".into()
    }
}

struct Transport {
    calls: Mutex<Vec<StooqRequest>>,
    responses: Mutex<Vec<StooqResponse>>,
}

impl StooqTransport for Transport {
    fn execute(&self, request: &StooqRequest) -> Result<StooqResponse, StooqTransportError> {
        self.calls.lock().unwrap().push(request.clone());
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

fn response(status: u16, content_type: &str, body: &str) -> StooqResponse {
    StooqResponse {
        status,
        content_type: content_type.into(),
        headers: serde_json::json!({"server":"stooq","set-cookie":"secret"}),
        body: body.as_bytes().to_vec(),
    }
}

fn adapter(responses: Vec<StooqResponse>) -> (StooqAdapter, Arc<Transport>) {
    let transport = Arc::new(Transport {
        calls: Mutex::new(Vec::new()),
        responses: Mutex::new(responses),
    });
    (
        StooqAdapter::new(transport.clone(), Arc::new(Clock)),
        transport,
    )
}

fn request(timeframe: &str, start: &str, end: &str) -> NativeFetchRequest {
    NativeFetchRequest {
        provider: "stooq".into(),
        canonical_instrument: "SPY".into(),
        provider_symbol: "SPY.US".into(),
        timeframe: timeframe.into(),
        start: start.into(),
        end: end.into(),
    }
}

#[test]
fn stooq_exact_native_contract() {
    let csv =
        "Date,Open,High,Low,Close,Volume\n2025-01-03,10,12,9,11,100\n2025-01-06,11,13,10,12,200\n";
    let (adapter, transport) = adapter(vec![response(200, "text/csv", csv)]);
    let prepared = adapter
        .fetch_for_ingest(&request("1d", "2025-01-03", "2025-01-06"))
        .unwrap();
    assert_eq!(prepared.history.bars.len(), 2);
    assert_eq!(prepared.history.bars[0].close, 11.0);
    assert!(!prepared.history.provenance.resampled);
    assert_eq!(prepared.evidence.native_interval, "1d");
    assert_eq!(prepared.evidence.observed_bars, 2);
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
    assert_eq!(
        prepared.evidence.request_fingerprint,
        calls[0].fingerprint()
    );
    drop(calls);
    assert!(prepared.redacted_headers.get("set-cookie").is_none());
}

#[test]
fn stooq_capability_is_bounded_and_never_fetches_history() {
    let (adapter, transport) = adapter(Vec::new());
    let capability = CapabilityRequest {
        provider: "stooq".into(),
        canonical_instrument: "SPY".into(),
        provider_symbol: "SPY.US".into(),
        timeframe: "1d".into(),
        checked_at: "2025-01-06T12:00:00Z".into(),
    };
    let evidence = adapter
        .probe_capability(&capability, ProbeBudget::default())
        .unwrap();
    assert!(evidence.native_interval);
    assert!(evidence.historical_supported);
    assert_eq!(evidence.candles_examined, 0);
    assert_eq!(evidence.response_bytes, 0);
    assert!(transport.calls.lock().unwrap().is_empty());

    for budget in [
        ProbeBudget {
            max_candles: 0,
            ..ProbeBudget::default()
        },
        ProbeBudget {
            max_duration: Duration::ZERO,
            ..ProbeBudget::default()
        },
        ProbeBudget {
            max_response_bytes: 0,
            ..ProbeBudget::default()
        },
    ] {
        assert_eq!(
            adapter.probe_capability(&capability, budget),
            Err(UnavailableReason::ProbeLimitExceeded)
        );
    }
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[test]
fn stooq_each_required_timeframe_needs_direct_evidence() {
    for (timeframe, body) in [
        (
            "1d",
            "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,100\n",
        ),
        (
            "1w",
            "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,100\n",
        ),
        (
            "1mo",
            "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,100\n",
        ),
    ] {
        let (adapter, transport) = adapter(vec![response(200, "text/csv", body)]);
        let prepared = adapter
            .fetch_for_ingest(&request(timeframe, "2025-01-06", "2025-01-06"))
            .unwrap();
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].query.iter().any(|(key, _)| key == "i"));
        assert_eq!(prepared.evidence.native_interval, timeframe);
        assert_eq!(
            prepared.evidence.request_fingerprint,
            calls[0].fingerprint()
        );
    }

    {
        let (adapter, transport) = adapter(Vec::new());
        assert_eq!(
            adapter
                .fetch_for_ingest(&request("4h", "2025-01-06", "2025-01-06"))
                .unwrap_err(),
            UnavailableReason::ExactNativeIntervalUnsupported
        );
        assert!(transport.calls.lock().unwrap().is_empty());
    }

    for alias in ["1D", "1W", "1M", "daily", "d"] {
        let (adapter, transport) = adapter(Vec::new());
        assert_eq!(
            adapter.fetch_for_ingest(&request(alias, "2025-01-06", "2025-01-06")),
            Err(UnavailableReason::ExactNativeIntervalUnsupported)
        );
        assert!(transport.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn stooq_status_and_html_classification_precede_parsing() {
    for (response, expected) in [
        (
            response(401, "text/csv", "credential=must-not-leak"),
            UnavailableReason::Unauthorized401,
        ),
        (
            response(403, "text/csv", "Date,Open,High,Low,Close,Volume"),
            UnavailableReason::ProviderBlocked403,
        ),
        (
            response(404, "text/csv", "Date,Open,High,Low,Close,Volume"),
            UnavailableReason::NotFound404,
        ),
        (
            response(429, "text/plain", "rate limited"),
            UnavailableReason::HttpStatusError,
        ),
        (
            response(200, "text/html", "Date,Open,High,Low,Close,Volume"),
            UnavailableReason::ProviderVerificationBlock,
        ),
        (
            response(200, "text/plain", "access denied"),
            UnavailableReason::ProviderVerificationBlock,
        ),
    ] {
        let (adapter, transport) = adapter(vec![response]);
        let result = adapter.fetch_for_ingest(&request("1d", "2025-01-06", "2025-01-06"));
        assert_eq!(result.unwrap_err(), expected);
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }

    let (adapter, _) = adapter(vec![response(
        200,
        "application/json",
        "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,100\n",
    )]);
    assert_eq!(
        adapter.fetch_for_ingest(&request("1d", "2025-01-06", "2025-01-06")),
        Err(UnavailableReason::MalformedResponse)
    );
}

#[test]
fn stooq_blocked_request_has_no_bypass_or_mutated_retry() {
    let (adapter, transport) = adapter(vec![response(403, "text/html", "<html>blocked</html>")]);
    let requested = request("1d", "2025-01-06", "2025-01-06");
    assert_eq!(
        adapter.fetch_for_ingest(&requested).unwrap_err(),
        UnavailableReason::ProviderBlocked403
    );
    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let fingerprint = calls[0].fingerprint();
    assert_eq!(fingerprint, calls[0].fingerprint());
    assert_eq!(calls[0].url, STOOQ_DOWNLOAD_URL);
}

#[test]
fn stooq_rejects_malformed_duplicate_disordered_and_gapped_rows() {
    for body in [
        "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,,100\n",
        "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,13,100\n2025-01-08,10,12,9,11,100\n",
        "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,-1\n2025-01-08,10,12,9,11,100\n",
        "Date,Open,High,Low,Close,Volume\n2025-01-06,NaN,12,9,11,100\n2025-01-08,10,12,9,11,100\n",
        "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,100\n2025-01-06,10,12,9,11,100\n",
        "Date,Open,High,Low,Close,Volume\n2025-01-07,10,12,9,11,100\n2025-01-06,10,12,9,11,100\n",
        "Date,Open,High,Low,Close,Volume\n2025-01-06,10,12,9,11,100\n2025-01-08,10,12,9,11,100\n",
    ] {
        let (adapter, _) = adapter(vec![response(200, "text/csv", body)]);
        assert_eq!(
            adapter
                .fetch_for_ingest(&request("1d", "2025-01-06", "2025-01-08"))
                .unwrap_err(),
            UnavailableReason::MalformedResponse
        );
    }
}

#[test]
fn stooq_bounds_history_response_bytes() {
    let (adapter, transport) = adapter(vec![StooqResponse {
        status: 200,
        content_type: "text/csv".into(),
        headers: serde_json::json!({}),
        body: vec![b'x'; STOOQ_MAX_RESPONSE_BYTES + 1],
    }]);
    let result = adapter.fetch_for_ingest(&request("1d", "2025-01-06", "2025-01-06"));
    assert_eq!(result, Err(UnavailableReason::ProbeLimitExceeded));
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}
