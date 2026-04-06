//! Behavioral rules engine.
//!
//! Rules are stored in the [`MemoryGraph`] as memories with
//! [`MemoryType::Rule`]. The attention score lives in the `importance`
//! field and source/trend metadata are encoded as tags.

use archon_memory::MemoryTrait;
use archon_memory::types::{MemoryType, SearchFilter};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── public types ─────────────────────────────────────────────

/// Direction the rule's score has been moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trend {
    Rising,
    Stable,
    Declining,
}

impl Trend {
    fn as_tag(self) -> String {
        match self {
            Self::Rising => "trend:rising".into(),
            Self::Stable => "trend:stable".into(),
            Self::Declining => "trend:declining".into(),
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "trend:rising" => Some(Self::Rising),
            "trend:stable" => Some(Self::Stable),
            "trend:declining" => Some(Self::Declining),
            _ => None,
        }
    }

    /// Arrow glyph used in prompt formatting.
    fn arrow(self) -> &'static str {
        match self {
            Self::Rising => "up",
            Self::Stable => "stable",
            Self::Declining => "down",
        }
    }
}

/// Where a rule originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleSource {
    UserDefined,
    CorrectionDerived,
    SystemDefault,
}

impl RuleSource {
    fn as_tag(self) -> String {
        match self {
            Self::UserDefined => "source:user_defined".into(),
            Self::CorrectionDerived => "source:correction_derived".into(),
            Self::SystemDefault => "source:system_default".into(),
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "source:user_defined" => Some(Self::UserDefined),
            "source:correction_derived" => Some(Self::CorrectionDerived),
            "source:system_default" => Some(Self::SystemDefault),
            _ => None,
        }
    }
}

/// A single behavioral rule with an attention score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralRule {
    pub id: String,
    pub text: String,
    /// Attention score in `0.0..=100.0`. Higher = more prominent in the
    /// system prompt.
    pub score: f64,
    pub trend: Trend,
    pub source: RuleSource,
    pub created_at: DateTime<Utc>,
    pub last_triggered: Option<DateTime<Utc>>,
}

// ── errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RulesError {
    #[error("rule not found: {0}")]
    NotFound(String),

    #[error("memory graph error: {0}")]
    Memory(#[from] archon_memory::MemoryError),
}

// ── engine ───────────────────────────────────────────────────

/// Manages behavioral rules stored in the memory graph.
pub struct RulesEngine<'g> {
    graph: &'g dyn MemoryTrait,
}

impl<'g> RulesEngine<'g> {
    /// Create a new engine backed by the given graph.
    pub fn new(graph: &'g dyn MemoryTrait) -> Self {
        Self { graph }
    }

    /// Add a new rule and return the populated struct.
    pub fn add_rule(&self, text: &str, source: RuleSource) -> Result<BehavioralRule, RulesError> {
        let score: f64 = 50.0;
        let trend = Trend::Stable;
        let tags = vec![source.as_tag(), trend.as_tag()];

        let id = self.graph.store_memory(
            text,
            "", // title
            MemoryType::Rule,
            score,
            &tags,
            "rules_engine",
            "",
        )?;

        let mem = self.graph.get_memory(&id)?;

        Ok(BehavioralRule {
            id,
            text: text.to_string(),
            score,
            trend,
            source,
            created_at: mem.created_at,
            last_triggered: None,
        })
    }

    /// Retrieve all rules sorted by score descending.
    pub fn get_rules_sorted(&self) -> Result<Vec<BehavioralRule>, RulesError> {
        let filter = SearchFilter {
            memory_type: Some(MemoryType::Rule),
            ..Default::default()
        };
        let memories = self.graph.search_memories(&filter)?;

        let mut rules: Vec<BehavioralRule> = memories
            .into_iter()
            .filter_map(|m| memory_to_rule(m).ok())
            .collect();

        rules.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(rules)
    }

    /// Increment a rule's score by 5.0 (clamped to 100.0) and update
    /// its `last_triggered` timestamp.
    pub fn reinforce_rule(&self, id: &str) -> Result<BehavioralRule, RulesError> {
        let mem = self
            .graph
            .get_memory(id)
            .map_err(|_| RulesError::NotFound(id.to_string()))?;
        let new_score = (mem.importance + 5.0).min(100.0);
        self.graph.update_importance(id, new_score)?;

        // Update the last_triggered tag.
        let now_str = Utc::now().to_rfc3339();
        let mut tags: Vec<String> = mem
            .tags
            .iter()
            .filter(|t| !t.starts_with("last_triggered:"))
            .cloned()
            .collect();
        tags.push(format!("last_triggered:{now_str}"));
        self.graph.update_memory(id, None, Some(&tags))?;

        let updated = self.graph.get_memory(id)?;
        memory_to_rule(updated).map_err(|_| RulesError::NotFound(id.to_string()))
    }

