use super::*;

#[derive(Default)]
struct FakeTransport {
    discovery: bool,
    rest: Option<Result<OpenBbResponse, OpenBbError>>,
    sdk: Option<Result<OpenBbResponse, OpenBbError>>,
    mcp: Option<Result<OpenBbResponse, OpenBbError>>,
    calls: Vec<OpenBbRoute>,
}

impl OpenBbTransport for FakeTransport {
    fn discover(&self, _request: &OpenBbRequest) -> bool {
        self.discovery
    }

    fn rest(&mut self, _request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError> {
        self.calls.push(OpenBbRoute::Rest);
        self.rest
            .take()
            .unwrap_or(Err(OpenBbError::TransportUnavailable))
    }

    fn sdk(&mut self, _request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError> {
        self.calls.push(OpenBbRoute::Sdk);
        self.sdk
            .take()
            .unwrap_or(Err(OpenBbError::TransportUnavailable))
    }

    fn mcp(&mut self, _request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError> {
        self.calls.push(OpenBbRoute::McpResearchOnly);
        self.mcp
            .take()
            .unwrap_or(Err(OpenBbError::TransportUnavailable))
    }
}

fn request(provider: Provider) -> OpenBbRequest {
    OpenBbRequest {
        provider,
        allowlist_data_type: openbb_allowlist::DataType::Ohlcv,
        lake_data_type: LakeDataType::Ohlcv,
        endpoint: "/api/v1/equity/price/historical".into(),
        params: BTreeMap::from([("symbol".into(), "SPY".into())]),
        creds_profile_ref: "openbb/default".into(),
        cache_key: "openbb:spy:ohlcv:v1".into(),
        schema_version: "openbb-v1".into(),
    }
}

fn good_response() -> OpenBbResponse {
    OpenBbResponse {
        body: b"[{\"close\": 500.0}]".to_vec(),
        warnings: vec![],
        metadata: BTreeMap::from([
            ("symbol".into(), "SPY".into()),
            ("provider_symbol".into(), "SPY".into()),
            ("timezone".into(), "America/New_York".into()),
            ("adjustment".into(), "split_and_dividend".into()),
            ("coverage_start".into(), "2023-01-01".into()),
            ("coverage_end".into(), "2024-01-01".into()),
            ("expected_bars".into(), "252".into()),
            ("observed_bars".into(), "252".into()),
            ("missing_bars".into(), "0".into()),
        ]),
        quality: DataQuality {
            complete: true,
            licensed: true,
            timestamp_fresh: true,
            survivorship_adjusted: true,
            corporate_actions_adjusted: true,
            reproducible: true,
        },
    }
}

#[test]
fn t_openbb_01_rest_first_and_provenance_before_use() {
    let mut gateway = OpenBbGateway::default();
    let mut transport = FakeTransport {
        discovery: true,
        rest: Some(Ok(good_response())),
        ..Default::default()
    };
    let dataset = gateway
        .fetch(
            &mut transport,
            request(Provider::Polygon),
            AccessMode::LiveRequired,
            "now",
        )
        .unwrap();
    assert_eq!(transport.calls, vec![OpenBbRoute::Rest]);
    assert_eq!(gateway.provenance_log()[0], dataset.provenance);
    assert!(gateway.lake_registry().get("openbb:spy:ohlcv:v1").is_some());
    assert!(dataset.promotion_eligible);
}

#[test]
fn t_openbb_03_sdk_fallback_then_mcp_research_only() {
    let mut gateway = OpenBbGateway::default();
    let mut transport = FakeTransport {
        discovery: true,
        rest: Some(Err(OpenBbError::TransportUnavailable)),
        sdk: Some(Err(OpenBbError::TransportUnavailable)),
        mcp: Some(Ok(good_response())),
        ..Default::default()
    };
    let dataset = gateway
        .fetch(
            &mut transport,
            request(Provider::YFinance),
            AccessMode::Research,
            "now",
        )
        .unwrap();
    assert_eq!(dataset.provenance.route, OpenBbRoute::McpResearchOnly);
    assert!(!dataset.promotion_eligible);
    assert_eq!(transport.calls.len(), 3);
}

#[test]
fn t_openbb_04_live_fail_closed_and_secret_rejected() {
    let mut stale = good_response();
    stale.quality.timestamp_fresh = false;
    let mut gateway = OpenBbGateway::default();
    let mut transport = FakeTransport {
        discovery: true,
        rest: Some(Ok(stale)),
        ..Default::default()
    };
    let error = gateway
        .fetch(
            &mut transport,
            request(Provider::Polygon),
            AccessMode::LiveRequired,
            "now",
        )
        .unwrap_err();
    assert_eq!(error.code(), "ERR-OPENBB-STALE");
    assert!(gateway.provenance_log().is_empty());
    let mut secret_request = request(Provider::Polygon);
    secret_request.creds_profile_ref = "token=abc123".into();
    assert_eq!(
        gateway
            .fetch(
                &mut FakeTransport::default(),
                secret_request,
                AccessMode::Research,
                "now"
            )
            .unwrap_err(),
        OpenBbError::SecretMaterialRejected
    );
}

#[test]
fn a_openbb_02_denies_non_allowlisted_provider_pairs() {
    let mut denied = request(Provider::Fred);
    denied.allowlist_data_type = openbb_allowlist::DataType::News;
    let error = OpenBbGateway::default()
        .fetch(
            &mut FakeTransport::default(),
            denied,
            AccessMode::Research,
            "now",
        )
        .unwrap_err();
    assert_eq!(error.code(), "ERR-OPENBB-NOT-ALLOWLISTED");
}

#[test]
fn ec_trl_09_429_uses_cache_and_ec_trl_10_live_miss_fails() {
    let mut gateway = OpenBbGateway::default();
    let mut first = FakeTransport {
        discovery: true,
        rest: Some(Ok(good_response())),
        ..Default::default()
    };
    let seed = gateway
        .fetch(
            &mut first,
            request(Provider::Polygon),
            AccessMode::LiveRequired,
            "then",
        )
        .unwrap();
    let mut rate_limited = FakeTransport {
        discovery: true,
        rest: Some(Err(OpenBbError::RateLimited)),
        ..Default::default()
    };
    let cached = gateway
        .fetch(
            &mut rate_limited,
            request(Provider::Polygon),
            AccessMode::LiveRequired,
            "now",
        )
        .unwrap();
    assert_eq!(cached.provenance.checksum, seed.provenance.checksum);
    let mut miss = FakeTransport {
        discovery: true,
        rest: Some(Err(OpenBbError::RateLimited)),
        ..Default::default()
    };
    let err = OpenBbGateway::default()
        .fetch(
            &mut miss,
            request(Provider::Polygon),
            AccessMode::LiveRequired,
            "now",
        )
        .unwrap_err();
    assert_eq!(err, OpenBbError::CacheMissLiveRequired);
}
