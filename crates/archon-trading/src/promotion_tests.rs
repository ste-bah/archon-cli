use super::*;
use crate::adapters::openbb_allowlist::{DataType, LicenseTier, Provider, ProviderClass};
use crate::postmortem::{SessionMode, TradeSummary};
use crate::spec_registry::{
    BenchmarkRef, CostModel, DatasetRef, FormulaSet, Instrument, PositionSizing, RuleSet,
    SpecF08Stops, TimeSession,
};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn ac006_requires_persisted_oos_and_walk_forward_and_excludes_exploratory() {
    let spec = full_spec(PromotionStatus::Research);
    let evidence = vec![
        ok_evidence(BacktestEvidenceKind::OutOfSample),
        exploratory_wf(),
    ];
    let error =
        evaluate_promotion(&spec, PromotionStatus::Backtest, &evidence, None, None).unwrap_err();
    assert_eq!(
        error,
        PromotionError::MissingConditions(vec!["walk_forward_required"])
    );
    let report = evaluate_promotion(&spec, PromotionStatus::Backtest, &both(), None, None).unwrap();
    assert_eq!(report.evidence_summary.accepted_oos, 1);
    assert_eq!(report.evidence_summary.accepted_walk_forward, 1);
}

#[test]
fn research_only_and_strategy_tester_evidence_never_count() {
    let spec = full_spec(PromotionStatus::Backtest);
    let evidence = vec![
        research_only(),
        tester_evidence(BacktestEvidenceKind::WalkForward),
    ];
    let error =
        evaluate_promotion(&spec, PromotionStatus::Paper, &evidence, None, None).unwrap_err();
    assert_eq!(
        error,
        PromotionError::MissingConditions(vec!["oos_required", "walk_forward_required"])
    );
}

#[test]
fn degraded_dataset_evidence_never_counts_for_promotion() {
    let spec = full_spec(PromotionStatus::Research);
    let evidence = vec![
        degraded_evidence(BacktestEvidenceKind::OutOfSample),
        ok_evidence(BacktestEvidenceKind::WalkForward),
    ];

    let error =
        evaluate_promotion(&spec, PromotionStatus::Backtest, &evidence, None, None).unwrap_err();

    assert_eq!(
        error,
        PromotionError::MissingConditions(vec!["oos_required"])
    );
    assert_eq!(summarize_evidence(&evidence).excluded_degraded, 1);
}

#[test]
fn one_step_advance_is_enforced() {
    let spec = full_spec(PromotionStatus::Research);
    let error = evaluate_promotion(&spec, PromotionStatus::Paper, &[], None, None).unwrap_err();
    assert_eq!(error, PromotionError::Spec(SpecRegistryError::StatusSkip));
}

#[test]
fn paper_to_live_pilot_requires_sample_and_postmortem() {
    let spec = full_spec(PromotionStatus::Paper);
    let sample = PaperSample {
        closed_trades: 200,
        calendar_days: 60,
        regime_ids: BTreeSet::from([1, 2]),
        postmortem_ready: true,
    };
    let (advanced, report) = promote_spec(
        &spec,
        PromotionStatus::LivePilot,
        &both(),
        Some(&sample),
        Some(&postmortem()),
    )
    .unwrap();
    assert_eq!(
        advanced.spec_f15_promotion_status,
        Some(PromotionStatus::LivePilot)
    );
    assert_eq!(report.from, PromotionStatus::Paper);
}

fn both() -> Vec<PromotionEvidence> {
    vec![
        ok_evidence(BacktestEvidenceKind::OutOfSample),
        ok_evidence(BacktestEvidenceKind::WalkForward),
    ]
}

fn ok_evidence(kind: BacktestEvidenceKind) -> PromotionEvidence {
    provider_evidence(kind, true, true, false)
}

fn exploratory_wf() -> PromotionEvidence {
    provider_evidence(BacktestEvidenceKind::WalkForward, true, true, true)
}

fn degraded_evidence(kind: BacktestEvidenceKind) -> PromotionEvidence {
    let mut evidence = ok_evidence(kind);
    evidence.dataset_degraded = true;
    evidence
}

