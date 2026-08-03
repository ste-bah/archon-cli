//! Assembling one row per requirement, and the direct F1 detector.
//!
//! Data only — no rendering. The command layer formats this; keeping the model
//! free of formatting is what lets a test assert on a level rather than on a
//! string of dashes.
//!
//! # Fail-closed, per PRD §32
//!
//! An unproven requirement is neither a pass nor a failure: it is a **declared
//! residual gap** with named fail-closed behaviour. [`TraceReport::gate_verdict`]
//! is the only thing that turns rows into a verdict, and it refuses to call a
//! `Candidate` satisfied no matter how many of them there are. §32 forbids
//! "should work", "later" and "best effort" without stating what fails closed;
//! every row below `Exercised` carries a [`MissingForPromotion`] that names the
//! specific fact that is absent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::anchors::{Anchor, AnchorFreshness, AnchorGap};
use super::coverage::CoverageReport;
use super::falsification::{FalsificationPlan, NotPlannable};
use super::ladder::{ExercisedProof, MissingForPromotion, ProofLevel};
use super::requirements::Severity;

/// One anchor with everything decided about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchorVerdict {
    pub anchor: Anchor,
    pub freshness: AnchorFreshness,
    pub level: ProofLevel,
    pub proof: Option<ExercisedProof>,
    pub missing: Option<MissingForPromotion>,
    pub falsification: std::result::Result<FalsificationPlan, NotPlannable>,
}

/// One requirement's whole story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementRow {
    pub requirement_id: String,
    pub severity: Severity,
    pub severity_evidence: Option<String>,
    /// Tasks claiming this requirement via `implements:`.
    pub claimed_by: Vec<String>,
    pub anchors: Vec<AnchorVerdict>,
    /// Why there are no anchors, when there are none.
    pub anchor_gap: Option<AnchorGap>,
    /// The strongest level any live anchor reached. `Unproven` when none did.
    pub level: ProofLevel,
}

impl RequirementRow {
    /// The single satisfaction question, answered in one place.
    pub fn satisfied(&self) -> bool {
        self.level.satisfies_promotion_gate()
    }

    /// Everything standing between this requirement and `Exercised`, in the
    /// words of whichever check found it absent.
    pub fn missing_reasons(&self) -> Vec<String> {
        if self.claimed_by.is_empty() {
            return vec![
                "no task claims this requirement (`implements:`); this is a decomposition gap, \
                 not a code gap"
                    .to_string(),
            ];
        }
        if let Some(gap) = &self.anchor_gap {
            return vec![describe_anchor_gap(gap)];
        }
        let mut reasons: Vec<String> = Vec::new();
        for verdict in &self.anchors {
            match &verdict.freshness {
                AnchorFreshness::Stale { recorded, current } => reasons.push(format!(
                    "anchor {} is stale: file hashed {} when anchored, hashes {} now — \
                     re-index out of band before trusting the line range",
                    verdict.anchor.citation(),
                    short(recorded),
                    short(current),
                )),
                AnchorFreshness::FileMissing => reasons.push(format!(
                    "anchor {} names a file that is gone",
                    verdict.anchor.citation()
                )),
                AnchorFreshness::Fresh => {
                    if let Some(missing) = &verdict.missing {
                        reasons.push(format!(
                            "anchor {}: {}",
                            verdict.anchor.citation(),
                            missing.describe()
                        ));
                    }
                }
            }
        }
        reasons.sort();
        reasons.dedup();
        reasons
    }
}

fn describe_anchor_gap(gap: &AnchorGap) -> String {
    match gap {
        AnchorGap::Unclaimed => "no task claims this requirement (`implements:`)".to_string(),
        AnchorGap::NoDeclaredPaths { task_id } => format!(
            "{task_id} declares no paths under `## Files Expected to Change`, so there is \
             nothing to scope a search to; a repository-wide search would return the same \
             generic top hit for every requirement, which is the shape of finding F1"
        ),
        AnchorGap::NoHitInScope { task_id } => {
            format!("the code index returned nothing inside {task_id}'s declared paths")
        }
    }
}

fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// One `file:line-line` cited by more than one requirement.
///
/// The direct F1 detector. F1's counter-evidence was *"repeated generic evidence
/// for REQ-DL-001..004"* — the same support standing in for four requirements.
/// With anchored edges that reuse is a computable property of the graph rather
/// than something a reviewer has to notice by reading.
///
/// Reported, not enforced. One function legitimately satisfying two requirements
/// is real; the same span answering for twenty is not, and the difference is a
/// judgement a reader makes from the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedAnchor {
    pub citation: String,
    pub requirement_ids: Vec<String>,
}

/// The whole report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceReport {
    pub prd_path: String,
    pub task_dir: String,
    pub coverage: CoverageReport,
    pub rows: Vec<RequirementRow>,
    pub shared_anchors: Vec<SharedAnchor>,
    /// Anchors whose file changed since they were recorded.
    pub stale_anchors: usize,
    /// Whether a code index was consulted at all.
    pub index_consulted: bool,
}

impl TraceReport {
    /// Requirements at each level, for the summary line.
    pub fn level_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::from([
            (ProofLevel::Unproven.as_str(), 0),
            (ProofLevel::Candidate.as_str(), 0),
            (ProofLevel::Exercised.as_str(), 0),
            (ProofLevel::Falsifiable.as_str(), 0),
        ]);
        for row in &self.rows {
            *counts.entry(row.level.as_str()).or_default() += 1;
        }
        counts
    }

    pub fn satisfied(&self) -> Vec<&RequirementRow> {
        self.rows.iter().filter(|row| row.satisfied()).collect()
    }

    /// Requirements that are declared residual gaps under PRD §32.
    pub fn residual_gaps(&self) -> Vec<&RequirementRow> {
        self.rows.iter().filter(|row| !row.satisfied()).collect()
    }

    /// The verdict, stated so it can be neither a pass nor a failure.
    ///
    /// A traceability report that "failed" would be treated as flaky and muted;
    /// one that "passed" would be F1 again. It reports a count and refuses to
    /// call anything below `Exercised` satisfied.
    pub fn gate_verdict(&self) -> String {
        let satisfied = self.satisfied().len();
        let total = self.rows.len();
        let gaps = total - satisfied;
        if total == 0 {
            return "no requirements extracted; nothing to report".to_string();
        }
        format!(
            "{satisfied}/{total} requirements satisfied on evidence (Exercised or above). \
             {gaps} are declared residual gaps: each names what is missing, none counts as \
             satisfied, and none satisfies a promotion gate."
        )
    }
}

/// Group anchors by citation to find evidence reused across requirements.
pub fn find_shared_anchors(rows: &[RequirementRow]) -> Vec<SharedAnchor> {
    let mut by_citation: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        for verdict in &row.anchors {
            let claimants = by_citation.entry(verdict.anchor.citation()).or_default();
            if !claimants.contains(&row.requirement_id) {
                claimants.push(row.requirement_id.clone());
            }
        }
    }
    by_citation
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(citation, requirement_ids)| SharedAnchor {
            citation,
            requirement_ids,
        })
        .collect()
}

/// The strongest level among anchors that are still fresh.
///
/// A stale anchor contributes nothing: its line range describes a file that no
/// longer exists in that form, so whatever a verifier once read there is not
/// evidence about the code as it stands. Known-stale beats silently-wrong.
pub fn strongest_level(anchors: &[AnchorVerdict]) -> ProofLevel {
    anchors
        .iter()
        .filter(|verdict| verdict.freshness.is_fresh())
        .map(|verdict| verdict.level)
        .max()
        .unwrap_or(ProofLevel::Unproven)
}

#[cfg(test)]
mod tests;
