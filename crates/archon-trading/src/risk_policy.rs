use crate::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use crate::maker_checker::MakerCheckerApproval;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskPolicy {
    pub version_hash: String,
    pub thresholds: RiskThresholds,
    pub promotion: PromotionPolicy,
    pub capital: CapitalPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskThresholds {
    pub max_order_notional_pct: f64,
    pub max_strategy_exposure_pct: f64,
    pub max_account_exposure_pct: f64,
    pub max_symbol_concentration_pct: f64,
    pub max_daily_loss_pct: f64,
    pub max_strategy_drawdown_pct: f64,
    pub pilot_max_drawdown_pct: f64,
    pub max_open_orders: u32,
    pub max_order_rate_per_min: u32,
    pub min_liquidity_ratio: f64,
    pub max_spread_bps: f64,
    pub max_slippage_bps: f64,
    pub stale_data_max_seconds: u64,
    pub broker_health_timeout_seconds: u64,
    pub kill_switch_cancel_seconds: u64,
    pub max_leverage: f64,
    pub halt_after_consecutive_losses: u32,
    pub cooldown_minutes_after_halt: u32,
    pub max_correlated_exposure_pct: f64,
    pub manual_approval_notional_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionPolicy {
    pub paper_min_closed_trades: u32,
    pub paper_min_calendar_days: u32,
    pub paper_min_regimes: u32,
    pub phase5_min_months: u32,
    pub phase5_min_zero_halt_sessions: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapitalPolicy {
    pub paper_sim_capital: f64,
    pub pilot_capital_max_usd: f64,
    pub pilot_capital_max_equity_pct: f64,
    pub per_order_manual_approval_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskRuntimeState {
    pub strategy_id: String,
    pub consecutive_losses: u32,
    pub daily_loss_cents: i64,
    pub cooldown_until_unix_ms: Option<i64>,
    pub restart_auto_halt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskPolicyError {
    UpwardChangeNeedsMakerChecker,
    AuditRequired,
    MakerChecker(String),
    Audit(String),
    Store(String),
}

impl Default for RiskPolicy {
    fn default() -> Self {
        Self::new(
            RiskThresholds::default(),
            PromotionPolicy::default(),
            CapitalPolicy::default(),
        )
    }
}

impl RiskPolicy {
    pub fn new(
        thresholds: RiskThresholds,
        promotion: PromotionPolicy,
        capital: CapitalPolicy,
    ) -> Self {
        let mut policy = Self {
            version_hash: String::new(),
            thresholds,
            promotion,
            capital,
        };
        policy.version_hash = policy.compute_hash();
        policy
    }

    pub fn compute_hash(&self) -> String {
        let mut copy = self.clone();
        copy.version_hash.clear();
        let bytes = serde_json::to_vec(&copy).unwrap_or_default();
        blake3::hash(&bytes).to_hex().to_string()
    }

    pub fn validate_hash(&self) -> bool {
        self.version_hash == self.compute_hash()
    }

    pub fn apply_change(
        &self,
        next: RiskPolicy,
        approval: Option<&MakerCheckerApproval>,
        audit: Option<&AuditLedger>,
        actor: &str,
    ) -> Result<RiskPolicy, RiskPolicyError> {
        if self.is_upward_change(&next) {
            let approval = approval.ok_or(RiskPolicyError::UpwardChangeNeedsMakerChecker)?;
            approval
                .verify_pair()
                .map_err(|err| RiskPolicyError::MakerChecker(err.to_string()))?;
            let audit = audit.ok_or(RiskPolicyError::AuditRequired)?;
            audit
                .log_before_act(policy_change_record(actor, self, &next, approval.clone()))
                .map_err(|err| RiskPolicyError::Audit(err.to_string()))?;
        }
        let mut updated = next;
        updated.version_hash = updated.compute_hash();
        Ok(updated)
    }

    fn is_upward_change(&self, next: &RiskPolicy) -> bool {
        let current = self.safety_scores();
        let proposed = next.safety_scores();
        proposed
            .iter()
            .zip(current.iter())
            .any(|(new, old)| new > old)
    }

    fn safety_scores(&self) -> Vec<f64> {
        let t = &self.thresholds;
        vec![
            t.max_order_notional_pct,
            t.max_strategy_exposure_pct,
            t.max_account_exposure_pct,
            t.max_symbol_concentration_pct,
            t.max_daily_loss_pct,
            t.max_strategy_drawdown_pct,
            t.pilot_max_drawdown_pct,
            t.max_open_orders as f64,
            t.max_order_rate_per_min as f64,
            inverse(t.min_liquidity_ratio),
            t.max_spread_bps,
            t.max_slippage_bps,
            t.stale_data_max_seconds as f64,
            t.broker_health_timeout_seconds as f64,
            t.kill_switch_cancel_seconds as f64,
            t.max_leverage,
            t.halt_after_consecutive_losses as f64,
            t.cooldown_minutes_after_halt as f64,
            t.max_correlated_exposure_pct,
            t.manual_approval_notional_pct,
            self.capital.pilot_capital_max_usd,
            self.capital.pilot_capital_max_equity_pct,
            self.capital.paper_sim_capital,
        ]
    }
}

impl Default for RiskThresholds {
    fn default() -> Self {
        Self {
            max_order_notional_pct: 2.0,
            max_strategy_exposure_pct: 10.0,
            max_account_exposure_pct: 50.0,
            max_symbol_concentration_pct: 15.0,
            max_daily_loss_pct: 2.0,
            max_strategy_drawdown_pct: 10.0,
            pilot_max_drawdown_pct: 10.0,
            max_open_orders: 20,
            max_order_rate_per_min: 5,
            min_liquidity_ratio: 20.0,
            max_spread_bps: 25.0,
            max_slippage_bps: 50.0,
            stale_data_max_seconds: 3,
            broker_health_timeout_seconds: 3,
            kill_switch_cancel_seconds: 2,
            max_leverage: 1.0,
            halt_after_consecutive_losses: 3,
            cooldown_minutes_after_halt: 60,
            max_correlated_exposure_pct: 25.0,
            manual_approval_notional_pct: 0.0,
        }
    }
}

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            paper_min_closed_trades: 200,
            paper_min_calendar_days: 60,
            paper_min_regimes: 2,
            phase5_min_months: 6,
            phase5_min_zero_halt_sessions: 30,
        }
    }
}

impl Default for CapitalPolicy {
    fn default() -> Self {
        Self {
            paper_sim_capital: 100_000.0,
            pilot_capital_max_usd: 1_000.0,
            pilot_capital_max_equity_pct: 1.0,
            per_order_manual_approval_required: true,
        }
    }
}

pub struct RiskStateStore {
    db: DbInstance,
    guard: archon_cozo::CozoGuardConfig,
}

impl RiskStateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RiskPolicyError> {
        let path_text = path.as_ref().to_string_lossy().to_string();
        let guard = archon_cozo::CozoGuardConfig::for_db_path(path.as_ref());
        let db = archon_cozo::open_sqlite_guarded(&path_text, "open risk policy state", &guard)
            .map_err(|err| RiskPolicyError::Store(err.to_string()))?;
        Ok(Self { db, guard })
    }

    pub fn persist_state(&self, state: &RiskRuntimeState) -> Result<(), RiskPolicyError> {
        self.ensure_relation()?;
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(state.strategy_id.as_str()));
        params.insert(
            "losses".into(),
            DataValue::from(state.consecutive_losses as i64),
        );
        params.insert("daily".into(), DataValue::from(state.daily_loss_cents));
        params.insert(
            "cooldown".into(),
            DataValue::from(state.cooldown_until_unix_ms.unwrap_or(0)),
        );
        params.insert("halt".into(), DataValue::from(state.restart_auto_halt));
        self.run(
            state_put_script(),
            params,
            ScriptMutability::Mutable,
            "persist risk state",
        )?;
        Ok(())
    }

    pub fn restore_state(
        &self,
        strategy_id: &str,
    ) -> Result<Option<RiskRuntimeState>, RiskPolicyError> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(strategy_id));
        let rows = self.run(
            state_get_script(),
            params,
            ScriptMutability::Immutable,
            "restore risk state",
        )?;
        Ok(rows.rows.first().map(|row| row_to_state(row)))
    }

    fn ensure_relation(&self) -> Result<(), RiskPolicyError> {
        let _ = self.run(
            ":create trading_risk_state { strategy_id => consecutive_losses, daily_loss_cents, cooldown_until_unix_ms, restart_auto_halt }",
            BTreeMap::new(),
            ScriptMutability::Mutable,
            "ensure risk state relation",
        );
        Ok(())
    }

    fn run(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
        mutability: ScriptMutability,
        context: &str,
    ) -> Result<cozo::NamedRows, RiskPolicyError> {
        archon_cozo::run_script_guarded(&self.db, script, params, mutability, context, &self.guard)
            .map_err(|err| RiskPolicyError::Store(err.to_string()))
    }
}

