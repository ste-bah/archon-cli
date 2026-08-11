//! Building rule observations from rules and the corrections behind them.
//!
//! The pure retirement analysis in `archon-memory` takes plain data and holds no
//! store handle, so something has to read that data. This is that reader, and it
//! is deliberately the only place the two sources are joined.
//!
//! # Provenance comes from correction records
//!
//! Rules ship as a small set of fixed identities with constant text, one per
//! correction category. That was the right fix for an earlier defect — raw user
//! text can never reach a rule body — and it means rule text carries no
//! provenance at all: two rules of the same category read identically whether
//! one correction or a hundred produced them.
//!
//! So the evidence is read from the correction rows themselves. Each correction
//! carries a `target-rule:` tag naming the rule it was matched to, which makes
//! "how many corrections support this rule, and when was the most recent" a
//! direct query rather than an inference from rule text.
//!
//! Reading the tag rather than the `Correction` struct is deliberate: the
//! struct's `rule_id` is reconstructed as `None` when a correction is read back
//! out of the graph, so the recall API cannot answer this question and the tag
//! is the only surviving link.

use archon_consciousness::rules::{
    MAX_PROMPT_RULES, MIN_PROMPT_RULE_SCORE, RuleSource, RulesEngine,
};
use archon_memory::MemoryTrait;
use archon_memory::garden::{RuleObservation, RuleOrigin};
use archon_memory::types::{MemoryType, SearchFilter};

/// Tag a correction carries naming the rule it was matched to.
const TARGET_RULE_TAG_PREFIX: &str = "target-rule:";

/// Read every rule and the correction evidence behind it.
///
/// Returns an empty vector rather than failing when rules cannot be read: a
/// consolidation pass that cannot see rules should propose no rule retirements,
/// which is what an empty observation set produces.
pub(crate) fn rule_observations(memory: &dyn MemoryTrait) -> Vec<RuleObservation> {
    let engine = RulesEngine::new(memory);
    let rules = match engine.get_rules_sorted() {
        Ok(rules) => rules,
        Err(error) => {
            tracing::warn!(%error, "garden: could not read rules; proposing no retirements");
            return Vec::new();
        }
    };

    rules
        .iter()
        .enumerate()
        .map(|(rank, rule)| {
            let (supporting, most_recent) = correction_evidence(memory, &rule.id);
            RuleObservation {
                rule_id: rule.id.clone(),
                rule_text: rule.text.clone(),
                score: rule.score,
                origin: match rule.source {
                    RuleSource::UserDefined => RuleOrigin::UserDefined,
                    RuleSource::CorrectionDerived => RuleOrigin::CorrectionDerived,
                    RuleSource::SystemDefault => RuleOrigin::SystemDefault,
                },
                created_at: rule.created_at,
                last_triggered: rule.last_triggered,
                supporting_corrections: supporting,
                most_recent_correction: most_recent,
                // Reproduces the prompt block's own admission rule rather than
                // approximating it: `get_rules_sorted` is score-descending, so
                // rank below the cap and score at or above the floor is exactly
                // what `format_for_prompt` keeps.
                in_prompt: rank < MAX_PROMPT_RULES && rule.score >= MIN_PROMPT_RULE_SCORE,
            }
        })
        .collect()
}

/// How many corrections name this rule, and when the most recent was recorded.
fn correction_evidence(
    memory: &dyn MemoryTrait,
    rule_id: &str,
) -> (usize, Option<chrono::DateTime<chrono::Utc>>) {
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Correction),
        tags: vec![format!("{TARGET_RULE_TAG_PREFIX}{rule_id}")],
        require_all_tags: true,
        ..SearchFilter::default()
    };
    match memory.search_memories(&filter) {
        Ok(rows) => {
            let newest = rows.iter().map(|row| row.created_at).max();
            (rows.len(), newest)
        }
        Err(error) => {
            // Reported as "no evidence found" would be wrong in the dangerous
            // direction: absent corrections read as silence, and silence is
            // what retires a rule. A failed read must not look like silence, so
            // this claims one supporting correction dated now, which keeps the
            // rule.
            tracing::warn!(%error, rule_id, "garden: correction evidence unreadable; keeping the rule");
            (1, Some(chrono::Utc::now()))
        }
    }
}

#[cfg(test)]
#[path = "garden_rule_observations_tests.rs"]
mod tests;