fn research_only() -> PromotionEvidence {
    provider_evidence(BacktestEvidenceKind::OutOfSample, true, false, false)
}

fn provider_evidence(
    kind: BacktestEvidenceKind,
    persisted: bool,
    eligible: bool,
    exploratory: bool,
) -> PromotionEvidence {
    PromotionEvidence {
        evidence_id: format!("{kind:?}"),
        strategy_id: "s1".into(),
        kind,
        persisted,
        exploratory,
        source: EvidenceSource::ApprovedData(EvidenceProviderFlag {
            provider: Provider::Polygon,
            data_type: DataType::Ohlcv,
            license_tier: if eligible {
                LicenseTier::Licensed
            } else {
                LicenseTier::ResearchOnly
            },
            provider_class: if eligible {
                ProviderClass::PaidLicensed
            } else {
                ProviderClass::Unofficial
            },
            promotion_eligible: eligible,
        }),
        dataset_degraded: false,
        dataset_checksum: "dataset".into(),
        config_hash: "config".into(),
    }
}

fn tester_evidence(kind: BacktestEvidenceKind) -> PromotionEvidence {
    PromotionEvidence {
        evidence_id: "tester".into(),
        strategy_id: "s1".into(),
        kind,
        persisted: true,
        exploratory: false,
        source: EvidenceSource::StrategyTesterAuxiliary,
        dataset_degraded: false,
        dataset_checksum: "dataset".into(),
        config_hash: "config".into(),
    }
}

fn postmortem() -> SessionPostmortem {
    SessionPostmortem {
        session_id: "session".into(),
        mode: SessionMode::Paper,
        strategy_ids: vec!["s1".into()],
        trades: vec![TradeSummary {
            trade_id: "t1".into(),
            instrument: "SPY".into(),
            quantity: 1.0,
            realized_pnl: 1.0,
        }],
        realized_pnl: 1.0,
        risk_events: vec![],
        spec_f13_deviations: vec![],
        lessons: vec!["keep".into()],
        session_closed_unix_ms: 1_000,
        completed_unix_ms: 2_000,
    }
}

fn full_spec(status: PromotionStatus) -> StrategySpec {
    StrategySpec {
        spec_f01_instrument_universe: Some(vec![Instrument {
            symbol: "SPY".into(),
            venue: "ARCA".into(),
            asset_class: "equity".into(),
        }]),
        spec_f02_timeframe_session: Some(TimeSession {
            timeframe: "1D".into(),
            session_hours: "regular".into(),
        }),
        spec_f03_market_regime_assumptions: Some(vec!["baseline".into()]),
        spec_f04_data_dependencies: Some(vec![DatasetRef {
            dataset_id: "ohlcv".into(),
            version: "v1".into(),
        }]),
        spec_f05_entry_exit_rules: Some(RuleSet {
            rules: vec!["close > sma20".into()],
        }),
        spec_f06_indicator_formulas: Some(FormulaSet {
            formulas: vec!["sma(close,20)".into()],
        }),
        spec_f07_position_sizing: Some(PositionSizing {
            model: "fixed_fractional".into(),
            max_risk_pct: "1".into(),
        }),
        spec_f08_stops: Some(SpecF08Stops {
            stop_rules: vec!["2atr".into()],
            take_profit_rules: vec!["3atr".into()],
            trailing_rules: vec![],
            max_strategy_drawdown_pct: 8.0,
        }),
        spec_f09_invalidation_rules: Some(RuleSet {
            rules: vec!["vol regime shift".into()],
        }),
        spec_f10_no_trade_conditions: Some(RuleSet {
            rules: vec!["event".into()],
        }),
        spec_f11_cost_assumptions: Some(CostModel {
            slippage_bps: 2,
            fee_bps: 1,
        }),
        spec_f12_benchmark: Some(BenchmarkRef {
            symbol: "SPY".into(),
            source: "approved".into(),
        }),
        spec_f13_expected_failure_modes: Some(vec!["gap".into()]),
        spec_f14_data_quality_tolerances_ms: Some(BTreeMap::from([("ohlcv".into(), 5000)])),
        spec_f15_promotion_status: Some(status),
    }
}
