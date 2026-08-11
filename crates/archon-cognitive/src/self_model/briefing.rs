//! Startup briefing generated from self-model facts and unresolved lessons.
//!
//! Roadmap slice 6, item 7. `SelfModelStore::export_briefing` already counted
//! rows, but nothing turned those rows into something the agent reads, so a
//! self-model that had been measured for weeks changed no behaviour.
//!
//! The rule the briefing has to respect is that **an absent fact is absent**.
//! A domain with no `domain_trust` fact does not get a neutral 0.5 line: it is
//! listed by name under "no measured evidence", because a reader (human or
//! model) cannot otherwise tell a measured 0.5 apart from a fabricated one.

use std::collections::{BTreeMap, BTreeSet};

use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::CognitiveError;
use crate::cozo_guard::run_script_guarded;
use crate::executive_support::domain_for;
use crate::reflection_recall::{UnresolvedReflection, render_block};
use crate::schema::ensure_cognitive_schema;
use crate::self_model::prediction::TRUST_DIMENSION;
use crate::self_model::types::FactKind;
use crate::types::SituationKind;

/// A domain the self-model has actually measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasuredDomain {
    pub domain: String,
    pub confidence: f32,
    pub evidence_count: u64,
}

/// Everything the first turn of a session is told about the agent's own state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SelfModelStartupBriefing {
    pub measured: Vec<MeasuredDomain>,
    /// Domains reachable from a situation kind that have no fact at all.
    pub unmeasured_domains: Vec<String>,
    pub caution_rules: Vec<String>,
    pub active_failure_clusters: usize,
    pub unresolved_lessons: Vec<UnresolvedReflection>,
}

impl SelfModelStartupBriefing {
    /// Whether the briefing has anything to say beyond "nothing is measured".
    ///
    /// A briefing listing only unmeasured domains is noise, and injecting it
    /// would make an unmeasured self-model look like a reported one.
    pub fn is_empty(&self) -> bool {
        self.measured.is_empty()
            && self.caution_rules.is_empty()
            && self.unresolved_lessons.is_empty()
            && self.active_failure_clusters == 0
    }

    /// Prompt block, or `None` when there is nothing measured to report.
    pub fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut text = String::from(
            "<self_model_briefing>\nMeasured from verified outcomes only. A domain absent from \
             the list below has no measurement — treat it as unknown, not as average.\n",
        );
        for domain in &self.measured {
            text.push_str(&format!(
                "- {}: confidence {:.2} over {} verified outcomes\n",
                domain.domain, domain.confidence, domain.evidence_count
            ));
        }
        if !self.unmeasured_domains.is_empty() {
            text.push_str(&format!(
                "No measured evidence yet for: {}\n",
                self.unmeasured_domains.join(", ")
            ));
        }
        if self.active_failure_clusters > 0 {
            text.push_str(&format!(
                "Active failure clusters on record: {}\n",
                self.active_failure_clusters
            ));
        }
        for rule in &self.caution_rules {
            text.push_str(&format!("Caution: {rule}\n"));
        }
        if let Some(block) = render_block(&self.unresolved_lessons) {
            text.push_str(&block);
        }
        text.push_str("</self_model_briefing>");
        Some(text)
    }
}

/// Read the current self-model facts and attach `unresolved_lessons`.
///
/// The lessons are passed in rather than queried here so the briefing and the
/// per-turn injection select from exactly one pool, and one injection budget.
pub fn build(
    db: &DbInstance,
    unresolved_lessons: Vec<UnresolvedReflection>,
) -> Result<SelfModelStartupBriefing, CognitiveError> {
    ensure_cognitive_schema(db)?;
    let rows = run_script_guarded(
        db,
        "?[fact_id, domain, fact_kind, statement, confidence, evidence_count] := \
         *self_model_facts{fact_id, domain, fact_kind, statement, confidence, evidence_count}",
        Default::default(),
        ScriptMutability::Immutable,
        "read self-model facts for startup briefing",
    )?;

    let mut measured: BTreeMap<String, MeasuredDomain> = BTreeMap::new();
    let mut caution_rules = Vec::new();
    let mut active_failure_clusters = 0;
    for row in &rows.rows {
        let fact_kind = str_col(row, 2);
        if fact_kind == FactKind::CautionRule.as_str() {
            caution_rules.push(str_col(row, 3));
            continue;
        }
        if fact_kind == FactKind::FailureCluster.as_str() {
            active_failure_clusters += 1;
            continue;
        }
        if fact_kind != FactKind::DomainTrust.as_str() {
            continue;
        }
        let Some(confidence) = row[4].get_float().map(|value| value as f32) else {
            continue;
        };
        // A non-finite confidence is not a measurement. Dropping it keeps the
        // briefing from rendering `NaN` as if it were an observation.
        if !confidence.is_finite() {
            continue;
        }
        let domain = str_col(row, 1);
        measured.insert(
            domain.clone(),
            MeasuredDomain {
                domain,
                confidence,
                evidence_count: row[5].get_int().unwrap_or(0).max(0) as u64,
            },
        );
    }

    let unmeasured_domains = known_domains()
        .into_iter()
        .filter(|domain| !measured.contains_key(domain))
        .collect();
    caution_rules.sort();
    caution_rules.dedup();

    Ok(SelfModelStartupBriefing {
        measured: measured.into_values().collect(),
        unmeasured_domains,
        caution_rules,
        active_failure_clusters,
        unresolved_lessons,
    })
}

/// Fact id the domain-trust fact for `domain` is written under.
///
/// Shared with the writer's `trust_fact_id` through the same dimension
/// constant, so the briefing reads the ids the writer produces.
pub fn trust_fact_id(domain: &str) -> String {
    format!("{TRUST_DIMENSION}:{domain}")
}

/// Every domain a situation kind can map to.
pub fn known_domains() -> Vec<String> {
    SituationKind::ALL
        .iter()
        .map(|kind| domain_for(*kind).to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}
