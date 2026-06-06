use crate::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use crate::order_intent::{GatedOrderIntent, OrderIntent, TradingMode, pre_trade_gate};
use crate::regime::RegimeLabel;
use crate::risk_governor::{
    AccountState, MarketState, RiskDecisionStatus, RiskGovernor, RiskGovernorError,
};
use crate::risk_policy::RiskPolicy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperFill {
    pub quantity: f64,
    pub price: f64,
    pub slippage_bps: f64,
    pub latency_ms: u128,
    pub realized_pnl: f64,
    pub position_after: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperLedgerEntry {
    pub order_id: String,
    pub timestamp_unix_ms: u128,
    pub status: OrderStatus,
    pub intent: OrderIntent,
    pub fill: Option<PaperFill>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperSample {
    pub closed_trades: u32,
    pub calendar_days: u32,
    pub regime_ids: BTreeSet<u32>,
    pub postmortem_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleGateDecision {
    pub allowed: bool,
    pub missing_conditions: Vec<&'static str>,
    pub binding_condition: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaperTerminalError {
    NonPaperIntent,
    Risk(String),
    RiskRejected(Option<&'static str>),
    Audit(String),
}

pub struct PaperTerminal {
    governor: RiskGovernor,
    policy: RiskPolicy,
    audit: Option<AuditLedger>,
    ledger: Vec<PaperLedgerEntry>,
    next_order: u64,
}

impl PaperSample {
    pub fn from_closed_trades(
        closed_trades: u32,
        calendar_days: u32,
        regimes: &[RegimeLabel],
        postmortem_ready: bool,
    ) -> Self {
        Self {
            closed_trades,
            calendar_days,
            regime_ids: regimes.iter().map(|label| label.regime_id).collect(),
            postmortem_ready,
        }
    }
}

impl PaperTerminal {
    pub fn new(governor: RiskGovernor, policy: RiskPolicy) -> Self {
        Self {
            governor,
            policy,
            audit: None,
            ledger: Vec::new(),
            next_order: 1,
        }
    }

    pub fn with_audit(mut self, audit: AuditLedger) -> Self {
        self.audit = Some(audit);
        self
    }

    pub fn submit_order(
        &mut self,
        intent: OrderIntent,
        account: &AccountState,
        market: &MarketState,
    ) -> Result<GatedOrderIntent, PaperTerminalError> {
        if intent.mode != TradingMode::Paper {
            return Err(PaperTerminalError::NonPaperIntent);
        }
        let order_id = self.allocate_order_id();
        self.record_status(
            &order_id,
            OrderStatus::Requested,
            &intent,
            None,
            "requested",
        )?;
        let gated =
            pre_trade_gate(&self.governor, intent, account, market).map_err(risk_error_message)?;
        if gated.decision.status != RiskDecisionStatus::Approved {
            self.record_status(
                &order_id,
                OrderStatus::Rejected,
                &gated.intent,
                None,
                "risk rejected",
            )?;
            return Err(PaperTerminalError::RiskRejected(gated.decision.control_id));
        }
        self.record_status(
            &order_id,
            OrderStatus::Accepted,
            &gated.intent,
            None,
            "accepted",
        )?;
        Ok(gated)
    }

    pub fn record_fill(
        &mut self,
        order_id: &str,
        intent: &OrderIntent,
        fill: PaperFill,
    ) -> Result<(), PaperTerminalError> {
        self.record_status(order_id, OrderStatus::Filled, intent, Some(fill), "filled")
    }

    pub fn ledger(&self) -> &[PaperLedgerEntry] {
        &self.ledger
    }

    pub fn evaluate_sample_gate(&self, sample: &PaperSample) -> SampleGateDecision {
        let promotion = &self.policy.promotion;
        let mut missing = Vec::new();
        if sample.closed_trades < promotion.paper_min_closed_trades {
            missing.push("min_closed_trades");
        }
        if sample.calendar_days < promotion.paper_min_calendar_days {
            missing.push("min_calendar_days");
        }
        if sample.regime_ids.len() < promotion.paper_min_regimes as usize {
            missing.push("min_regimes");
        }
        if !sample.postmortem_ready {
            missing.push("postmortem_required");
        }
        SampleGateDecision {
            allowed: missing.is_empty(),
            binding_condition: missing.first().copied(),
            missing_conditions: missing,
        }
    }

    pub fn paper_before_live_satisfied(&self, sample: &PaperSample) -> bool {
        self.evaluate_sample_gate(sample).allowed
    }

    fn allocate_order_id(&mut self) -> String {
        let order_id = format!("paper-{}", self.next_order);
        self.next_order += 1;
        order_id
    }

    fn record_status(
        &mut self,
        order_id: &str,
        status: OrderStatus,
        intent: &OrderIntent,
        fill: Option<PaperFill>,
        note: &str,
    ) -> Result<(), PaperTerminalError> {
        let entry = PaperLedgerEntry {
            order_id: order_id.to_string(),
            timestamp_unix_ms: now_ms(),
            status,
            intent: intent.clone(),
            fill,
            note: note.to_string(),
        };
        if let Some(audit) = &self.audit {
            audit
                .append_status(self.audit_record(&entry))
                .map_err(|err| PaperTerminalError::Audit(err.to_string()))?;
        }
        self.ledger.push(entry);
        Ok(())
    }

    fn audit_record(&self, entry: &PaperLedgerEntry) -> NewLedgerRecord {
        NewLedgerRecord {
            actor: "paper-terminal".to_string(),
            strategy_id: entry.intent.strategy_id.clone(),
            policy_version: self.policy.version_hash.clone(),
            status: entry.status,
            risk_decision: json!({"paper_terminal": true, "note": entry.note}),
            order_intent: json!(&entry.intent),
            broker_response: json!({"paper_simulated": true, "fill": entry.fill}),
            account: json!({"mode": "paper"}),
            tax: TaxFields::default(),
            artefacts: vec![],
            maker_checker: None,
        }
    }
}

impl Default for PaperSample {
    fn default() -> Self {
        Self {
            closed_trades: 0,
            calendar_days: 0,
            regime_ids: BTreeSet::new(),
            postmortem_ready: false,
        }
    }
}

impl std::fmt::Display for PaperTerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonPaperIntent => formatter.write_str("paper terminal rejects non-paper intent"),
            Self::Risk(message) => {
                write!(formatter, "paper terminal risk governor error: {message}")
            }
            Self::RiskRejected(control) => {
                write!(formatter, "paper intent rejected by {control:?}")
            }
            Self::Audit(message) => write!(formatter, "paper audit failed: {message}"),
        }
    }
}

impl std::error::Error for PaperTerminalError {}

fn risk_error_message(error: RiskGovernorError) -> PaperTerminalError {
    PaperTerminalError::Risk(format!("{error:?}"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order_intent::{OrderPrices, OrderSide, OrderType};

    fn terminal() -> PaperTerminal {
        let policy = RiskPolicy::default();
        PaperTerminal::new(RiskGovernor::new(policy.clone()), policy)
    }

    #[test]
    fn t_paper_01_rejects_live_schema_and_preserves_paper_before_live_gate() {
        let mut terminal = terminal();
        let live = OrderIntent::new(
            "strategy-a",
            "SPY",
            OrderSide::Buy,
            OrderType::Market,
            1.0,
            OrderPrices::market(10.0),
            TradingMode::LivePilot,
        );

        let error = terminal
            .submit_order(live, &AccountState::default(), &MarketState::default())
            .expect_err("live schema rejected by paper terminal");

        assert_eq!(error, PaperTerminalError::NonPaperIntent);
        assert!(!terminal.paper_before_live_satisfied(&PaperSample::default()));
    }

    #[test]
    fn t_paper_02_uses_same_governor_path_and_records_distinct_statuses() {
        let mut terminal = terminal();
        let gated = terminal
            .submit_order(
                OrderIntent::default(),
                &AccountState::default(),
                &MarketState::default(),
            )
            .expect("paper order approved");
        terminal
            .record_fill(
                "paper-1",
                &gated.intent,
                PaperFill {
                    quantity: 1.0,
                    price: 10.0,
                    slippage_bps: 0.0,
                    latency_ms: gated.decision.latency_ms,
                    realized_pnl: 0.0,
                    position_after: 1.0,
                },
            )
            .expect("fill recorded");

        let statuses: Vec<OrderStatus> =
            terminal.ledger().iter().map(|entry| entry.status).collect();
        assert_eq!(
            statuses,
            vec![
                OrderStatus::Requested,
                OrderStatus::Accepted,
                OrderStatus::Filled
            ]
        );
        assert!(terminal.ledger()[2].fill.is_some());
    }

    #[test]
    fn t_paper_03_sample_gate_is_longest_binding_all_conditions_required() {
        let terminal = terminal();
        let mut regimes = BTreeSet::new();
        regimes.insert(0);
        let short = PaperSample {
            closed_trades: 199,
            calendar_days: 59,
            regime_ids: regimes,
            postmortem_ready: false,
        };

        let blocked = terminal.evaluate_sample_gate(&short);

        assert!(!blocked.allowed);
        assert_eq!(blocked.binding_condition, Some("min_closed_trades"));
        assert_eq!(blocked.missing_conditions.len(), 4);
    }

    #[test]
    fn a_paper_01_all_sample_conditions_and_postmortem_allow_promotion() {
        let terminal = terminal();
        let mut regimes = BTreeSet::new();
        regimes.insert(0);
        regimes.insert(1);
        let complete = PaperSample {
            closed_trades: 200,
            calendar_days: 60,
            regime_ids: regimes,
            postmortem_ready: true,
        };

        let decision = terminal.evaluate_sample_gate(&complete);

        assert!(decision.allowed);
        assert!(decision.missing_conditions.is_empty());
        assert!(terminal.paper_before_live_satisfied(&complete));
    }
}
