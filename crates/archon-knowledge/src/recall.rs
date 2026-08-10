//! R7 — one recall contract over four stores that keep their own databases.
//!
//! Memory, docs, the knowledge graph and the code index each own a CozoDB and a
//! query dialect. This module defines the shape a caller sees and the rules for
//! merging four answers into one; it moves no data and creates no new physical
//! store. That is the explicit non-goal at
//! `docs/development/learning-roadmap-r1-r8-w5-w6.md` line 334.
//!
//! # The scores are NOT calibrated, and the type says so
//!
//! Every hit this module can currently produce carries
//! [`ScoreCalibration::UncalibratedRankOrder`]. That is not a formality: the R7
//! promotion gate (roadmap line 306) requires 500 replayable queries with
//! adjudicated relevant sources, at least 50 per source, and no such corpus
//! exists in this repository. Without it, any function that turned four stores'
//! raw scores into one comparable number would be a guess wearing a decimal
//! point.
//!
//! So the placeholder deliberately throws the raw scores away and keeps only
//! within-source rank — see [`normalize`]. Ordering inside one source is
//! preserved because the source earned it. Comparing magnitudes *across*
//! sources is not evidence and must not be reported as relevance. The raw value
//! survives on [`RecallHit::source_score`] so a future calibration has its
//! input, and so nobody has to re-run a query to find out what the store
//! actually said.
//!
//! # Why the adapter trait is synchronous
//!
//! The roadmap sketch writes `async fn recall`. It is sync here. Every store
//! behind it is blocking CozoDB work, this crate has no async runtime and
//! acquiring one for four blocking calls would buy nothing; and the latency
//! budget the slice actually asks for is enforced by *not waiting*, which
//! [`facade`] does with one detached thread per source and a per-source
//! deadline. An async signature would have implied cancellation this design
//! cannot deliver — a `tokio` task dropped at its deadline still leaves a Cozo
//! query running.
//!
//! # Where the four adapters live
//!
//! [`adapters::KnowledgeStoreAdapter`] is in-crate: the knowledge graph is this
//! crate's own store, so it costs no dependency. The other three go through the
//! narrow read-only ports in [`adapters`], implemented in the command layer over
//! the real `archon-memory`, `archon-docs` and `archon-leann` handles. Same
//! reason [`crate::traceability::CodeSearch`] is a port: this crate must not
//! acquire an edge onto three crates that drag in tokio, fastembed, RocksDB,
//! tree-sitter and an embedding provider, and a test must be able to drive the
//! merge rules with no store and no model at all.

pub mod adapters;
pub mod facade;
pub mod identity;
pub mod normalize;

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::Result;

pub use facade::{RecallResponse, SourceOutcome, SourceStatus, UnifiedRecall};
pub use identity::{ConflictKind, ConflictMember, RecallConflict};

/// Default per-source latency budget.
///
/// A policy choice, not a measurement: it is the point past which a recall is
/// no longer worth waiting for interactively. The R7 gate scores p95 latency
/// against "the slowest enabled source budget", so this number is the thing
/// that gate would be measured against — change it deliberately.
pub const DEFAULT_SOURCE_LATENCY_BUDGET: Duration = Duration::from_secs(5);

/// One of the four stores R7 unifies.
///
/// The ordering of this enum is load-bearing: it is the final, deterministic
/// tie-break when two hits normalize to the same score, so a replay of the same
/// query returns the same list in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallSource {
    /// `archon-memory` — episodic and semantic agent memory.
    Memory,
    /// `archon-docs` — ingested document chunks.
    Docs,
    /// `archon-knowledge` — the claim/entity/relation graph, this crate.
    Knowledge,
    /// `archon-leann` — the semantic code index.
    Code,
}

impl RecallSource {
    /// Every source, in the tie-break order above.
    pub const ALL: [RecallSource; 4] = [
        RecallSource::Memory,
        RecallSource::Docs,
        RecallSource::Knowledge,
        RecallSource::Code,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RecallSource::Memory => "memory",
            RecallSource::Docs => "docs",
            RecallSource::Knowledge => "knowledge",
            RecallSource::Code => "code",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        RecallSource::ALL
            .into_iter()
            .find(|source| source.as_str() == value)
    }
}

