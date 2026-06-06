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
    pub dataset_degraded: bool,
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
    pub excluded_degraded: usize,
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
            && !self.dataset_degraded
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
            excluded_degraded: 0,
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
    if item.dataset_degraded {
        summary.excluded_degraded += 1;
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
#[path = "promotion_tests.rs"]
mod tests;
