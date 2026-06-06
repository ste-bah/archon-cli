use anyhow::{Result, anyhow};
use archon_mcp::types::{McpToolResult, ToolContent};
use archon_trading::adapters::tv_mcp::{TimedMcpResult, TvMcpConfig, TvMcpTransport};
use archon_trading::adapters::tv_paper::{
    TradingViewPaperAdapter, TradingViewReplayReceipt, TradingViewReplayRequest,
};
use archon_trading::audit_ledger::AuditLedger;
use archon_trading::maker_checker::MakerCheckerApproval;
use archon_trading::order_intent::OrderIntent;
use archon_trading::paper_terminal::{PaperLedgerEntry, PaperSample, PaperTerminal};
use archon_trading::risk_governor::{RiskDecision, RiskGovernor};
use archon_trading::risk_policy::RiskPolicy;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cli_args::TradingCliPaperAction;
use crate::command::trading_io::{read_json, write_or_render};
use crate::command::trading_tools::{join_output, project_root, run_node_script, tv_cli};

#[derive(Debug, Clone, Serialize)]
struct PaperSubmitReport {
    accepted: bool,
    error: Option<String>,
    decision: Option<RiskDecision>,
    ledger: Vec<PaperLedgerEntry>,
}

#[derive(Debug, Serialize)]
struct PaperSampleReport {
    allowed: bool,
    missing_conditions: Vec<&'static str>,
    binding_condition: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct PaperTradingViewReplayReport {
    accepted: bool,
    paper: PaperSubmitReport,
    tradingview: Option<TradingViewReplayReceipt>,
    error: Option<String>,
}

pub(crate) fn render_paper(action: &TradingCliPaperAction) -> Result<String> {
    match action {
        TradingCliPaperAction::Submit {
            intent,
            account,
            market,
            audit,
            out,
        } => submit(
            intent,
            account.as_deref(),
            market.as_deref(),
            audit.as_deref(),
            out.as_deref(),
        ),
        TradingCliPaperAction::Sample { sample, out } => {
            let sample: PaperSample = read_json(sample, "PaperSample")?;
            let policy = RiskPolicy::default();
            let decision = PaperTerminal::new(RiskGovernor::new(policy.clone()), policy)
                .evaluate_sample_gate(&sample);
            write_or_render(
                &PaperSampleReport {
                    allowed: decision.allowed,
                    missing_conditions: decision.missing_conditions,
                    binding_condition: decision.binding_condition,
                },
                out.as_deref(),
            )
        }
        TradingCliPaperAction::TradingviewReplaySubmit {
            target,
            intent,
            account,
            market,
            audit,
            adapter_pin,
            write_tier_enabled,
            sandbox_certified,
            approval_id,
            maker,
            checker,
            rationale,
            out,
        } => submit_tradingview_replay(
            target.as_ref(),
            intent,
            account.as_deref(),
            market.as_deref(),
            audit.as_deref(),
            TvApprovalInput {
                adapter_pin,
                write_tier_enabled: *write_tier_enabled,
                sandbox_certified: *sandbox_certified,
                approval_id,
                maker,
                checker,
                rationale,
            },
            out.as_deref(),
        ),
    }
}

fn submit(
    intent_path: &Path,
    account_path: Option<&Path>,
    market_path: Option<&Path>,
    audit_path: Option<&Path>,
    out: Option<&Path>,
) -> Result<String> {
    let (report, _) = paper_gate(intent_path, account_path, market_path, audit_path)?;
    write_or_render(&report, out)
}

fn paper_gate(
    intent_path: &Path,
    account_path: Option<&Path>,
    market_path: Option<&Path>,
    audit_path: Option<&Path>,
) -> Result<(PaperSubmitReport, Option<OrderIntent>)> {
    let intent: OrderIntent = read_json(intent_path, "OrderIntent")?;
    let account = read_or_default(account_path, "AccountState")?;
    let market = read_or_default(market_path, "MarketState")?;
    let policy = RiskPolicy::default();
    let mut terminal = PaperTerminal::new(RiskGovernor::new(policy.clone()), policy);
    if let Some(path) = audit_path {
        terminal = terminal.with_audit(AuditLedger::open(path).map_err(|err| anyhow!("{err}"))?);
    }
    let result = terminal.submit_order(intent, &account, &market);
    let (accepted, error, decision, gated_intent) = match result {
        Ok(gated) => {
            let intent = gated.intent.clone();
            (true, None, Some(gated.decision), Some(intent))
        }
        Err(err) => (false, Some(format!("{err:?}")), None, None),
    };
    let report = PaperSubmitReport {
        accepted,
        error,
        decision,
        ledger: terminal.ledger().to_vec(),
    };
    Ok((report, gated_intent))
}

struct TvApprovalInput<'a> {
    adapter_pin: &'a str,
    write_tier_enabled: bool,
    sandbox_certified: bool,
    approval_id: &'a str,
    maker: &'a str,
    checker: &'a str,
    rationale: &'a str,
}

