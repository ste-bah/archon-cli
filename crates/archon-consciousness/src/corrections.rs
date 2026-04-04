//! Correction tracking and learning.
//!
//! Records user corrections, links them to the behavioral rules that
//! caused the mistake, and reinforces rule scores proportional to the
//! correction severity.

use archon_memory::types::{MemoryType, RelType, SearchFilter};
use archon_memory::MemoryGraph;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::rules::{RuleSource, RulesEngine};

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

    #[error("memory graph error: {0}")]
    Memory(#[from] archon_memory::MemoryError),

    #[error("rules engine error: {0}")]
    Rules(#[from] crate::rules::RulesError),
}

// ── tracker ─────────────────────────────────────────────────

/// Records corrections in the memory graph, links them to rules, and
/// adjusts rule scores proportional to severity.
pub struct CorrectionTracker<'g> {
    graph: &'g MemoryGraph,
    rules: RulesEngine<'g>,
}

impl<'g> CorrectionTracker<'g> {
    /// Create a new tracker backed by the given graph.
    pub fn new(graph: &'g MemoryGraph) -> Self {
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
    /// * If `rule_id` is `None`, a new `CorrectionDerived` rule is
    ///   auto-created from the correction content and linked.
    pub fn record_correction(
        &self,
        correction_type: CorrectionType,
        content: &str,
        context: &str,
        rule_id: Option<&str>,
    ) -> Result<Correction, CorrectionError> {
        let severity = correction_type.severity_multiplier();

        let tags = vec![
            correction_type.as_tag(),
            format!("severity:{severity}"),
        ];

        let importance = severity * 10.0; // 15..50 range
        let mem_id = self.graph.store_memory(
            content,
            "correction",
            MemoryType::Correction,
            importance.min(100.0),
            &tags,
            "correction_tracker",
            context,
        )?;

        let effective_rule_id = match rule_id {
            Some(rid) => {
                // Link correction -> rule via CausedBy
                self.graph.create_relationship(
                    &mem_id,
                    rid,
                    RelType::CausedBy,
                    Some(context),
                    severity,
                )?;
                self.boost_rule(rid, severity)?;
                Some(rid.to_string())
            }
            None => {
                // Auto-create a rule from the correction.
                let rule_text = format!("Avoid: {content}");
                let rule = self.rules.add_rule(&rule_text, RuleSource::CorrectionDerived)?;
                self.graph.create_relationship(
                    &mem_id,
                    &rule.id,
                    RelType::CausedBy,
                    Some(context),
                    severity,
                )?;
                self.boost_rule(&rule.id, severity)?;
                Some(rule.id)
            }
        };

        let mem = self.graph.get_memory(&mem_id)?;

        Ok(Correction {
            id: mem_id,
            correction_type,
            content: content.to_string(),
            context: context.to_string(),
            severity,
            rule_id: effective_rule_id,
            timestamp: mem.created_at,
        })
    }

    /// Recall corrections similar to the given context string.
    pub fn recall_corrections(
        &self,
        context: &str,
        limit: usize,
    ) -> Result<Vec<Correction>, CorrectionError> {
        let filter = SearchFilter {
            memory_type: Some(MemoryType::Correction),
            text: Some(context.to_string()),
            ..Default::default()
        };
        let memories = self.graph.search_memories(&filter)?;

        let mut corrections: Vec<Correction> = memories
            .into_iter()
            .filter_map(|m| memory_to_correction(m).ok())
            .collect();

        corrections.truncate(limit);
        Ok(corrections)
    }

    /// Boost a rule's score by `multiplier * 5.0`, clamped to 100.
    fn boost_rule(&self, rule_id: &str, multiplier: f64) -> Result<(), CorrectionError> {
        let mem = self.graph.get_memory(rule_id)?;
        let increment = multiplier * 5.0;
        let new_score = (mem.importance + increment).min(100.0);
        self.graph.update_importance(rule_id, new_score)?;
        Ok(())
    }
}

// ── helpers ──────────────────────────────────────────────────

/// Convert a [`Memory`] into a [`Correction`].
fn memory_to_correction(
    m: archon_memory::Memory,
) -> Result<Correction, CorrectionError> {
    let correction_type = m
        .tags
        .iter()
        .find_map(|t| CorrectionType::from_tag(t))
        .unwrap_or(CorrectionType::FactualError);

    let severity = m
        .tags
        .iter()
        .find_map(|t| {
            t.strip_prefix("severity:")
                .and_then(|v| v.parse::<f64>().ok())
        })
        .unwrap_or(1.0);

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

// ── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracker() -> (MemoryGraph, ()) {
        let graph = MemoryGraph::in_memory()
            .expect("in-memory graph should succeed");
        (graph, ())
    }

    #[test]
    fn severity_multipliers_are_ordered() {
        assert!(
            CorrectionType::FactualError.severity_multiplier()
                < CorrectionType::ApproachCorrection.severity_multiplier()
        );
        assert!(
            CorrectionType::ApproachCorrection.severity_multiplier()
                < CorrectionType::RepeatedInstruction.severity_multiplier()
        );
        assert!(
            CorrectionType::RepeatedInstruction.severity_multiplier()
                < CorrectionType::DidForbiddenAction.severity_multiplier()
        );
        assert!(
            CorrectionType::DidForbiddenAction.severity_multiplier()
                < CorrectionType::ActedWithoutPermission.severity_multiplier()
        );
    }

