use crate::adapters::openbb_allowlist::EvidenceProviderFlag;
use crate::paper_terminal::PaperSample;
use crate::postmortem::{PostmortemError, SessionPostmortem, require_postmortem_for_promotion};
use crate::spec_registry::{PromotionStatus, SpecRegistryError, StrategySpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BacktestEvidenceKind {
    OutOfSample,
    WalkForward,
    MonteCarlo,
    RegimeSlice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSource {
    ApprovedData(EvidenceProviderFlag),
    InternalReplay,
    StrategyTesterAuxiliary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionEvidence {
    pub evidence_id: String,
    pub strategy_id: String,
    pub kind: BacktestEvidenceKind,
    pub persisted: bool,
    pub exploratory: bool,
    pub source: EvidenceSource,
    pub dataset_checksum: String,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub accepted_oos: usize,
    pub accepted_walk_forward: usize,
    pub excluded_exploratory: usize,
    pub excluded_research_only: usize,
    pub excluded_strategy_tester: usize,
    pub excluded_unpersisted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromotionReport {
    pub from: PromotionStatus,
    pub to: PromotionStatus,
    pub advanced: bool,
    pub evidence_summary: EvidenceSummary,
    pub missing_conditions: Vec<&'static str>,
    pub new_spec_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionError {
    Spec(SpecRegistryError),
    MissingConditions(Vec<&'static str>),
    Postmortem(PostmortemError),
}

impl PromotionEvidence {
    pub fn counts_for_promotion(&self) -> bool {
        self.persisted
            && !self.exploratory
            && !matches!(self.source, EvidenceSource::StrategyTesterAuxiliary)
            && self.source_is_promotion_eligible()
    }

    fn source_is_promotion_eligible(&self) -> bool {
        match &self.source {
            EvidenceSource::ApprovedData(flag) => flag.promotion_eligible,
            EvidenceSource::InternalReplay => true,
            EvidenceSource::StrategyTesterAuxiliary => false,
        }
    }
}

pub fn evaluate_promotion(
    spec: &StrategySpec,
    target: PromotionStatus,
    evidence: &[PromotionEvidence],
    paper_sample: Option<&PaperSample>,
    postmortem: Option<&SessionPostmortem>,
) -> Result<PromotionReport, PromotionError> {
    let current = current_status(spec)?;
    spec.advance_status(target).map_err(PromotionError::Spec)?;
    let evidence_summary = summarize_evidence(evidence);
    let mut missing_conditions = missing_backtest_conditions(current, &evidence_summary);
    if current == PromotionStatus::Paper && target == PromotionStatus::LivePilot {
        missing_conditions.extend(missing_paper_conditions(paper_sample));
        require_postmortem_for_promotion(postmortem).map_err(PromotionError::Postmortem)?;
    }
    if !missing_conditions.is_empty() {
        return Err(PromotionError::MissingConditions(missing_conditions));
    }
    let advanced = spec.advance_status(target).map_err(PromotionError::Spec)?;
    Ok(PromotionReport {
        from: current,
        to: target,
        advanced: true,
        evidence_summary,
        missing_conditions,
        new_spec_hash: advanced.content_hash().ok(),
    })
}

pub fn promote_spec(
    spec: &StrategySpec,
    target: PromotionStatus,
    evidence: &[PromotionEvidence],
    paper_sample: Option<&PaperSample>,
    postmortem: Option<&SessionPostmortem>,
) -> Result<(StrategySpec, PromotionReport), PromotionError> {
    let report = evaluate_promotion(spec, target, evidence, paper_sample, postmortem)?;
    let advanced = spec.advance_status(target).map_err(PromotionError::Spec)?;
    Ok((advanced, report))
}

pub fn summarize_evidence(evidence: &[PromotionEvidence]) -> EvidenceSummary {
    let mut summary = EvidenceSummary::default();
    for item in evidence {
        apply_exclusions(item, &mut summary);
        if item.counts_for_promotion() {
            match item.kind {
                BacktestEvidenceKind::OutOfSample => summary.accepted_oos += 1,
                BacktestEvidenceKind::WalkForward => summary.accepted_walk_forward += 1,
                BacktestEvidenceKind::MonteCarlo | BacktestEvidenceKind::RegimeSlice => {}
            }
        }
    }
    summary
}

impl Default for EvidenceSummary {
    fn default() -> Self {
        Self {
            accepted_oos: 0,
            accepted_walk_forward: 0,
            excluded_exploratory: 0,
            excluded_research_only: 0,
            excluded_strategy_tester: 0,
            excluded_unpersisted: 0,
        }
    }
}

fn apply_exclusions(item: &PromotionEvidence, summary: &mut EvidenceSummary) {
    if item.exploratory {
        summary.excluded_exploratory += 1;
    }
    if !item.persisted {
        summary.excluded_unpersisted += 1;
    }
    if matches!(item.source, EvidenceSource::StrategyTesterAuxiliary) {
        summary.excluded_strategy_tester += 1;
    }
    if !item.source_is_promotion_eligible() {
        summary.excluded_research_only += 1;
    }
}

fn current_status(spec: &StrategySpec) -> Result<PromotionStatus, PromotionError> {
    spec.validated().map_err(PromotionError::Spec)?;
    spec.spec_f15_promotion_status.ok_or_else(|| {
        PromotionError::Spec(SpecRegistryError::MissingOrInvalidFields(vec!["SPEC-F15"]))
    })
}

fn missing_backtest_conditions(
    current: PromotionStatus,
    summary: &EvidenceSummary,
) -> Vec<&'static str> {
    if !matches!(
        current,
        PromotionStatus::Research | PromotionStatus::Backtest | PromotionStatus::Paper
    ) {
        return Vec::new();
    }
    let mut missing = Vec::new();
    if summary.accepted_oos == 0 {
        missing.push("oos_required");
    }
    if summary.accepted_walk_forward == 0 {
        missing.push("walk_forward_required");
    }
    missing
}

fn missing_paper_conditions(sample: Option<&PaperSample>) -> Vec<&'static str> {
    let Some(sample) = sample else {
        return vec!["paper_sample_required"];
    };
    let mut missing = Vec::new();
    if sample.closed_trades < 200 {
        missing.push("min_closed_trades");
    }
    if sample.calendar_days < 60 {
        missing.push("min_calendar_days");
    }
    if sample.regime_ids.len() < 2 {
        missing.push("min_regimes");
    }
    if !sample.postmortem_ready {
        missing.push("postmortem_required");
    }
    missing
}

#[cfg(test)]
mod tests {
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
        let error = evaluate_promotion(&spec, PromotionStatus::Backtest, &evidence, None, None)
            .unwrap_err();
        assert_eq!(
            error,
            PromotionError::MissingConditions(vec!["walk_forward_required"])
        );
        let report =
            evaluate_promotion(&spec, PromotionStatus::Backtest, &both(), None, None).unwrap();
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
}
