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

    #[error("invalid rule score: {0}")]
    InvalidScore(f64),

    #[error("rule identity collision for {id}: {reason}")]
    IdentityCollision { id: String, reason: String },

    #[error("memory graph error: {0}")]
    Memory(#[from] archon_memory::MemoryError),
}

// ── engine ───────────────────────────────────────────────────

/// Maximum behavioral rules included in the system prompt.
pub const MAX_PROMPT_RULES: usize = 10;

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

    /// Add a rule at a deterministic ID, or return the matching existing rule.
    pub fn add_rule_with_id(
        &self,
        id: &str,
        text: &str,
        source: RuleSource,
    ) -> Result<BehavioralRule, RulesError> {
        let tags = vec![source.as_tag(), Trend::Stable.as_tag()];
        let memory = self.graph.store_memory_with_id(
            id,
            text,
            "",
            MemoryType::Rule,
            50.0,
            &tags,
            "rules_engine",
            "",
        )?;
        validate_rule_identity(&memory, id, source)?;
        memory_to_rule(memory)
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

        rules.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        Ok(rules)
    }

    /// Increment a rule's score by 5.0 and record a new trigger event.
    pub fn reinforce_rule(&self, id: &str) -> Result<BehavioralRule, RulesError> {
        let provenance_id = format!("rule-reinforcement:{}", uuid::Uuid::new_v4());
        let updated = self.apply_score_delta(id, 5.0, &provenance_id)?;
        self.record_last_triggered(&updated.id)?;
        self.rule_from_graph(&updated.id)
    }

    /// Decay all rule scores by `rate` once for this invocation.
    pub fn decay_scores(&self, rate: f64) -> Result<(), RulesError> {
        if !rate.is_finite() || rate < 0.0 {
            return Err(RulesError::InvalidScore(rate));
        }
        let run_id = uuid::Uuid::new_v4();
        for rule in self.get_rules_sorted()? {
            let provenance_id = format!("rule-decay:{run_id}:{}", rule.id);
            self.apply_score_delta(&rule.id, -rate, &provenance_id)?;
        }
        Ok(())
    }

    /// Increase a rule's score by an explicit amount for one correction event.
    pub fn boost_rule_by(
        &self,
        id: &str,
        increment: f64,
        provenance_id: &str,
    ) -> Result<BehavioralRule, RulesError> {
        self.apply_score_delta(id, increment, provenance_id)
    }

    /// Apply a score change against the persisted source of truth.
    pub fn apply_score_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<BehavioralRule, RulesError> {
        if !delta.is_finite() {
            return Err(RulesError::InvalidScore(delta));
        }
        let updated = self
            .graph
            .apply_importance_delta(id, delta, provenance_id)
            .map_err(|error| match error {
                archon_memory::MemoryError::NotFound(_) => RulesError::NotFound(id.to_string()),
                other => RulesError::Memory(other),
            })?;
        memory_to_rule(updated)
    }

    fn record_last_triggered(&self, id: &str) -> Result<(), RulesError> {
        let memory = self.graph.get_memory(id)?;
        let mut tags: Vec<String> = memory
            .tags
            .into_iter()
            .filter(|tag| !tag.starts_with("last_triggered:"))
            .collect();
        tags.push(format!("last_triggered:{}", Utc::now().to_rfc3339()));
        self.graph.update_memory(id, None, Some(&tags))?;
        Ok(())
    }

    fn rule_from_graph(&self, id: &str) -> Result<BehavioralRule, RulesError> {
        memory_to_rule(self.graph.get_memory(id)?).map_err(|_| RulesError::NotFound(id.to_string()))
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

    /// Export current rule scores as a list of [`RuleScoreEntry`](crate::persistence::RuleScoreEntry).
    pub fn export_scores(&self) -> Result<Vec<crate::persistence::RuleScoreEntry>, RulesError> {
        let rules = self.get_rules_sorted()?;
        Ok(rules
            .into_iter()
            .map(|r| crate::persistence::RuleScoreEntry {
                rule_id: r.id,
                rule_text: r.text,
                score: r.score,
            })
            .collect())
    }

    /// Import rule scores from a previous session.
    ///
    /// For each entry, if a rule with matching text exists, its score is
    /// updated. Rules that no longer exist are silently skipped.
    pub fn import_scores(
        &self,
        scores: &[crate::persistence::RuleScoreEntry],
    ) -> Result<usize, RulesError> {
        let current_rules = self.get_rules_sorted()?;
        let mut imported = 0;

        for entry in scores {
            if !is_valid_rule_score(entry.score) {
                return Err(RulesError::InvalidScore(entry.score));
            }

            // Match by rule_id first, fall back to text match.
            let target = current_rules
                .iter()
                .find(|r| r.id == entry.rule_id)
                .or_else(|| current_rules.iter().find(|r| r.text == entry.rule_text));

            if let Some(rule) = target {
                let provenance_id = format!(
                    "rule-import:{}:{}:{}",
                    entry.rule_id, entry.rule_text, entry.score
                );
                let delta = entry.score - rule.score;
                self.apply_score_delta(&rule.id, delta, &provenance_id)?;
                imported += 1;
            }
        }

        Ok(imported)
    }

    /// Render the highest-priority rules into a block suitable for system-prompt
    /// injection.
    pub fn format_for_prompt(&self) -> Result<String, RulesError> {
        let rules = self.get_rules_sorted()?;
        if rules.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("<behavioral_rules>\n## Rules (sorted by priority)\n");
        for (i, r) in rules.iter().take(MAX_PROMPT_RULES).enumerate() {
            out.push_str(&format!(
                "{}. [score: {:.1} {}] {}\n",
                i + 1,
                r.score,
                r.trend.arrow(),
                r.text,
            ));
        }
        out.push_str("</behavioral_rules>");
        Ok(out)
    }
}

// ── helpers ──────────────────────────────────────────────────

fn is_valid_rule_score(score: f64) -> bool {
    score.is_finite() && (0.0..=100.0).contains(&score)
}

fn validate_rule_identity(
    memory: &archon_memory::Memory,
    id: &str,
    source: RuleSource,
) -> Result<(), RulesError> {
    let source_tag = source.as_tag();
    let source_tags: Vec<_> = memory
        .tags
        .iter()
        .filter(|tag| tag.starts_with("source:"))
        .collect();
    let trend_tags: Vec<_> = memory
        .tags
        .iter()
        .filter(|tag| tag.starts_with("trend:"))
        .collect();
    let rule = memory_to_rule(memory.clone())?;
    if memory.memory_type != MemoryType::Rule
        || memory.source_type != "rules_engine"
        || source_tags != [&source_tag]
        || trend_tags.len() != 1
        || Trend::from_tag(trend_tags[0]).is_none()
        || rule.source != source
    {
        return Err(RulesError::IdentityCollision {
            id: id.to_string(),
            reason: "stored rule source or required tags differ".to_string(),
        });
    }
    Ok(())
}

/// Convert a [`Memory`] into a [`BehavioralRule`].
fn memory_to_rule(m: archon_memory::Memory) -> Result<BehavioralRule, RulesError> {
    if !is_valid_rule_score(m.importance) {
        return Err(RulesError::InvalidScore(m.importance));
    }

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

// ── tests ────────────────────────────────────

#[cfg(test)]
#[path = "rules/tests.rs"]
mod tests;
