use crate::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use crate::order_intent::{OrderIntent, TradingMode};
use crate::risk_controls::{CONTROL_ORDER, ControlId, HaltAttribution, evaluate_control};
use crate::risk_policy::{RiskPolicy, RiskRuntimeState, RiskStateStore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountState {
    pub equity: f64,
    pub gross_exposure: f64,
    pub net_exposure: f64,
    pub correlated_exposure_pct: f64,
    pub daily_loss_pct: f64,
    pub weekly_loss_pct: f64,
    pub strategy_drawdown_pct: f64,
    pub pilot_drawdown_pct: f64,
    pub order_rate_per_min: u32,
    pub message_to_fill_ratio: f64,
    pub allowed_instruments: Vec<String>,
    pub leverage_after: f64,
    pub cooldown_until_unix_ms: Option<i64>,
    pub governor_available: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketState {
    pub event_window_active: bool,
    pub realized_vol: f64,
    pub median_vol: f64,
    pub data_age_seconds: u64,
    pub broker_healthy: bool,
    pub broker_last_seen_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskDecisionStatus {
    Approved,
    Rejected,
    Halted,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskDecision {
    pub status: RiskDecisionStatus,
    pub control_id: Option<&'static str>,
    pub attribution: Option<HaltAttribution>,
    pub terminal: bool,
    pub recoverable: bool,
    pub latency_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskGovernorError {
    Audit(String),
    State(String),
}

pub struct RiskGovernor {
    policy: RiskPolicy,
    audit: Option<AuditLedger>,
    state_store: Option<RiskStateStore>,
}

impl Default for AccountState {
    fn default() -> Self {
        Self {
            equity: 100_000.0,
            gross_exposure: 0.0,
            net_exposure: 0.0,
            correlated_exposure_pct: 0.0,
            daily_loss_pct: 0.0,
            weekly_loss_pct: 0.0,
            strategy_drawdown_pct: 0.0,
            pilot_drawdown_pct: 0.0,
            order_rate_per_min: 0,
            message_to_fill_ratio: 0.0,
            allowed_instruments: vec!["SPY".to_string()],
            leverage_after: 1.0,
            cooldown_until_unix_ms: None,
            governor_available: true,
        }
    }
}

impl Default for MarketState {
    fn default() -> Self {
        Self {
            event_window_active: false,
            realized_vol: 0.01,
            median_vol: 0.01,
            data_age_seconds: 0,
            broker_healthy: true,
            broker_last_seen_seconds: 0,
        }
    }
}

impl RiskGovernor {
    pub fn new(policy: RiskPolicy) -> Self {
        Self {
            policy,
            audit: None,
            state_store: None,
        }
    }

    pub fn with_audit(mut self, audit: AuditLedger) -> Self {
        self.audit = Some(audit);
        self
    }

    pub fn with_state_store(mut self, state_store: RiskStateStore) -> Self {
        self.state_store = Some(state_store);
        self
    }

    pub fn decide(
        &self,
        intent: &OrderIntent,
        mode: TradingMode,
        account: &AccountState,
        market: &MarketState,
    ) -> Result<RiskDecision, RiskGovernorError> {
        let started = now_ms();
        let now = started as i64;
        let violation = CONTROL_ORDER.iter().copied().find(|control| {
            !evaluate_control(*control, intent, mode, account, market, &self.policy, now)
        });
        let decision = violation.map_or_else(
            || RiskDecision::approved(elapsed(started)),
            |control| RiskDecision::blocked(control, elapsed(started)),
        );
        self.audit_decision(intent, mode, account, market, &decision)?;
        self.persist_halt_state(intent, account, &decision)?;
        Ok(RiskDecision {
            latency_ms: elapsed(started),
            ..decision
        })
    }

    fn audit_decision(
        &self,
        intent: &OrderIntent,
        mode: TradingMode,
        account: &AccountState,
        market: &MarketState,
        decision: &RiskDecision,
    ) -> Result<(), RiskGovernorError> {
        if let Some(ledger) = &self.audit {
            ledger
                .log_before_act(NewLedgerRecord {
                    actor: "risk-governor".to_string(),
                    strategy_id: intent.strategy_id.clone(),
                    policy_version: self.policy.version_hash.clone(),
                    status: OrderStatus::Requested,
                    risk_decision: json!(decision),
                    order_intent: json!({"intent": intent, "mode": mode}),
                    broker_response: json!({"not_submitted": true}),
                    account: json!({"account": account, "market": market}),
                    tax: TaxFields::default(),
                    artefacts: vec![],
                    maker_checker: None,
                })
                .map_err(|err| RiskGovernorError::Audit(err.code().to_string()))?;
        }
        Ok(())
    }

    fn persist_halt_state(
        &self,
        intent: &OrderIntent,
        account: &AccountState,
        decision: &RiskDecision,
    ) -> Result<(), RiskGovernorError> {
        if let Some(store) = &self.state_store {
            if decision.status != RiskDecisionStatus::Approved {
                store
                    .persist_state(&RiskRuntimeState {
                        strategy_id: intent.strategy_id.clone(),
                        consecutive_losses: 0,
                        daily_loss_cents: (account.daily_loss_pct * 100.0) as i64,
                        cooldown_until_unix_ms: account.cooldown_until_unix_ms,
                        restart_auto_halt: true,
                    })
                    .map_err(|err| RiskGovernorError::State(format!("{err:?}")))?;
            }
        }
        Ok(())
    }
}

impl RiskDecision {
    fn approved(latency_ms: u128) -> Self {
        Self {
            status: RiskDecisionStatus::Approved,
            control_id: None,
            attribution: None,
            terminal: false,
            recoverable: false,
            latency_ms,
        }
    }

    fn blocked(control: ControlId, latency_ms: u128) -> Self {
        Self {
            status: status_for(control),
            control_id: Some(control.as_str()),
            attribution: Some(control.attribution()),
            terminal: control.terminal(),
            recoverable: control.recoverable(),
            latency_ms,
        }
    }
}

impl Default for TaxFields {
    fn default() -> Self {
        Self {
            jurisdiction: "N/A".to_string(),
            account_type: "N/A".to_string(),
            tax_lot_method: "N/A".to_string(),
            wash_sale_relevant: false,
        }
    }
}

fn status_for(control: ControlId) -> RiskDecisionStatus {
    if control.terminal() {
        RiskDecisionStatus::Retired
    } else if matches!(
        control.attribution(),
        HaltAttribution::StrategyAttributable | HaltAttribution::MarketOrInfrastructure
    ) {
        RiskDecisionStatus::Halted
    } else {
        RiskDecisionStatus::Rejected
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn elapsed(started_ms: u128) -> u128 {
    now_ms().saturating_sub(started_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn governor() -> RiskGovernor {
        RiskGovernor::new(RiskPolicy::default())
    }

    fn reject_for(account: AccountState, market: MarketState) -> RiskDecision {
        governor()
            .decide(
                &OrderIntent::default(),
                TradingMode::Paper,
                &account,
                &market,
            )
            .unwrap()
    }

    #[test]
    fn ac_013_emits_control_id_for_reject_and_gates_paper_live() {
        let mut account = AccountState::default();
        account.allowed_instruments.clear();
        let paper = reject_for(account.clone(), MarketState::default());
        let live = governor()
            .decide(
                &OrderIntent::default(),
                TradingMode::LivePilot,
                &account,
                &MarketState::default(),
            )
            .unwrap();
        assert_eq!(paper.control_id, Some("REQ-RISK-010"));
        assert_eq!(live.control_id, Some("REQ-RISK-010"));
    }

    #[test]
    fn ec_14_to_17_loss_controls_are_distinct() {
        let mut account = AccountState {
            daily_loss_pct: 2.0,
            ..Default::default()
        };
        assert_eq!(
            reject_for(account.clone(), MarketState::default()).control_id,
            Some("REQ-RISK-004")
        );
        account.daily_loss_pct = 0.0;
        account.weekly_loss_pct = 5.0;
        assert_eq!(
            reject_for(account.clone(), MarketState::default()).control_id,
            Some("REQ-RISK-005")
        );
        account.weekly_loss_pct = 0.0;
        account.strategy_drawdown_pct = 10.0;
        let retired = reject_for(account.clone(), MarketState::default());
        assert_eq!(retired.control_id, Some("REQ-RISK-006"));
        assert!(retired.terminal);
        account.strategy_drawdown_pct = 0.0;
        account.pilot_drawdown_pct = 10.0;
        let recoverable = governor()
            .decide(
                &OrderIntent::default(),
                TradingMode::LivePilot,
                &account,
                &MarketState::default(),
            )
            .unwrap();
        assert_eq!(recoverable.control_id, Some("REQ-RISK-006a"));
        assert!(recoverable.recoverable);
    }

    #[test]
    fn ec_18_to_21_market_and_cooldown_halts() {
        let mut market = MarketState {
            realized_vol: 0.031,
            median_vol: 0.01,
            ..Default::default()
        };
        assert_eq!(
            reject_for(AccountState::default(), market.clone()).control_id,
            Some("REQ-RISK-012")
        );
        market.realized_vol = 0.01;
        market.data_age_seconds = 4;
        assert_eq!(
            reject_for(AccountState::default(), market.clone()).control_id,
            Some("REQ-RISK-013")
        );
        market.data_age_seconds = 0;
        market.broker_healthy = false;
        assert_eq!(
            reject_for(AccountState::default(), market).control_id,
            Some("REQ-RISK-014")
        );
        let account = AccountState {
            cooldown_until_unix_ms: Some(i64::MAX),
            ..Default::default()
        };
        assert_eq!(
            reject_for(account, MarketState::default()).control_id,
            Some("REQ-RISK-015")
        );
    }

    #[test]
    fn ec_23_27_28_29_fail_closed_rate_event_instrument() {
        let mut account = AccountState {
            governor_available: false,
            ..Default::default()
        };
        assert_eq!(
            reject_for(account.clone(), MarketState::default()).control_id,
            Some("REQ-FAIL-004")
        );
        account.governor_available = true;
        account.message_to_fill_ratio = 51.0;
        assert_eq!(
            reject_for(account.clone(), MarketState::default()).control_id,
            Some("REQ-RISK-009")
        );
        let market = MarketState {
            event_window_active: true,
            ..Default::default()
        };
        assert_eq!(
            reject_for(AccountState::default(), market).control_id,
            Some("REQ-RISK-011")
        );
        account.message_to_fill_ratio = 0.0;
        account.allowed_instruments = vec!["QQQ".to_string()];
        assert_eq!(
            reject_for(account, MarketState::default()).control_id,
            Some("REQ-RISK-010")
        );
    }

    #[test]
    fn no_bypass_rejects_missing_governor_requirement() {
        let intent = OrderIntent {
            governor_required: false,
            ..Default::default()
        };
        let decision = governor()
            .decide(
                &intent,
                TradingMode::Paper,
                &AccountState::default(),
                &MarketState::default(),
            )
            .unwrap();
        assert_eq!(decision.control_id, Some("REQ-RISK-017"));
    }

    #[test]
    #[serial]
    fn ec_34_restart_state_restore_auto_halt_and_audit_before_act() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = AuditLedger::open(dir.path().join("risk-audit.jsonl")).unwrap();
        let store = RiskStateStore::open(dir.path().join("risk-state.db")).unwrap();
        let account = AccountState {
            daily_loss_pct: 2.0,
            ..Default::default()
        };
        let decision = RiskGovernor::new(RiskPolicy::default())
            .with_audit(ledger.clone())
            .with_state_store(store)
            .decide(
                &OrderIntent::default(),
                TradingMode::Paper,
                &account,
                &MarketState::default(),
            )
            .unwrap();
        assert_eq!(decision.control_id, Some("REQ-RISK-004"));
        assert_eq!(ledger.records().unwrap().len(), 1);
        let restored = RiskStateStore::open(dir.path().join("risk-state.db"))
            .unwrap()
            .restore_state("strategy-a")
            .unwrap()
            .unwrap();
        assert!(restored.restart_auto_halt);
    }
}
