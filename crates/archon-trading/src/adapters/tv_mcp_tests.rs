use super::*;

struct FakeTransport {
    failures_before_success: u8,
    calls: u8,
    elapsed: Duration,
    called_tools: Vec<String>,
}

impl TvMcpTransport for FakeTransport {
    fn call_tool(&mut self, tool_name: &str, _arguments: Value) -> Result<TimedMcpResult, String> {
        self.calls += 1;
        self.called_tools.push(tool_name.into());
        if self.calls <= self.failures_before_success {
            return Err("mcp unavailable".into());
        }
        Ok(TimedMcpResult {
            result: McpToolResult {
                content: vec![ToolContent::Text { text: "ok".into() }],
                is_error: false,
            },
            elapsed: self.elapsed,
        })
    }
}

fn adapter(write_enabled: bool, sandbox_certified: bool) -> TradingViewMcpAdapter {
    TradingViewMcpAdapter::new(TvMcpConfig {
        adapter_pin: "vendor@abcdef1".into(),
        sandbox_certified,
        write_tier_enabled: write_enabled,
    })
    .expect("valid adapter pin")
}

#[test]
fn read_tier_is_default_on_and_pinned() {
    let mut transport = FakeTransport {
        failures_before_success: 0,
        calls: 0,
        elapsed: Duration::from_millis(20),
        called_tools: Vec::new(),
    };
    let response = adapter(false, false)
        .docs_lookup(&mut transport, "pine v6")
        .unwrap();
    assert_eq!(response.adapter_pin, "vendor@abcdef1");
    assert_eq!(response.content_text, vec!["ok"]);
}

#[test]
fn t_pine_05_write_tier_denies_without_enablement_and_sandbox() {
    let mut transport = FakeTransport {
        failures_before_success: 0,
        calls: 0,
        elapsed: Duration::from_millis(1),
        called_tools: Vec::new(),
    };
    let err = adapter(false, false)
        .write_action(&mut transport, TvWriteAction::AlertSetup, json!({}), None)
        .unwrap_err();
    assert_eq!(transport.calls, 0);
    assert_eq!(err, write_denied("write tier disabled"));
}

#[test]
fn write_tier_requires_distinct_maker_checker_pair() {
    let approval = MakerCheckerApproval::new("r1", "alice", "bob", "alert", true, "ok");
    let mut transport = FakeTransport {
        failures_before_success: 0,
        calls: 0,
        elapsed: Duration::from_millis(5),
        called_tools: Vec::new(),
    };
    let response = adapter(true, true)
        .write_action(
            &mut transport,
            TvWriteAction::AlertSetup,
            json!({}),
            Some(&approval),
        )
        .unwrap();
    assert_eq!(response.attempts, 1);
}

#[test]
fn ec_trl_06_mcp_failure_fails_closed_after_three_retries() {
    let mut transport = FakeTransport {
        failures_before_success: 5,
        calls: 0,
        elapsed: Duration::from_millis(1),
        called_tools: Vec::new(),
    };
    let err = adapter(false, false)
        .script_version_sync(&mut transport, "s1")
        .unwrap_err();
    assert_eq!(transport.calls, MAX_RETRIES);
    assert_eq!(
        err,
        TvMcpError::McpFailureEscalated {
            attempts: 3,
            partial_script_persisted: false
        }
    );
}

#[test]
fn compile_check_enforces_thirty_second_sla() {
    let mut transport = FakeTransport {
        failures_before_success: 0,
        calls: 0,
        elapsed: Duration::from_millis(30_001),
        called_tools: Vec::new(),
    };
    let err = adapter(false, false)
        .pine_compile_check(&mut transport, "//@version=6")
        .unwrap_err();
    assert_eq!(err, TvMcpError::CompileSlaExceeded { elapsed_ms: 30_001 });
}

#[test]
fn native_ohlcv_candles_require_supported_interval_preflight_contract() {
    let preflight = TvNativeOhlcvPreflight::require("BINANCE:BTCUSDT", "240").unwrap();
    assert_eq!(preflight.health_tool, "mcp__tradingview__tv_health_check");
    assert_eq!(
        preflight.chart_state_tool,
        "mcp__tradingview__chart_get_state"
    );
    assert_eq!(preflight.ohlcv_tool, "mcp__tradingview__data_get_ohlcv");
    let contract = preflight.request_contract();
    assert_eq!(
        contract["source_classification"],
        "chart_equivalent_research_data"
    );
    assert_eq!(contract["not_institutional_vendor_data"], true);
}

#[test]
fn native_ohlcv_candles_run_health_and_state_before_fetch() {
    let mut transport = FakeTransport {
        failures_before_success: 0,
        calls: 0,
        elapsed: Duration::from_millis(1),
        called_tools: Vec::new(),
    };

    let response = adapter(false, false)
        .ohlcv_native_candles(
            &mut transport,
            "BINANCE:BTCUSDT",
            "240",
            "2024-01-01",
            "2024-01-02",
        )
        .unwrap();

    assert_eq!(response.content_text, vec!["ok"]);
    assert_eq!(
        transport.called_tools,
        vec![
            "mcp__tradingview__tv_health_check",
            "mcp__tradingview__chart_get_state",
            "mcp__tradingview__data_get_ohlcv",
        ]
    );
}

#[test]
fn native_ohlcv_candles_fail_closed_when_health_preflight_fails() {
    let mut transport = FakeTransport {
        failures_before_success: MAX_RETRIES,
        calls: 0,
        elapsed: Duration::from_millis(1),
        called_tools: Vec::new(),
    };

    let err = adapter(false, false)
        .ohlcv_native_candles(
            &mut transport,
            "BINANCE:BTCUSDT",
            "240",
            "2024-01-01",
            "2024-01-02",
        )
        .unwrap_err();

    assert_eq!(transport.calls, MAX_RETRIES);
    assert!(matches!(err, TvMcpError::NativeOhlcvUnavailable { .. }));
    assert!(
        transport
            .called_tools
            .iter()
            .all(|tool| tool == "mcp__tradingview__tv_health_check")
    );
}

#[test]
fn native_ohlcv_candles_fail_closed_on_unsupported_interval() {
    let mut transport = FakeTransport {
        failures_before_success: 0,
        calls: 0,
        elapsed: Duration::from_millis(1),
        called_tools: Vec::new(),
    };
    let err = adapter(false, false)
        .ohlcv_native_candles(
            &mut transport,
            "BINANCE:BTCUSDT",
            "4H",
            "2024-01-01",
            "2024-01-02",
        )
        .unwrap_err();
    assert_eq!(transport.calls, 0);
    assert!(matches!(err, TvMcpError::NativeOhlcvUnavailable { .. }));
}
