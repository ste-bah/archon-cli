use crate::maker_checker::MakerCheckerApproval;
use crate::risk_controls::HaltAttribution;
use crate::risk_policy::RiskPolicy;
use crate::spec_registry::StrategySpec;
use serde::{Deserialize, Serialize};

pub const PHASE4_MANUAL_APPROVAL_REQUIRED: bool = true;
const SUPPORTED_JURISDICTIONS: &[&str] = &["US", "UK", "EU", "CA", "AU"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTradingConfig {
    pub enabled: bool,
    pub compliance_jurisdiction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveEnablementRequest {
    pub strategy_id: String,
    pub account_id: String,
    pub broker_id: String,
    pub kill_switch_validated: bool,
    pub policy: RiskPolicy,
    pub approval: Option<MakerCheckerApproval>,
    pub compliance_jurisdiction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PilotPlan {
    pub strategy_id: String,
    pub account_equity: f64,
    pub pilot_capital_usd: f64,
    pub max_daily_loss_pct: f64,
    pub max_pilot_drawdown_pct: f64,
    pub per_order_manual_approval_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionEvidence {
    pub backtest_approved: bool,
    pub paper_approved: bool,
    pub risk_approved: bool,
    pub postmortem_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phase5Evidence {
    pub months_live_pilot: u32,
    pub oos_sharpe_net: f64,
    pub realized_drawdown_pct: f64,
    pub profitable_regime_count: u32,
    pub zero_strategy_attributable_halt_sessions: u32,
    pub ac006_backtest_evidence: bool,
    pub policy_change_logged: bool,
    pub approval: Option<MakerCheckerApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveEnablementDecision {
    pub approved: bool,
    pub code: &'static str,
    pub reasons: Vec<&'static str>,
    pub logged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEnablementError {
    DisabledByDefault,
    UnsupportedJurisdiction,
    MissingValidation(&'static str),
    MakerChecker(String),
    PilotLimit(&'static str),
    ProductionEvidenceMissing(&'static str),
    Phase5Prereq(Vec<&'static str>),
}

impl Default for LiveTradingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            compliance_jurisdiction: None,
        }
    }
}

impl LiveTradingConfig {
    pub fn live_is_enabled_by_default() -> bool {
        Self::default().enabled
    }

    pub fn jurisdiction_supported(&self) -> bool {
        self.compliance_jurisdiction
            .as_deref()
            .is_some_and(is_supported_jurisdiction)
    }
}

impl LiveEnablementRequest {
    pub fn evaluate(&self) -> Result<LiveEnablementDecision, LiveEnablementError> {
        require_supported_jurisdiction(self.compliance_jurisdiction.as_deref())?;
        require_non_empty(&self.strategy_id, "strategy")?;
        require_non_empty(&self.account_id, "account")?;
        require_non_empty(&self.broker_id, "broker")?;
        if !self.kill_switch_validated {
            return Err(LiveEnablementError::MissingValidation("kill_switch"));
        }
        let approval = self
            .approval
            .as_ref()
            .ok_or(LiveEnablementError::MissingValidation("maker_checker"))?;
        approval
            .verify_pair()
            .map_err(|err| LiveEnablementError::MakerChecker(err.to_string()))?;
        Ok(LiveEnablementDecision::approved("LIVE_ENABLEMENT_APPROVED"))
    }
}

impl PilotPlan {
    pub fn new(
        strategy_id: impl Into<String>,
        account_equity: f64,
        requested_capital_usd: f64,
        policy: &RiskPolicy,
    ) -> Result<Self, LiveEnablementError> {
        let max_capital = account_equity * (policy.capital.pilot_capital_max_equity_pct / 100.0);
        let allowed_capital = max_capital
            .min(policy.capital.pilot_capital_max_usd)
            .min(1_000.0);
        if requested_capital_usd > allowed_capital || requested_capital_usd <= 0.0 {
            return Err(LiveEnablementError::PilotLimit("capital"));
        }
        if policy.thresholds.pilot_max_drawdown_pct > 10.0 {
            return Err(LiveEnablementError::PilotLimit("pilot_drawdown"));
        }
        if policy.thresholds.max_daily_loss_pct > 2.0 {
            return Err(LiveEnablementError::PilotLimit("daily_loss"));
        }
        if !policy.capital.per_order_manual_approval_required {
            return Err(LiveEnablementError::PilotLimit("manual_approval_phase4"));
        }
        Ok(Self {
            strategy_id: strategy_id.into(),
            account_equity,
            pilot_capital_usd: requested_capital_usd,
            max_daily_loss_pct: policy.thresholds.max_daily_loss_pct,
            max_pilot_drawdown_pct: policy.thresholds.pilot_max_drawdown_pct,
            per_order_manual_approval_required: PHASE4_MANUAL_APPROVAL_REQUIRED,
        })
    }
}

impl ProductionEvidence {
    pub fn validate(&self) -> Result<(), LiveEnablementError> {
        if !self.backtest_approved {
            return Err(LiveEnablementError::ProductionEvidenceMissing("backtest"));
        }
        if !self.paper_approved {
            return Err(LiveEnablementError::ProductionEvidenceMissing("paper"));
        }
        if !self.risk_approved {
            return Err(LiveEnablementError::ProductionEvidenceMissing("risk"));
        }
        if !self.postmortem_approved {
            return Err(LiveEnablementError::ProductionEvidenceMissing("postmortem"));
        }
        Ok(())
    }
}

impl Phase5Evidence {
    pub fn evaluate(
        &self,
        spec: &StrategySpec,
        policy: &RiskPolicy,
    ) -> Result<LiveEnablementDecision, LiveEnablementError> {
        let mut missing = self.missing_prereqs(spec, policy);
        match self.approval.as_ref() {
            Some(approval) => {
                if let Err(err) = approval.verify_pair() {
                    return Err(LiveEnablementError::MakerChecker(err.to_string()));
                }
            }
            None => missing.push("maker_checker"),
        }
        if missing.is_empty() {
            Ok(LiveEnablementDecision::approved("PHASE5_AUTONOMY_APPROVED"))
        } else {
            Err(LiveEnablementError::Phase5Prereq(missing))
        }
    }

    pub fn blocked_decision(
        &self,
        spec: &StrategySpec,
        policy: &RiskPolicy,
    ) -> LiveEnablementDecision {
        let mut reasons = self.missing_prereqs(spec, policy);
        if self.approval.is_none() {
            reasons.push("maker_checker");
        }
        LiveEnablementDecision {
            approved: false,
            code: "ERR-LIVE-PHASE5-PREREQ",
            reasons,
            logged: true,
        }
    }

    fn missing_prereqs(&self, spec: &StrategySpec, policy: &RiskPolicy) -> Vec<&'static str> {
        let mut missing = Vec::new();
        push_if(
            self.months_live_pilot < policy.promotion.phase5_min_months,
            "six_months",
            &mut missing,
        );
        push_if(self.oos_sharpe_net < 1.0, "oos_sharpe", &mut missing);
        push_if(
            !drawdown_within_spec(self.realized_drawdown_pct, spec),
            "drawdown",
            &mut missing,
        );
        push_if(
            self.profitable_regime_count < 2,
            "profitable_regimes",
            &mut missing,
        );
        push_if(
            self.zero_strategy_attributable_halt_sessions
                < policy.promotion.phase5_min_zero_halt_sessions,
            "zero_strategy_attributable_halts",
            &mut missing,
        );
        push_if(!self.ac006_backtest_evidence, "ac006", &mut missing);
        push_if(
            !self.policy_change_logged,
            "policy_change_logged",
            &mut missing,
        );
        missing
    }
}

impl LiveEnablementDecision {
    fn approved(code: &'static str) -> Self {
        Self {
            approved: true,
            code,
            reasons: Vec::new(),
            logged: true,
        }
    }
}

pub fn live_default_decision() -> LiveEnablementDecision {
    LiveEnablementDecision {
        approved: false,
        code: "LIVE_DISABLED_BY_DEFAULT",
        reasons: vec!["trading_enabled_false"],
        logged: true,
    }
}

pub fn live_fail_closes_for_jurisdiction(jurisdiction: Option<&str>) -> bool {
    require_supported_jurisdiction(jurisdiction).is_err()
}

pub fn reset_zero_halt_counter(attribution: HaltAttribution) -> bool {
    matches!(attribution, HaltAttribution::StrategyAttributable)
}

fn require_supported_jurisdiction(jurisdiction: Option<&str>) -> Result<(), LiveEnablementError> {
    if jurisdiction.is_some_and(is_supported_jurisdiction) {
        Ok(())
    } else {
        Err(LiveEnablementError::UnsupportedJurisdiction)
    }
}

fn is_supported_jurisdiction(value: &str) -> bool {
    SUPPORTED_JURISDICTIONS.contains(&value.trim().to_uppercase().as_str())
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), LiveEnablementError> {
    if value.trim().is_empty() {
        Err(LiveEnablementError::MissingValidation(field))
    } else {
        Ok(())
    }
}

fn drawdown_within_spec(realized: f64, spec: &StrategySpec) -> bool {
    spec.spec_f08_stops
        .as_ref()
        .is_some_and(|stops| realized <= stops.max_strategy_drawdown_pct)
}

fn push_if(condition: bool, reason: &'static str, missing: &mut Vec<&'static str>) {
    if condition {
        missing.push(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk_policy::RiskPolicy;
    use crate::spec_registry::{
        BenchmarkRef, CostModel, DatasetRef, FormulaSet, Instrument, PositionSizing,
        PromotionStatus, RuleSet, SpecF08Stops, TimeSession,
    };
    use std::collections::BTreeMap;

    #[test]
    fn live_is_disabled_by_default_ac009() {
        assert!(!LiveTradingConfig::live_is_enabled_by_default());
        let decision = live_default_decision();
        assert!(!decision.approved);
        assert_eq!(decision.code, "LIVE_DISABLED_BY_DEFAULT");
        assert!(decision.logged);
    }

    #[test]
    fn enablement_requires_maker_checker_and_valid_controls_ac020() {
        let request = LiveEnablementRequest {
            strategy_id: "s1".to_string(),
            account_id: "acct".to_string(),
            broker_id: "broker".to_string(),
            kill_switch_validated: true,
            policy: RiskPolicy::default(),
            approval: Some(MakerCheckerApproval::new(
                "r1",
                "alice",
                "bob",
                "enable-live",
                true,
                "ok",
            )),
            compliance_jurisdiction: Some("US".to_string()),
        };
        assert!(request.evaluate().expect("approved").approved);
    }

    #[test]
    fn pilot_is_bounded_and_manual_approval_cannot_be_lifted() {
        let mut policy = RiskPolicy::default();
        policy.capital.per_order_manual_approval_required = false;
        assert!(matches!(
            PilotPlan::new("s1", 100_000.0, 1_000.0, &policy),
            Err(LiveEnablementError::PilotLimit("manual_approval_phase4"))
        ));
        policy.capital.per_order_manual_approval_required = true;
        assert!(PilotPlan::new("s1", 100_000.0, 1_000.0, &policy).is_ok());
        assert!(matches!(
            PilotPlan::new("s1", 100_000.0, 1_000.01, &policy),
            Err(LiveEnablementError::PilotLimit("capital"))
        ));
    }

    #[test]
    fn phase5_without_all_prereqs_is_blocked_and_logged_ec_trl_31() {
        let evidence = Phase5Evidence {
            months_live_pilot: 5,
            oos_sharpe_net: 0.9,
            realized_drawdown_pct: 9.0,
            profitable_regime_count: 1,
            zero_strategy_attributable_halt_sessions: 29,
            ac006_backtest_evidence: false,
            policy_change_logged: false,
            approval: None,
        };
        let decision = evidence.blocked_decision(&valid_spec(), &RiskPolicy::default());
        assert!(!decision.approved);
        assert_eq!(decision.code, "ERR-LIVE-PHASE5-PREREQ");
        assert!(decision.logged);
        assert!(decision.reasons.contains(&"six_months"));
        assert!(decision.reasons.contains(&"maker_checker"));
    }

    #[test]
    fn phase5_approves_only_with_all_prereqs_ac031() {
        let evidence = Phase5Evidence {
            months_live_pilot: 6,
            oos_sharpe_net: 1.0,
            realized_drawdown_pct: 9.0,
            profitable_regime_count: 2,
            zero_strategy_attributable_halt_sessions: 30,
            ac006_backtest_evidence: true,
            policy_change_logged: true,
            approval: Some(MakerCheckerApproval::new(
                "r2", "alice", "bob", "phase5", true, "ok",
            )),
        };
        assert!(
            evidence
                .evaluate(&valid_spec(), &RiskPolicy::default())
                .expect("phase5")
                .approved
        );
    }

    #[test]
    fn unsupported_or_unset_jurisdiction_fail_closes_ec_trl_32() {
        assert!(live_fail_closes_for_jurisdiction(None));
        assert!(live_fail_closes_for_jurisdiction(Some("moon")));
        assert!(!live_fail_closes_for_jurisdiction(Some("us")));
    }

    fn valid_spec() -> StrategySpec {
        let mut tolerances = BTreeMap::new();
        tolerances.insert("quotes".to_string(), 1_000);
        StrategySpec {
            spec_f01_instrument_universe: Some(vec![Instrument {
                symbol: "SPY".to_string(),
                venue: "ARCX".to_string(),
                asset_class: "equity".to_string(),
            }]),
            spec_f02_timeframe_session: Some(TimeSession {
                timeframe: "1D".to_string(),
                session_hours: "regular".to_string(),
            }),
            spec_f03_market_regime_assumptions: Some(vec!["normal".to_string()]),
            spec_f04_data_dependencies: Some(vec![DatasetRef {
                dataset_id: "d1".to_string(),
                version: "v1".to_string(),
            }]),
            spec_f05_entry_exit_rules: Some(RuleSet {
                rules: vec!["enter".to_string()],
            }),
            spec_f06_indicator_formulas: Some(FormulaSet {
                formulas: vec!["sma".to_string()],
            }),
            spec_f07_position_sizing: Some(PositionSizing {
                model: "fixed".to_string(),
                max_risk_pct: "1".to_string(),
            }),
            spec_f08_stops: Some(SpecF08Stops {
                stop_rules: vec!["stop".to_string()],
                take_profit_rules: vec!["tp".to_string()],
                trailing_rules: vec![],
                max_strategy_drawdown_pct: 10.0,
            }),
            spec_f09_invalidation_rules: Some(RuleSet {
                rules: vec!["invalid".to_string()],
            }),
            spec_f10_no_trade_conditions: Some(RuleSet {
                rules: vec!["none".to_string()],
            }),
            spec_f11_cost_assumptions: Some(CostModel {
                slippage_bps: 1,
                fee_bps: 1,
            }),
            spec_f12_benchmark: Some(BenchmarkRef {
                symbol: "SPY".to_string(),
                source: "public".to_string(),
            }),
            spec_f13_expected_failure_modes: Some(vec!["chop".to_string()]),
            spec_f14_data_quality_tolerances_ms: Some(tolerances),
            spec_f15_promotion_status: Some(PromotionStatus::LivePilot),
        }
    }
}
