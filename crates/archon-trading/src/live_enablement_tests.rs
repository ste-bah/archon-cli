use super::*;
use crate::dryrun_cert::CertificationCheck;
use crate::risk_policy::RiskPolicy;
use crate::spec_registry::{
    BenchmarkRef, CostModel, DatasetRef, FormulaSet, Instrument, PositionSizing, PromotionStatus,
    RuleSet, SpecF08Stops, TimeSession,
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
    assert!(
        valid_enablement_request()
            .evaluate()
            .expect("approved")
            .approved
    );
}

#[test]
fn enablement_rejects_tampered_policy_and_failed_certification() {
    let mut request = valid_enablement_request();
    request.policy.version_hash = "stale".to_string();
    assert_eq!(
        request.evaluate(),
        Err(LiveEnablementError::MissingValidation("risk_policy_hash"))
    );

    let mut failed_cert = passing_certification();
    failed_cert.passed = false;
    request = valid_enablement_request();
    request.certification_report = Some(failed_cert);
    assert_eq!(
        request.evaluate(),
        Err(LiveEnablementError::CertificationFailed)
    );
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

fn valid_enablement_request() -> LiveEnablementRequest {
    LiveEnablementRequest {
        strategy_id: "s1".to_string(),
        account_id: "acct".to_string(),
        broker_id: "broker".to_string(),
        kill_switch_validated: true,
        policy: RiskPolicy::default(),
        production_evidence: Some(approved_evidence()),
        certification_report: Some(passing_certification()),
        approval: Some(MakerCheckerApproval::new(
            "r1",
            "alice",
            "bob",
            "enable-live",
            true,
            "ok",
        )),
        compliance_jurisdiction: Some("US".to_string()),
    }
}

fn approved_evidence() -> ProductionEvidence {
    ProductionEvidence {
        backtest_approved: true,
        paper_approved: true,
        risk_approved: true,
        postmortem_approved: true,
    }
}

fn passing_certification() -> CertificationReport {
    CertificationReport {
        adapter_name: "dry-run".to_string(),
        passed: true,
        checks: vec![CertificationCheck {
            id: "REQ-TERM-006:manifest".to_string(),
            passed: true,
        }],
        pre_trade_p99_ms: 1,
    }
}

fn valid_spec() -> StrategySpec {
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
        spec_f14_data_quality_tolerances_ms: Some(BTreeMap::from([("quotes".to_string(), 1_000)])),
        spec_f15_promotion_status: Some(PromotionStatus::LivePilot),
    }
}