fn policy_change_record(
    actor: &str,
    current: &RiskPolicy,
    next: &RiskPolicy,
    approval: MakerCheckerApproval,
) -> NewLedgerRecord {
    NewLedgerRecord {
        actor: actor.to_string(),
        strategy_id: "risk-policy".to_string(),
        policy_version: current.version_hash.clone(),
        status: OrderStatus::Requested,
        risk_decision: json!({"change":"upward-risk-policy", "from": current.version_hash, "to": next.compute_hash()}),
        order_intent: json!({"policy": next}),
        broker_response: json!({"not_applicable": true}),
        account: json!({"scope": "risk-governor-policy"}),
        tax: TaxFields {
            jurisdiction: "N/A".to_string(),
            account_type: "N/A".to_string(),
            tax_lot_method: "N/A".to_string(),
            wash_sale_relevant: false,
        },
        artefacts: vec![serde_json::to_vec(next).unwrap_or_default()],
        maker_checker: Some(approval),
    }
}

fn state_put_script() -> &'static str {
    "?[strategy_id, consecutive_losses, daily_loss_cents, cooldown_until_unix_ms, restart_auto_halt] <- [[$sid, $losses, $daily, $cooldown, $halt]] :put trading_risk_state { strategy_id => consecutive_losses, daily_loss_cents, cooldown_until_unix_ms, restart_auto_halt }"
}