    /// Decay all rule scores by `rate` (subtracted), clamping to 0.0.
    pub fn decay_scores(&self, rate: f64) -> Result<(), RulesError> {
        let rules = self.get_rules_sorted()?;
        for rule in &rules {
            let new_score = (rule.score - rate).max(0.0);
            self.graph.update_importance(&rule.id, new_score)?;
        }
        Ok(())
    }

    /// Remove a rule from the graph.
    pub fn remove_rule(&self, id: &str) -> Result<(), RulesError> {
        self.graph
            .delete_memory(id)
            .map_err(|_| RulesError::NotFound(id.to_string()))
    }

    /// Update the text of an existing rule.
    pub fn update_rule(&self, id: &str, text: &str) -> Result<(), RulesError> {
        self.graph
            .update_memory(id, Some(text), None)
            .map_err(|_| RulesError::NotFound(id.to_string()))
    }

    /// Calculate the trend of a rule based on its current score vs a
    /// reference threshold. In a full implementation this would compare
    /// against a historical snapshot; here we use a simple heuristic:
    /// score > 60 → Rising, score < 40 → Declining, else Stable.
    pub fn calculate_trend(rule: &BehavioralRule) -> Trend {
        if rule.score > 60.0 {
            Trend::Rising
        } else if rule.score < 40.0 {
            Trend::Declining
        } else {
            Trend::Stable
        }
    }

    /// Render all rules into a block suitable for system-prompt
    /// injection.
    pub fn format_for_prompt(&self) -> Result<String, RulesError> {
        let rules = self.get_rules_sorted()?;
        if rules.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("<behavioral_rules>\n## Rules (sorted by priority)\n");
        for (i, r) in rules.iter().enumerate() {
            let trend = Self::calculate_trend(r);
            out.push_str(&format!(
                "{}. [score: {:.1} {}] {}\n",
                i + 1,
                r.score,
                trend.arrow(),
                r.text,
            ));
        }
        out.push_str("</behavioral_rules>");
        Ok(out)
    }
}

// ── helpers ──────────────────────────────────────────────────

/// Convert a [`Memory`] into a [`BehavioralRule`].
fn memory_to_rule(m: archon_memory::Memory) -> Result<BehavioralRule, RulesError> {
    let source = m
        .tags
        .iter()
        .find_map(|t| RuleSource::from_tag(t))
        .unwrap_or(RuleSource::SystemDefault);

    let trend = m
        .tags
        .iter()
        .find_map(|t| Trend::from_tag(t))
        .unwrap_or(Trend::Stable);

    let last_triggered = m.tags.iter().find_map(|t| {
        t.strip_prefix("last_triggered:")
            .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
            .map(|dt| dt.with_timezone(&Utc))
    });

    Ok(BehavioralRule {
        id: m.id,
        text: m.content,
        score: m.importance,
        trend,
        source,
        created_at: m.created_at,
        last_triggered,
    })
}

// ── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::MemoryGraph;
    use uuid::Uuid;

    fn make_engine() -> (MemoryGraph, ()) {
        let graph = MemoryGraph::in_memory().expect("in-memory graph should succeed");
        (graph, ())
    }

    #[test]
    fn add_and_get_rule() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let rule = engine
            .add_rule(
                "Do not modify files without asking",
                RuleSource::UserDefined,
            )
            .expect("add_rule should succeed");

        assert_eq!(rule.text, "Do not modify files without asking");
        assert!((rule.score - 50.0).abs() < f64::EPSILON);
        assert_eq!(rule.source, RuleSource::UserDefined);
        assert_eq!(rule.trend, Trend::Stable);

        let all = engine.get_rules_sorted().expect("get_rules_sorted");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, rule.id);
    }

    #[test]
    fn remove_rule() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let rule = engine
            .add_rule("temp rule", RuleSource::SystemDefault)
            .expect("add");
        engine.remove_rule(&rule.id).expect("remove");

        let all = engine.get_rules_sorted().expect("list");
        assert!(all.is_empty());
    }

    #[test]
    fn remove_nonexistent_fails() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);
        let err = engine.remove_rule("no-such-id");
        assert!(err.is_err());
    }

    #[test]
    fn reinforce_increases_score() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let rule = engine
            .add_rule("be polite", RuleSource::CorrectionDerived)
            .expect("add");
        let reinforced = engine.reinforce_rule(&rule.id).expect("reinforce");

        assert!((reinforced.score - 55.0).abs() < f64::EPSILON);
        assert!(reinforced.last_triggered.is_some());
    }

    #[test]
    fn reinforce_clamps_at_100() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let rule = engine
            .add_rule("max rule", RuleSource::UserDefined)
            .expect("add");

        // Set score close to max.
        graph.update_importance(&rule.id, 98.0).expect("set score");

        let reinforced = engine.reinforce_rule(&rule.id).expect("reinforce");
        assert!((reinforced.score - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn decay_reduces_scores() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        engine
            .add_rule("rule a", RuleSource::SystemDefault)
            .expect("add");
        engine
            .add_rule("rule b", RuleSource::SystemDefault)
            .expect("add");

        engine.decay_scores(10.0).expect("decay");

        let rules = engine.get_rules_sorted().expect("list");
        for r in &rules {
            assert!((r.score - 40.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn decay_clamps_at_zero() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let rule = engine
            .add_rule("low", RuleSource::SystemDefault)
            .expect("add");
        graph.update_importance(&rule.id, 3.0).expect("set");

        engine.decay_scores(10.0).expect("decay");

        let rules = engine.get_rules_sorted().expect("list");
        assert!((rules[0].score).abs() < f64::EPSILON);
    }

    #[test]
    fn sorting_by_score_descending() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let r1 = engine
            .add_rule("low priority", RuleSource::SystemDefault)
            .expect("add");
        let r2 = engine
            .add_rule("high priority", RuleSource::UserDefined)
            .expect("add");

        graph.update_importance(&r1.id, 20.0).expect("set");
        graph.update_importance(&r2.id, 80.0).expect("set");

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, r2.id);
        assert_eq!(rules[1].id, r1.id);
    }

    #[test]
    fn trend_calculation() {
        let rule_high = BehavioralRule {
            id: Uuid::new_v4().to_string(),
            text: "high".into(),
            score: 75.0,
            trend: Trend::Stable,
            source: RuleSource::SystemDefault,
            created_at: Utc::now(),
            last_triggered: None,
        };
        assert_eq!(RulesEngine::calculate_trend(&rule_high), Trend::Rising);

        let rule_low = BehavioralRule {
            id: Uuid::new_v4().to_string(),
            text: "low".into(),
            score: 25.0,
            trend: Trend::Stable,
            source: RuleSource::SystemDefault,
            created_at: Utc::now(),
            last_triggered: None,
        };
        assert_eq!(RulesEngine::calculate_trend(&rule_low), Trend::Declining);

        let rule_mid = BehavioralRule {
            id: Uuid::new_v4().to_string(),
            text: "mid".into(),
            score: 50.0,
            trend: Trend::Stable,
            source: RuleSource::SystemDefault,
            created_at: Utc::now(),
            last_triggered: None,
        };
        assert_eq!(RulesEngine::calculate_trend(&rule_mid), Trend::Stable);
    }

    #[test]
    fn format_for_prompt_output() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let r1 = engine
            .add_rule("Ask before modifying", RuleSource::UserDefined)
            .expect("add");
        let r2 = engine
            .add_rule("Explain reasoning", RuleSource::SystemDefault)
            .expect("add");

        graph.update_importance(&r1.id, 85.0).expect("set");
        graph.update_importance(&r2.id, 45.0).expect("set");

        let output = engine.format_for_prompt().expect("format");
        assert!(output.starts_with("<behavioral_rules>"));
        assert!(output.ends_with("</behavioral_rules>"));
        assert!(output.contains("[score: 85.0 up]"));
        assert!(output.contains("[score: 45.0 stable]"));
        // Higher score should come first.
        let pos_85 = output.find("85.0").expect("contains 85");
        let pos_45 = output.find("45.0").expect("contains 45");
        assert!(pos_85 < pos_45);
    }

    #[test]
    fn format_empty_returns_empty_string() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);
        let output = engine.format_for_prompt().expect("format");
        assert!(output.is_empty());
    }

    #[test]
    fn update_rule_text() {
        let (graph, _) = make_engine();
        let engine = RulesEngine::new(&graph);

        let rule = engine
            .add_rule("old text", RuleSource::UserDefined)
            .expect("add");
        engine.update_rule(&rule.id, "new text").expect("update");

        let rules = engine.get_rules_sorted().expect("list");
        assert_eq!(rules[0].text, "new text");
    }
}
