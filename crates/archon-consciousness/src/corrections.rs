//! Correction tracking and learning.
//!
//! Records user corrections, links them to the behavioral rules that
//! caused the mistake, and reinforces rule scores proportional to the
//! correction severity.

use archon_memory::MemoryTrait;
use archon_memory::types::{MemoryType, RelType, SearchFilter};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::rules::{RuleSource, RulesEngine};

const FACTUAL_RULE: (&str, &str) = (
    "rule:correction:factual-error:v2",
    "Verify factual claims against available evidence before presenting them.",
);
const APPROACH_RULE: (&str, &str) = (
    "rule:correction:approach-correction:v2",
    "Review the chosen approach against the user's goal before continuing.",
);
const REPEATED_INSTRUCTION_RULE: (&str, &str) = (
    "rule:correction:repeated-instruction:v2",
    "Re-read and follow relevant user instructions before acting.",
);
const FORBIDDEN_ACTION_RULE: (&str, &str) = (
    "rule:correction:did-forbidden-action:v2",
    "Check constraints and permissions before performing a potentially forbidden action.",
);
const PERMISSION_RULE: (&str, &str) = (
    "rule:correction:acted-without-permission:v2",
    "Obtain explicit user approval before actions that require confirmation.",
);

// ── public types ─────────────────────────────────────────────

/// Classification of a correction with an associated severity multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CorrectionType {
    /// Agent stated something factually wrong.
    FactualError,
    /// Agent took a suboptimal approach.
    ApproachCorrection,
    /// User had to repeat an instruction already given.
    RepeatedInstruction,
    /// Agent performed a forbidden action.
    DidForbiddenAction,
    /// Agent acted without explicit permission.
    ActedWithoutPermission,
}

impl CorrectionType {
    /// Base severity multiplier used when boosting rule scores.
    pub fn severity_multiplier(self) -> f64 {
        match self {
            Self::FactualError => 1.5,
            Self::ApproachCorrection => 2.0,
            Self::RepeatedInstruction => 3.0,
            Self::DidForbiddenAction => 4.0,
            Self::ActedWithoutPermission => 5.0,
        }
    }

    fn as_tag(self) -> String {
        match self {
            Self::FactualError => "ctype:factual_error".into(),
            Self::ApproachCorrection => "ctype:approach_correction".into(),
            Self::RepeatedInstruction => "ctype:repeated_instruction".into(),
            Self::DidForbiddenAction => "ctype:did_forbidden_action".into(),
            Self::ActedWithoutPermission => "ctype:acted_without_permission".into(),
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "ctype:factual_error" => Some(Self::FactualError),
            "ctype:approach_correction" => Some(Self::ApproachCorrection),
            "ctype:repeated_instruction" => Some(Self::RepeatedInstruction),
            "ctype:did_forbidden_action" => Some(Self::DidForbiddenAction),
            "ctype:acted_without_permission" => Some(Self::ActedWithoutPermission),
            _ => None,
        }
    }

    fn derived_rule(self) -> (&'static str, &'static str) {
        match self {
            Self::FactualError => FACTUAL_RULE,
            Self::ApproachCorrection => APPROACH_RULE,
            Self::RepeatedInstruction => REPEATED_INSTRUCTION_RULE,
            Self::DidForbiddenAction => FORBIDDEN_ACTION_RULE,
            Self::ActedWithoutPermission => PERMISSION_RULE,
        }
    }
}

/// A recorded correction event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    pub id: String,
    pub correction_type: CorrectionType,
    /// Free-text description of what went wrong.
    pub content: String,
    /// Situational context in which the mistake occurred.
    pub context: String,
    /// Effective severity (multiplier applied to base score increment).
    pub severity: f64,
    /// Optional link to the rule that was violated.
    pub rule_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

// ── errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CorrectionError {
    #[error("correction not found: {0}")]
    NotFound(String),

    #[error("correction boost outcome is uncertain after retry errors: {0}")]
    BoostOutcomeUnknown(String),

    #[error("correction operation failed: {cause}; cleanup failed: {cleanup}")]
    Cleanup { cause: String, cleanup: String },

    #[error("memory graph error: {0}")]
    Memory(#[from] archon_memory::MemoryError),

    #[error("rules engine error: {0}")]
    Rules(#[from] crate::rules::RulesError),
}

// ── tracker ─────────────────────────────────────────────────

/// Records corrections in the memory graph, links them to rules, and
/// adjusts rule scores proportional to severity.
pub struct CorrectionTracker<'g> {
    graph: &'g dyn MemoryTrait,
    rules: RulesEngine<'g>,
}

impl<'g> CorrectionTracker<'g> {
    /// Create a new tracker backed by the given graph.
    pub fn new(graph: &'g dyn MemoryTrait) -> Self {
        Self {
            graph,
            rules: RulesEngine::new(graph),
        }
    }

