use crate::adapters::tv_mcp::{
    TradingViewMcpAdapter, TvMcpConfig, TvMcpError, TvMcpResponse, TvMcpTransport, TvWriteAction,
};
use crate::maker_checker::MakerCheckerApproval;
use crate::order_intent::{OrderIntent, OrderSide, OrderType, TradingMode};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Governed TradingView replay submission.
///
/// The supported TradingView MCP integration exposes replay trade actions, not
/// a broker-grade paper-account order API. Archon treats this as paper/replay
/// evidence only and still requires the normal risk gate before this adapter is
/// called by the CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradingViewReplayRequest {
    pub intent: OrderIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingViewReplayReceipt {
    pub accepted: bool,
    pub replay_action: String,
    pub response: TvMcpResponse,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TvPaperError {
    NonPaperIntent,
    UnsupportedReplayOrderType(OrderType),
    Mcp(TvMcpError),
}

#[derive(Debug, Clone)]
pub struct TradingViewPaperAdapter {
    mcp: TradingViewMcpAdapter,
}

impl TradingViewPaperAdapter {
    pub fn new(config: TvMcpConfig) -> Result<Self, TvMcpError> {
        Ok(Self {
            mcp: TradingViewMcpAdapter::new(config)?,
        })
    }

    pub fn submit_replay<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        request: TradingViewReplayRequest,
        approval: &MakerCheckerApproval,
    ) -> Result<TradingViewReplayReceipt, TvPaperError> {
        let action = replay_action(&request.intent)?;
        let response = self
            .mcp
            .write_action(
                transport,
                TvWriteAction::TerminalInteraction,
                json!({
                    "command": "replay_trade",
                    "trade_action": action,
                    "strategy_id": request.intent.strategy_id,
                    "instrument": request.intent.instrument,
                    "quantity": request.intent.quantity,
                    "intent": request.intent,
                }),
                Some(approval),
            )
            .map_err(TvPaperError::Mcp)?;

        Ok(TradingViewReplayReceipt {
            accepted: true,
            replay_action: action.to_string(),
            response,
            note: "TradingView replay trade accepted; no live broker order submitted".into(),
        })
    }
}

fn replay_action(intent: &OrderIntent) -> Result<&'static str, TvPaperError> {
    if intent.mode != TradingMode::Paper {
        return Err(TvPaperError::NonPaperIntent);
    }
    if intent.order_type != OrderType::Market {
        return Err(TvPaperError::UnsupportedReplayOrderType(intent.order_type));
    }
    Ok(match intent.side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    })
}

impl std::fmt::Display for TvPaperError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonPaperIntent => formatter.write_str("ERR-TV-PAPER-NON-PAPER-INTENT"),
            Self::UnsupportedReplayOrderType(order_type) => {
                write!(
                    formatter,
                    "ERR-TV-PAPER-UNSUPPORTED-ORDER-TYPE:{order_type:?}"
                )
            }
            Self::Mcp(err) => write!(formatter, "ERR-TV-PAPER-MCP:{err:?}"),
        }
    }
}

impl std::error::Error for TvPaperError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order_intent::{OrderPrices, OrderSide};
    use archon_mcp::types::{McpToolResult, ToolContent};
    use serde_json::Value;
    use std::time::Duration;

    struct RecordingTransport {
        calls: Vec<(String, Value)>,
    }

    impl TvMcpTransport for RecordingTransport {
        fn call_tool(
            &mut self,
            tool_name: &str,
            arguments: Value,
        ) -> Result<crate::adapters::tv_mcp::TimedMcpResult, String> {
            self.calls.push((tool_name.to_string(), arguments));
            Ok(crate::adapters::tv_mcp::TimedMcpResult {
                result: McpToolResult {
                    content: vec![ToolContent::Text {
                        text: "replay ok".into(),
                    }],
                    is_error: false,
                },
                elapsed: Duration::from_millis(7),
            })
        }
    }

    fn adapter(write_enabled: bool, sandbox_certified: bool) -> TradingViewPaperAdapter {
        TradingViewPaperAdapter::new(TvMcpConfig {
            adapter_pin: "vendor@abcdef1".into(),
            sandbox_certified,
            write_tier_enabled: write_enabled,
        })
        .expect("valid adapter")
    }

    fn approval() -> MakerCheckerApproval {
        MakerCheckerApproval::new("r1", "maker", "checker", "tv-replay", true, "approved")
    }

    #[test]
    fn replay_submit_requires_paper_market_intent() {
        let mut intent = OrderIntent::default();
        intent.mode = TradingMode::LivePilot;
        let mut transport = RecordingTransport { calls: Vec::new() };
        let err = adapter(true, true)
            .submit_replay(
                &mut transport,
                TradingViewReplayRequest { intent },
                &approval(),
            )
            .unwrap_err();
        assert_eq!(err, TvPaperError::NonPaperIntent);
        assert!(transport.calls.is_empty());
    }

    #[test]
    fn replay_submit_rejects_non_market_orders() {
        let mut intent = OrderIntent::default();
        intent.order_type = OrderType::Limit;
        intent.prices = OrderPrices {
            reference: 10.0,
            limit: Some(9.5),
            stop: None,
        };
        let mut transport = RecordingTransport { calls: Vec::new() };
        let err = adapter(true, true)
            .submit_replay(
                &mut transport,
                TradingViewReplayRequest { intent },
                &approval(),
            )
            .unwrap_err();
        assert_eq!(
            err,
            TvPaperError::UnsupportedReplayOrderType(OrderType::Limit)
        );
        assert!(transport.calls.is_empty());
    }

    #[test]
    fn replay_submit_uses_mcp_write_gate() {
        let mut transport = RecordingTransport { calls: Vec::new() };
        let err = adapter(false, false)
            .submit_replay(
                &mut transport,
                TradingViewReplayRequest {
                    intent: OrderIntent::default(),
                },
                &approval(),
            )
            .unwrap_err();
        assert!(matches!(
            err,
            TvPaperError::Mcp(TvMcpError::WriteTierDenied { .. })
        ));
        assert!(transport.calls.is_empty());
    }

    #[test]
    fn replay_submit_calls_terminal_interaction_with_trade_action() {
        let mut intent = OrderIntent::default();
        intent.side = OrderSide::Sell;
        let mut transport = RecordingTransport { calls: Vec::new() };
        let receipt = adapter(true, true)
            .submit_replay(
                &mut transport,
                TradingViewReplayRequest { intent },
                &approval(),
            )
            .expect("replay submit accepted");

        assert_eq!(receipt.replay_action, "sell");
        assert_eq!(transport.calls.len(), 1);
        assert_eq!(transport.calls[0].0, "tv.terminal_interaction");
        assert_eq!(transport.calls[0].1["command"], "replay_trade");
        assert_eq!(transport.calls[0].1["trade_action"], "sell");
    }
}