impl std::fmt::Display for RecallSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `pad`, not `write_str`: the CLI prints these in a fixed-width column,
        // and `write_str` silently ignores the format spec's width.
        f.pad(self.as_str())
    }
}

/// What one source is allowed to spend and to contribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBudget {
    pub source: RecallSource,
    /// Hard ceiling on hits this source may contribute to the merged answer.
    ///
    /// Enforced by [`facade::UnifiedRecall`] on the returned vector, not only
    /// passed to the adapter: a store that ignores its quota must not be able to
    /// crowd the others out.
    pub quota: usize,
    /// How long the merge will wait for this source before abandoning it.
    pub latency_budget: Duration,
}

/// Which sources may answer, how much each may contribute, and for how long.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePolicy {
    budgets: Vec<SourceBudget>,
}

impl SourcePolicy {
    /// Give each named source an equal share of `limit` and the same budget.
    ///
    /// Unfilled quota is deliberately NOT redistributed. Redistribution would
    /// have to happen after the fast sources have already answered, which hands
    /// the surplus to whoever replied first — precisely the starvation the
    /// quotas exist to prevent, re-introduced as a race.
    pub fn even_share(sources: &[RecallSource], limit: usize, latency_budget: Duration) -> Self {
        let share = if sources.is_empty() {
            0
        } else {
            limit.div_ceil(sources.len()).max(1)
        };
        Self {
            budgets: sources
                .iter()
                .map(|&source| SourceBudget {
                    source,
                    quota: share,
                    latency_budget,
                })
                .collect(),
        }
    }

    /// Build a policy from explicit per-source budgets, keeping the last entry
    /// for a repeated source so a caller can override a default.
    pub fn from_budgets(budgets: Vec<SourceBudget>) -> Self {
        let mut deduped: Vec<SourceBudget> = Vec::new();
        for budget in budgets {
            match deduped.iter_mut().find(|held| held.source == budget.source) {
                Some(held) => *held = budget,
                None => deduped.push(budget),
            }
        }
        Self { budgets: deduped }
    }

    pub fn budgets(&self) -> &[SourceBudget] {
        &self.budgets
    }

    pub fn budget_for(&self, source: RecallSource) -> Option<&SourceBudget> {
        self.budgets.iter().find(|budget| budget.source == source)
    }

    /// Whether this source may answer at all.
    pub fn allows(&self, source: RecallSource) -> bool {
        self.budget_for(source).is_some()
    }
}

impl Default for SourcePolicy {
    fn default() -> Self {
        Self::even_share(
            &RecallSource::ALL,
            RecallSource::ALL.len(),
            DEFAULT_SOURCE_LATENCY_BUDGET,
        )
    }
}

/// One recall request, identical for every source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallQuery {
    pub text: String,
    /// Ceiling on the merged answer, applied after dedupe.
    pub limit: usize,
    pub source_policy: SourcePolicy,
}

impl RecallQuery {
    /// A query over every source with even quotas and the default budget.
    pub fn new(text: impl Into<String>, limit: usize) -> Self {
        Self {
            text: text.into(),
            limit,
            source_policy: SourcePolicy::even_share(
                &RecallSource::ALL,
                limit,
                DEFAULT_SOURCE_LATENCY_BUDGET,
            ),
        }
    }

    /// How many hits `source` may contribute; `0` when the policy excludes it.
    pub fn quota_for(&self, source: RecallSource) -> usize {
        self.source_policy
            .budget_for(source)
            .map_or(0, |budget| budget.quota)
    }
}

/// How a [`RecallHit::normalized_score`] was produced.
///
/// This exists so a reader can never mistake the placeholder for a measurement.
/// See the module docs and [`normalize`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreCalibration {
    /// UNCALIBRATED. The score is a function of within-source rank alone and
    /// carries no cross-source meaning. Everything this crate can build today
    /// reports this.
    UncalibratedRankOrder,
    /// Reserved for a score fitted on an adjudicated corpus, identified so a
    /// result can be traced back to the evaluation that justified it.
    ///
    /// Nothing constructs this yet, and nothing should until the corpus in
    /// roadmap line 306 exists. It is declared so that "which calibration?" is a
    /// question the type can answer rather than one a reader has to research.
    Measured {
        corpus_id: String,
        corpus_version: String,
    },
}