fn submit_tradingview_replay(
    target: Option<&PathBuf>,
    intent_path: &Path,
    account_path: Option<&Path>,
    market_path: Option<&Path>,
    audit_path: Option<&Path>,
    input: TvApprovalInput<'_>,
    out: Option<&Path>,
) -> Result<String> {
    let (paper, gated_intent) = paper_gate(intent_path, account_path, market_path, audit_path)?;
    let Some(intent) = gated_intent else {
        let report = PaperTradingViewReplayReport {
            accepted: false,
            paper,
            tradingview: None,
            error: Some("paper risk gate rejected order; TradingView replay not called".into()),
        };
        return write_or_render(&report, out);
    };

    let root = project_root(target)?;
    let approval = MakerCheckerApproval::new(
        input.approval_id,
        input.maker,
        input.checker,
        "tradingview-replay-submit",
        true,
        input.rationale,
    );
    let adapter = TradingViewPaperAdapter::new(TvMcpConfig {
        adapter_pin: input.adapter_pin.to_string(),
        sandbox_certified: input.sandbox_certified,
        write_tier_enabled: input.write_tier_enabled,
    })
    .map_err(|err| anyhow!("invalid TradingView MCP adapter config: {err:?}"))?;
    let mut transport = TradingViewCliTransport::new(root)?;
    let tradingview = adapter
        .submit_replay(
            &mut transport,
            TradingViewReplayRequest { intent },
            &approval,
        )
        .map_err(|err| anyhow!("TradingView replay submit failed: {err}"))?;
    let report = PaperTradingViewReplayReport {
        accepted: true,
        paper,
        tradingview: Some(tradingview),
        error: None,
    };
    write_or_render(&report, out)
}

fn read_or_default<T>(path: Option<&std::path::Path>, label: &str) -> Result<T>
where
    T: Default + serde::de::DeserializeOwned,
{
    match path {
        Some(path) => read_json(path, label),
        None => Ok(T::default()),
    }
}

struct TradingViewCliTransport {
    root: PathBuf,
    cli: PathBuf,
}

impl TradingViewCliTransport {
    fn new(root: PathBuf) -> Result<Self> {
        let cli = tv_cli(&root);
        if !cli.is_file() {
            return Err(anyhow!(
                "TradingView MCP CLI missing at {}; run scripts/setup-trading-tools.sh --target {}",
                cli.display(),
                root.display()
            ));
        }
        Ok(Self { root, cli })
    }
}

impl TvMcpTransport for TradingViewCliTransport {
    fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<TimedMcpResult, String> {
        if tool_name != "tv.terminal_interaction" {
            return Err(format!("unsupported TradingView write tool: {tool_name}"));
        }
        if arguments.get("command").and_then(Value::as_str) != Some("replay_trade") {
            return Err("unsupported TradingView terminal command".into());
        }
        let action = arguments
            .get("trade_action")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing replay trade action".to_string())?;
        let start = Instant::now();
        let output = run_node_script(
            &self.root,
            &self.cli,
            &["replay".into(), "trade".into(), action.into()],
        )
        .map_err(|err| err.to_string())?;
        let text = join_output(&output);
        Ok(TimedMcpResult {
            result: McpToolResult {
                content: vec![ToolContent::Text { text }],
                is_error: output.status != 0,
            },
            elapsed: start.elapsed(),
        })
    }
}