    /// Record a correction.
    ///
    /// * Stores the correction as a `MemoryType::Correction` node.
    /// * If `rule_id` is `Some`, creates a `CausedBy` edge from the
    ///   correction to the rule and increments the rule's score by
    ///   `severity_multiplier * 5.0` (clamped to 100).
    /// * If `rule_id` is `None`, a deterministic `CorrectionDerived` rule is
    ///   created or reused without copying correction text into the rule body.
    pub fn record_correction(
        &self,
        correction_type: CorrectionType,
        content: &str,
        context: &str,
        rule_id: Option<&str>,
    ) -> Result<Correction, CorrectionError> {
        self.record_correction_with_id(
            &uuid::Uuid::new_v4().to_string(),
            correction_type,
            content,
            context,
            rule_id,
        )
    }

    /// Record a correction using a caller-stable ID so a lost response can be
    /// retried without applying the rule boost twice.
    pub fn record_correction_with_id(
        &self,
        correction_id: &str,
        correction_type: CorrectionType,
        content: &str,
        context: &str,
        rule_id: Option<&str>,
    ) -> Result<Correction, CorrectionError> {
        let (correction, newly_claimed) =
            self.record_claim(correction_id, correction_type, content, context, rule_id)?;
        self.boost_with_compensation(&correction, newly_claimed)?;
        Ok(correction)
    }

    /// Steps that establish the correction and its link, with no score change.
    ///
    /// Returns the correction and whether THIS call claimed the row, which is
    /// the rollback token [`Self::compensate_new_claim_failure`] needs.
    fn record_claim(
        &self,
        correction_id: &str,
        correction_type: CorrectionType,
        content: &str,
        context: &str,
        rule_id: Option<&str>,
    ) -> Result<(Correction, bool), CorrectionError> {
        let severity = correction_type.severity_multiplier();
        let derived_rule = correction_type.derived_rule();
        let effective_rule_id = rule_id.map_or_else(|| derived_rule.0.to_string(), str::to_string);
        let tags = vec![
            correction_type.as_tag(),
            format!("severity:{severity}"),
            target_rule_tag(&effective_rule_id),
        ];
        let importance = severity * 10.0; // 15..50 range
        let outcome = self.graph.store_memory_with_id_outcome(
            correction_id,
            content,
            "correction",
            MemoryType::Correction,
            importance.min(100.0),
            &tags,
            "correction_tracker",
            context,
        )?;
        let correction = outcome.memory;
        validate_correction_identity(
            &correction,
            correction_type,
            content,
            context,
            &effective_rule_id,
        )?;

        let target_resolution = match rule_id {
            Some(id) => self.validate_explicit_rule(id),
            None => self
                .resolve_derived_rule(derived_rule.0, derived_rule.1)
                .map(|_| ()),
        };
        if let Err(cause) = target_resolution {
            return Err(self.compensate_new_claim_failure(cause, &correction.id, outcome.created));
        }

        if let Err(cause) = self.graph.create_relationship(
            &correction.id,
            &effective_rule_id,
            RelType::CausedBy,
            Some(context),
            severity,
        ) {
            return Err(self.compensate_new_claim_failure(
                cause.into(),
                &correction.id,
                outcome.created,
            ));
        }

        Ok((
            Correction {
                id: correction.id,
                correction_type,
                content: content.to_string(),
                context: context.to_string(),
                severity,
                rule_id: Some(effective_rule_id),
                timestamp: correction.created_at,
            },
            outcome.created,
        ))
    }

    /// Recall corrections similar to the given context string.
    ///
    /// The query is bounded. It used to be issued with no limit at all: the
    /// whole matching set was materialised and `truncate(limit)` threw away
    /// everything but the first few rows. That is called once per iteration of
    /// the agent loop, before the LLM is contacted, so on a large store it was
    /// the dominant cost of a turn.
    ///
    /// The bound handed to the query is [`recall_candidate_window`], not
    /// `limit`, because the results are re-ranked by severity afterwards -- a
    /// maximum-severity correction can sit far below the query's own ordering,
    /// and bounding at `limit` would silently drop it.
    pub fn recall_corrections(
        &self,
        context: &str,
        limit: usize,
    ) -> Result<Vec<Correction>, CorrectionError> {
        let filter = SearchFilter {
            memory_type: Some(MemoryType::Correction),
            text: Some(context.to_string()),
            limit: Some(recall_candidate_window(limit)),
            ..Default::default()
        };
        let memories = self.graph.search_memories(&filter)?;

        let mut corrections: Vec<Correction> = memories
            .into_iter()
            .filter_map(|m| memory_to_correction(m).ok())
            .collect();

        sort_corrections(&mut corrections);
        corrections.truncate(limit);
        Ok(corrections)
    }

    fn validate_explicit_rule(&self, id: &str) -> Result<(), CorrectionError> {
        let memory = match self.graph.inspect_memory(id) {
            Ok(memory) => memory,
            Err(archon_memory::MemoryError::NotFound(_)) => {
                return Err(CorrectionError::Rules(crate::rules::RulesError::NotFound(
                    id.to_string(),
                )));
            }
            Err(error) => return Err(CorrectionError::Memory(error)),
        };
        if memory.memory_type != MemoryType::Rule {
            return Err(CorrectionError::Rules(crate::rules::RulesError::NotFound(
                id.to_string(),
            )));
        }
        Ok(())
    }

