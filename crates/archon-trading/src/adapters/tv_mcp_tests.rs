use super::*;

struct FakeTransport {
    failures_before_success: u8,
    calls: u8,
    elapsed: Duration,
}

impl TvMcpTransport for FakeTransport {
    fn call_tool(&mut self, _tool_name: &str, _arguments: Value) -> Result<TimedMcpResult, String> {
        self.calls += 1;
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
    };
    let err = adapter(false, false)
        .pine_compile_check(&mut transport, "//@version=6")
        .unwrap_err();
    assert_eq!(err, TvMcpError::CompileSlaExceeded { elapsed_ms: 30_001 });
}
