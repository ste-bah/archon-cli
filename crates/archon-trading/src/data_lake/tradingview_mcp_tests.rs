use super::*;
use std::collections::VecDeque;

#[derive(Default)]
struct SpyTransport {
    calls: Vec<(&'static str, Value)>,
    responses: VecDeque<Result<Value, String>>,
}

impl TradingViewMcpTransport for SpyTransport {
    fn call(&mut self, action: &'static str, arguments: Value) -> Result<Value, String> {
        self.calls.push((action, arguments));
        self.responses
            .pop_front()
            .unwrap_or_else(|| Err("unexpected call".into()))
    }
}

fn request() -> TradingViewHistoryRequest {
    TradingViewHistoryRequest {
        canonical_instrument: "ES".into(),
        provider_symbol: "CME_MINI:ES1!".into(),
        timeframe: "1D".into(),
        start: "2026-01-01T00:00:00Z".into(),
        end: "2026-01-02T00:00:00Z".into(),
        expected_bars: 2,
    }
}

fn bar(time: i64, close: f64) -> Value {
    json!({
        "time": time,
        "open": close,
        "high": close + 1.0,
        "low": close - 1.0,
        "close": close,
        "volume": close * 100.0,
    })
}

fn successful_transport() -> SpyTransport {
    SpyTransport {
        responses: VecDeque::from([
            Ok(json!({
                "success": true,
                "cdp_connected": true,
                "api_available": true
            })),
            Ok(json!({
                "success": true,
                "symbol":"BATS:GOOGL",
                "resolution":"1D"
            })),
            Ok(json!({
                "success":true,
                "requested_symbol":"CME_MINI:ES1!",
                "requested_timeframe":"1D",
                "actual_symbol":"CME_MINI:ES1!",
                "actual_timeframe":"1D",
                "requested_count":2,
                "bar_count":2,
                "bars":[
                    bar(1_767_225_600, 10.0),
                    bar(1_767_312_000, 11.0)
                ]
            })),
        ]),
        ..SpyTransport::default()
    }
}

fn successful_quote_transport() -> SpyTransport {
    SpyTransport {
        responses: VecDeque::from([
            Ok(json!({
                "success": true,
                "cdp_connected": true,
                "api_available": true
            })),
            Ok(json!({
                "success": true,
                "symbol":"CME_MINI:ES1!",
                "resolution":"1D"
            })),
            Ok(json!({
                "success": true,
                "symbol":"CME_MINI:ES1!",
                "last":10.5
            })),
        ]),
        ..SpyTransport::default()
    }
}

#[test]
fn tradingview_mcp_preflight_is_mandatory() {
    let mut adapter = TradingViewMcpDataAdapter::new(successful_transport());
    let history = adapter.fetch_native_history(request()).unwrap();
    assert_eq!(history.bars.len(), 2);
    assert_eq!(
        history.action_counts,
        TradingViewMcpActionCounts::exact_history_sequence()
    );
    assert_eq!(
        adapter
            .transport()
            .calls
            .iter()
            .map(|(action, _)| *action)
            .collect::<Vec<_>>(),
        [TV_HEALTH_CHECK, TV_CHART_GET_STATE, TV_DATA_GET_OHLCV]
    );
}

#[test]
fn tradingview_mcp_exact_symbols_and_intervals() {
    let mappings = [
        ("ES", "CME_MINI:ES1!"),
        ("NQ", "CME_MINI:NQ1!"),
        ("SPY", "SPY"),
        ("QQQ", "QQQ"),
        ("BTCUSDT", "BINANCE:BTCUSDT"),
        ("ETHUSDT", "BINANCE:ETHUSDT"),
    ];
    for (canonical, provider) in mappings {
        assert_eq!(tradingview_symbol(canonical), Some(provider));
    }
    for timeframe in TRADINGVIEW_NATIVE_TIMEFRAMES {
        let mut valid = request();
        valid.timeframe = (*timeframe).into();
        assert!(validate_request(&valid).is_ok());
    }
    for timeframe in ["1d", "D", "4H", "1h", "15m"] {
        let mut invalid = request();
        invalid.timeframe = timeframe.into();
        let mut adapter = TradingViewMcpDataAdapter::new(SpyTransport::default());
        assert!(matches!(
            adapter.fetch_native_history(invalid),
            Err(TradingViewMcpError::UnsupportedTimeframe(_))
        ));
        assert!(adapter.transport().calls.is_empty());
    }
    for symbol in ["CME_MINI:ES", "cme_mini:ES1!", "CME:ES1!", " ES "] {
        let mut invalid = request();
        invalid.provider_symbol = symbol.into();
        let mut adapter = TradingViewMcpDataAdapter::new(SpyTransport::default());
        assert!(matches!(
            adapter.fetch_native_history(invalid),
            Err(TradingViewMcpError::InvalidIdentity(_))
        ));
        assert!(adapter.transport().calls.is_empty());
    }
}

#[test]
fn tradingview_mcp_capability_probe_is_bounded() {
    let mut zero = request();
    zero.expected_bars = 0;
    let mut adapter = TradingViewMcpDataAdapter::new(SpyTransport::default());
    assert!(matches!(
        adapter.fetch_native_history(zero),
        Err(TradingViewMcpError::UnboundedRequest(0))
    ));
    let mut excessive = request();
    excessive.expected_bars = TRADINGVIEW_MAX_BARS_PER_CALL + 1;
    assert!(matches!(
        adapter.fetch_native_history(excessive),
        Err(TradingViewMcpError::UnboundedRequest(501))
    ));
    assert!(adapter.transport().calls.is_empty());
}

#[test]
fn tradingview_mcp_rejects_foreign_or_malformed_response() {
    let mut transport = successful_transport();
    transport.responses[2] = Ok(json!({
        "success":true,
        "actual_symbol":"CME_MINI:ES1!",
        "actual_timeframe":"1D",
        "bar_count":1,
        "bars":[bar(1_767_225_600, 10.0)]
    }));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert_eq!(
        adapter.fetch_native_history(request()),
        Err(TradingViewMcpError::LimitedCoverage {
            expected: 2,
            returned: 1,
        })
    );

    let mut transport = successful_transport();
    transport.responses[2] = Ok(json!({
        "success":true,
        "actual_symbol":"CME_MINI:NQ1!",
        "actual_timeframe":"1D",
        "bar_count":2,
        "bars":[bar(1_767_225_600, 10.0), bar(1_767_312_000, 11.0)]
    }));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert_eq!(
        adapter.fetch_native_history(request()),
        Err(TradingViewMcpError::ForeignIdentity)
    );

    let mut transport = successful_transport();
    transport.responses[2] = Ok(json!({
        "success":true,
        "actual_symbol":"CME_MINI:ES1!",
        "actual_timeframe":"1D",
        "bar_count":2,
        "bars":[{"time":"not-a-number"}]
    }));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert!(matches!(
        adapter.fetch_native_history(request()),
        Err(TradingViewMcpError::InvalidPayload(_))
    ));
}

#[test]
fn tradingview_quote_preflight_and_identity_are_mandatory() {
    let mut adapter = TradingViewMcpDataAdapter::new(successful_quote_transport());
    let snapshot = adapter
        .fetch_quote_snapshot("ES", "CME_MINI:ES1!", 1_767_225_600)
        .unwrap();
    assert_eq!(snapshot.payload["last"], 10.5);
    assert_eq!(
        adapter
            .transport()
            .calls
            .iter()
            .map(|(action, _)| *action)
            .collect::<Vec<_>>(),
        [TV_HEALTH_CHECK, TV_CHART_GET_STATE, TV_QUOTE_GET]
    );

    let mut transport = successful_quote_transport();
    transport.responses[1] = Ok(json!({
        "success": true,
        "symbol":"CME_MINI:NQ1!",
        "resolution":"1D"
    }));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert_eq!(
        adapter.fetch_quote_snapshot("ES", "CME_MINI:ES1!", 1_767_225_600),
        Err(TradingViewMcpError::ForeignIdentity)
    );
    assert_eq!(adapter.transport().calls.len(), 2);
}

#[test]
fn tradingview_rejects_sensitive_payloads() {
    let mut transport = successful_transport();
    transport.responses[2] = Ok(json!({
        "success":true,
        "actual_symbol":"CME_MINI:ES1!",
        "actual_timeframe":"1D",
        "bar_count":2,
        "bars":[bar(1_767_225_600, 10.0), bar(1_767_312_000, 11.0)],
        "nested":{"target_id":"sensitive"}
    }));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert!(matches!(
        adapter.fetch_native_history(request()),
        Err(TradingViewMcpError::InvalidPayload(_))
    ));
}

#[test]
fn unavailable_health_or_chart_state_stops_before_ohlcv() {
    let mut transport = successful_transport();
    transport.responses[0] = Ok(json!({
        "success": true,
        "cdp_connected": false,
        "api_available": true
    }));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert_eq!(
        adapter.fetch_native_history(request()),
        Err(TradingViewMcpError::Unhealthy)
    );
    assert_eq!(adapter.transport().calls.len(), 1);

    let mut transport = successful_transport();
    transport.responses[1] = Ok(json!({"success": false}));
    let mut adapter = TradingViewMcpDataAdapter::new(transport);
    assert_eq!(
        adapter.fetch_native_history(request()),
        Err(TradingViewMcpError::Unhealthy)
    );
    assert_eq!(adapter.transport().calls.len(), 2);
}