    fn resolve_derived_rule(
        &self,
        id: &str,
        text: &str,
    ) -> Result<crate::rules::BehavioralRule, CorrectionError> {
        self.rules
            .add_rule_with_id(id, text, RuleSource::CorrectionDerived)
            .map_err(CorrectionError::from)
    }

    fn compensate_new_claim_failure(
        &self,
        cause: CorrectionError,
        correction_id: &str,
        newly_claimed: bool,
    ) -> CorrectionError {
        if !newly_claimed {
            return cause;
        }
        match self.graph.delete_memory(correction_id) {
            Ok(()) => cause,
            Err(error) => CorrectionError::Cleanup {
                cause: cause.to_string(),
                cleanup: format!("delete correction {correction_id}: {error}"),
            },
        }
    }

    /// Boost a rule's score by `multiplier * 5.0` for one correction.
    fn boost_rule(
        &self,
        rule_id: &str,
        multiplier: f64,
        correction_id: &str,
    ) -> Result<(), CorrectionError> {
        self.rules
            .boost_rule_by(rule_id, multiplier * 5.0, correction_id)?;
        Ok(())
    }
}

// ── helpers ──────────────────────────────────────────────────

fn target_rule_tag(rule_id: &str) -> String {
    format!("target-rule:{rule_id}")
}

fn validate_correction_identity(
    correction: &archon_memory::Memory,
    correction_type: CorrectionType,
    content: &str,
    context: &str,
    target_rule_id: &str,
) -> Result<(), CorrectionError> {
    let expected_severity = correction_type.severity_multiplier();
    let expected_tags = vec![
        correction_type.as_tag(),
        format!("severity:{expected_severity}"),
        target_rule_tag(target_rule_id),
    ];
    if correction.memory_type != MemoryType::Correction
        || correction.content != content
        || correction.title != "correction"
        || correction.source_type != "correction_tracker"
        || correction.project_path != context
        || correction.importance != (expected_severity * 10.0).min(100.0)
        || correction.tags != expected_tags
    {
        return Err(CorrectionError::Memory(
            archon_memory::MemoryError::Database(format!(
                "correction ID collision for {}: existing correction semantics differ",
                correction.id
            )),
        ));
    }
    Ok(())
}

fn is_known_severity(severity: f64) -> bool {
    [
        CorrectionType::FactualError,
        CorrectionType::ApproachCorrection,
        CorrectionType::RepeatedInstruction,
        CorrectionType::DidForbiddenAction,
        CorrectionType::ActedWithoutPermission,
    ]
    .into_iter()
    .any(|correction_type| severity == correction_type.severity_multiplier())
}

/// Candidates fetched per correction the caller asked for.
///
/// `recall_corrections` re-ranks by severity after the query returns, so the
/// query must hand back a pool that is wider than the caller's limit. Bounding
/// the query at `limit` would mean the top-severity correction is only found
/// when it also happens to rank in the query's own top `limit`.
const RECALL_CANDIDATE_FACTOR: usize = 20;

/// Lower bound on that pool, so a small `limit` still ranks over a real set.
const RECALL_CANDIDATE_FLOOR: usize = 200;

/// How many rows the recall query is allowed to return.
///
/// Returns `0` for `limit == 0` (the caller wants nothing; do not query), and
/// otherwise a value comfortably above `limit` but bounded -- the point of the
/// change is that this is a number at all, rather than "every matching row in
/// the store".
fn recall_candidate_window(limit: usize) -> usize {
    if limit == 0 {
        return 0;
    }
    limit
        .saturating_mul(RECALL_CANDIDATE_FACTOR)
        .max(RECALL_CANDIDATE_FLOOR)
}

fn sort_corrections(corrections: &mut [Correction]) {
    corrections.sort_by(|left, right| {
        right
            .severity
            .total_cmp(&left.severity)
            .then_with(|| right.timestamp.cmp(&left.timestamp))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Convert a [`Memory`] into a [`Correction`].
fn memory_to_correction(m: archon_memory::Memory) -> Result<Correction, CorrectionError> {
    let correction_type = m
        .tags
        .iter()
        .find_map(|t| CorrectionType::from_tag(t))
        .unwrap_or(CorrectionType::FactualError);

    let severity = m
        .tags
        .iter()
        .find_map(|tag| {
            tag.strip_prefix("severity:")
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|severity| is_known_severity(*severity))
        })
        .unwrap_or_else(|| correction_type.severity_multiplier());

    // Try to find a linked rule via relationships (best-effort).
    // We don't have relationship data on the Memory struct, so we
    // leave rule_id as None in the recalled view.
    Ok(Correction {
        id: m.id,
        correction_type,
        content: m.content,
        context: m.project_path.clone(),
        severity,
        rule_id: None,
        timestamp: m.created_at,
    })
}

// ── reinforcement ────────────────────────────
//
// The rule-score half of recording a correction, split out so it can be
// withheld until an attribution justifies it. See the module note there.

#[path = "corrections/reinforcement.rs"]
mod reinforcement;

// ── tests ────────────────────────────────────

#[cfg(test)]
#[path = "corrections/tests.rs"]
mod tests;