    #[test]
    fn record_correction_with_existing_rule() {
        let (graph, _) = make_tracker();
        let tracker = CorrectionTracker::new(&graph);

        // Create a rule first.
        let rules = RulesEngine::new(&graph);
        let rule = rules
            .add_rule("Always ask before modifying files", RuleSource::UserDefined)
            .expect("add_rule");
        let original_score = rule.score; // 50.0

        let correction = tracker
            .record_correction(
                CorrectionType::ActedWithoutPermission,
                "Modified config.toml without asking",
                "editing session",
                Some(&rule.id),
            )
            .expect("record_correction");

        assert_eq!(correction.correction_type, CorrectionType::ActedWithoutPermission);
        assert!((correction.severity - 5.0).abs() < f64::EPSILON);
        assert_eq!(correction.rule_id.as_deref(), Some(rule.id.as_str()));

        // Rule score should have been boosted by 5.0 * 5.0 = 25.0
        let updated = graph.get_memory(&rule.id).expect("get rule");
        let expected = original_score + 25.0;
        assert!(
            (updated.importance - expected).abs() < f64::EPSILON,
            "expected {expected}, got {}",
            updated.importance,
        );
    }

    #[test]
    fn record_correction_auto_creates_rule() {
        let (graph, _) = make_tracker();
        let tracker = CorrectionTracker::new(&graph);

        let correction = tracker
            .record_correction(
                CorrectionType::FactualError,
                "Stated Rust 2024 edition does not exist",
                "research session",
                None,
            )
            .expect("record_correction");

        // A rule should have been auto-created.
        assert!(correction.rule_id.is_some());

        let rule_id = correction.rule_id.as_ref().expect("rule_id");
        let rule_mem = graph.get_memory(rule_id).expect("get auto-rule");
        assert!(rule_mem.content.starts_with("Avoid:"));

        // Rule score should be boosted from 50.0 by 1.5 * 5.0 = 7.5
        let expected = 50.0 + 7.5;
        assert!(
            (rule_mem.importance - expected).abs() < f64::EPSILON,
            "expected {expected}, got {}",
            rule_mem.importance,
        );
    }

    #[test]
    fn recall_corrections_finds_stored() {
        let (graph, _) = make_tracker();
        let tracker = CorrectionTracker::new(&graph);

        tracker
            .record_correction(
                CorrectionType::RepeatedInstruction,
                "User already said not to create README files",
                "doc session",
                None,
            )
            .expect("record");

        tracker
            .record_correction(
                CorrectionType::ApproachCorrection,
                "Should have used edit instead of write",
                "coding session",
                None,
            )
            .expect("record");

        let results = tracker
            .recall_corrections("README", 10)
            .expect("recall");

        assert!(
            !results.is_empty(),
            "should find at least one correction matching 'README'",
        );
        assert!(results[0].content.contains("README"));
    }

    #[test]
    fn recall_with_limit_truncates() {
        let (graph, _) = make_tracker();
        let tracker = CorrectionTracker::new(&graph);

        for i in 0..5 {
            tracker
                .record_correction(
                    CorrectionType::FactualError,
                    &format!("error number {i}"),
                    "bulk",
                    None,
                )
                .expect("record");
        }

        let results = tracker.recall_corrections("error", 2).expect("recall");
        assert!(results.len() <= 2);
    }

    #[test]
    fn correction_type_tag_roundtrip() {
        let types = [
            CorrectionType::FactualError,
            CorrectionType::ApproachCorrection,
            CorrectionType::RepeatedInstruction,
            CorrectionType::DidForbiddenAction,
            CorrectionType::ActedWithoutPermission,
        ];
        for ct in &types {
            let tag = ct.as_tag();
            let parsed = CorrectionType::from_tag(&tag);
            assert_eq!(parsed, Some(*ct), "roundtrip failed for {tag}");
        }
    }

    #[test]
    fn boost_clamps_at_100() {
        let (graph, _) = make_tracker();
        let tracker = CorrectionTracker::new(&graph);

        let rules = RulesEngine::new(&graph);
        let rule = rules
            .add_rule("fragile rule", RuleSource::SystemDefault)
            .expect("add");

        // Set score close to max.
        graph
            .update_importance(&rule.id, 98.0)
            .expect("set score");

        // DidForbiddenAction => 4.0 * 5.0 = 20.0 boost, should clamp.
        tracker
            .record_correction(
                CorrectionType::DidForbiddenAction,
                "created a file without permission",
                "test",
                Some(&rule.id),
            )
            .expect("record");

        let updated = graph.get_memory(&rule.id).expect("get");
        assert!(
            (updated.importance - 100.0).abs() < f64::EPSILON,
            "should clamp to 100.0, got {}",
            updated.importance,
        );
    }

    #[test]
    fn correction_persists_in_graph() {
        let (graph, _) = make_tracker();
        let tracker = CorrectionTracker::new(&graph);

        let correction = tracker
            .record_correction(
                CorrectionType::ApproachCorrection,
                "Used unwrap in library code",
                "code review",
                None,
            )
            .expect("record");

        // Verify the memory is retrievable directly.
        let mem = graph.get_memory(&correction.id).expect("get");
        assert_eq!(mem.memory_type, MemoryType::Correction);
        assert!(mem.content.contains("unwrap"));
    }
}