impl ScoreCalibration {
    /// Whether this score may be compared across sources. Always false today.
    pub fn is_measured(&self) -> bool {
        matches!(self, ScoreCalibration::Measured { .. })
    }
}

/// Another hit that folded into this one during dedupe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HitRef {
    pub source: RecallSource,
    pub source_id: String,
}

/// One result, in the shape every source is mapped into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallHit {
    pub source: RecallSource,
    /// The store's own identifier: memory id, chunk id, `file:line`.
    pub source_id: String,
    pub content: String,
    /// Rank-derived placeholder in `(0, 1]`. Read [`calibration`] before using
    /// it for anything but ordering within one source.
    ///
    /// [`calibration`]: RecallHit::calibration
    pub normalized_score: f32,
    /// Always [`ScoreCalibration::UncalibratedRankOrder`] today.
    pub calibration: ScoreCalibration,
    /// What the store itself reported, untouched. `None` where the store
    /// reports only an order — `archon-memory`'s `recall_memories` does exactly
    /// that, which is on its own enough to rule out score-based fusion.
    pub source_score: Option<f64>,
    /// Zero-based position in that source's own answer.
    pub source_rank: usize,
    /// Stable references to the artifacts this hit came from, e.g.
    /// `doc:<document_id>`, `chunk:<chunk_id>`, `file:<path>`. Dedupe and
    /// conflict detection both key on these.
    pub provenance_refs: Vec<String>,
    /// `None` where the store keeps no per-record timestamp.
    ///
    /// Deviates from the roadmap sketch's non-optional field on purpose:
    /// document chunks carry no creation time, and stamping `now` would make
    /// every chunk hit look like the freshest thing in the answer.
    pub created_at: Option<DateTime<Utc>>,
    /// The store's own confidence, where it has one. Not a relevance score.
    pub confidence: Option<f32>,
    /// Hits merged into this one because they name the same content.
    pub duplicates: Vec<HitRef>,
    /// Indices into [`RecallResponse::conflicts`] this hit takes part in.
    pub conflicts: Vec<usize>,
}

impl RecallHit {
    /// A hit at `rank` within its source, scored by the uncalibrated
    /// placeholder. The only constructor, so no caller can invent a score.
    pub fn at_rank(
        source: RecallSource,
        source_id: impl Into<String>,
        content: impl Into<String>,
        rank: usize,
    ) -> Self {
        Self {
            source,
            source_id: source_id.into(),
            content: content.into(),
            normalized_score: normalize::uncalibrated_rank_score(rank),
            calibration: ScoreCalibration::UncalibratedRankOrder,
            source_score: None,
            source_rank: rank,
            provenance_refs: Vec::new(),
            created_at: None,
            confidence: None,
            duplicates: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    pub fn with_provenance(mut self, refs: impl IntoIterator<Item = String>) -> Self {
        self.provenance_refs = refs.into_iter().collect();
        self.provenance_refs.sort();
        self.provenance_refs.dedup();
        self
    }

    pub fn with_source_score(mut self, score: f64) -> Self {
        self.source_score = Some(score);
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

/// A read-only view of one store, mapped into [`RecallHit`].
///
/// Implementors must not write, index, or take a write lock: a recall runs on
/// the interactive path and the code index in particular holds the Cozo write
/// lock across a whole `multi_transaction` while indexing.
///
/// An implementation may return more than its quota; [`facade::UnifiedRecall`]
/// truncates. It may also return an error — that is a per-source outcome, not a
/// failed query.
pub trait RecallSourceAdapter: Send + Sync {
    fn source(&self) -> RecallSource;

    fn recall(&self, query: &RecallQuery) -> Result<Vec<RecallHit>>;
}

#[cfg(test)]
#[path = "recall/tests.rs"]
mod tests;