fn state_get_script() -> &'static str {
    "?[strategy_id, consecutive_losses, daily_loss_cents, cooldown_until_unix_ms, restart_auto_halt] := *trading_risk_state{ strategy_id, consecutive_losses, daily_loss_cents, cooldown_until_unix_ms, restart_auto_halt }, strategy_id = $sid"
}

fn row_to_state(row: &[DataValue]) -> RiskRuntimeState {
    let cooldown = as_i64(&row[3]);
    RiskRuntimeState {
        strategy_id: row[0].get_str().unwrap_or_default().to_string(),
        consecutive_losses: as_i64(&row[1]).max(0) as u32,
        daily_loss_cents: as_i64(&row[2]),
        cooldown_until_unix_ms: (cooldown > 0).then_some(cooldown),
        restart_auto_halt: true,
    }
}

fn as_i64(value: &DataValue) -> i64 {
    match value {
        DataValue::Num(cozo::Num::Int(number)) => *number,
        DataValue::Num(cozo::Num::Float(number)) => *number as i64,
        DataValue::Bool(flag) => i64::from(*flag),
        _ => 0,
    }
}

fn inverse(value: f64) -> f64 {
    if value <= 0.0 { f64::MAX } else { 1.0 / value }
}

#[cfg(test)]
#[path = "risk_policy_tests.rs"]
mod tests;
