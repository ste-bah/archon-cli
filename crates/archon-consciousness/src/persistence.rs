//! Cross-session personality persistence.
//!
//! Stores and loads [`PersonalitySnapshot`] memories in the CozoDB memory
//! graph via the [`MemoryTrait`] interface. Provides trend computation and
//! human-readable session briefings.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use archon_memory::MemoryTrait;
use archon_memory::types::{MemoryType, SearchFilter};

use crate::inner_voice::InnerVoiceSnapshot;

// ── error type ──────────────────────────────────────────────

/// Errors produced by the persistence subsystem.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("memory error: {0}")]
    Memory(#[from] archon_memory::MemoryError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ── types ───────────────────────────────────────────────────

/// Full session-end state dump stored as a PersonalitySnapshot memory node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalitySnapshot {
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub inner_voice: InnerVoiceSnapshot,
    pub rule_scores: Vec<RuleScoreEntry>,
    pub stats: SessionStats,
}

/// A single rule's score at session end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleScoreEntry {
    pub rule_id: String,
    pub rule_text: String,
    pub score: f64,
}

/// Summary statistics for a single session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub total_turns: u32,
    pub total_corrections: u32,
    pub total_tool_calls: u32,
    pub total_tool_failures: u32,
    pub confidence_start: f32,
    pub confidence_end: f32,
    pub energy_end: f32,
    pub top_struggles: Vec<String>,
    pub top_successes: Vec<String>,
    pub duration_secs: u64,
}

/// Computed trends across recent sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityTrends {
    pub avg_confidence_end: f32,
    pub avg_corrections_per_session: f32,
    pub correction_trend: TrendDirection,
    pub persistent_struggles: Vec<(String, u32)>,
    pub reliable_successes: Vec<(String, u32)>,
    pub total_sessions: u32,
}

/// Direction of a measured trend.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TrendDirection {
    Rising,
    Falling,
    Stable,
}

// ── helpers ─────────────────────────────────────────────────

/// Search the memory graph for all PersonalitySnapshot memories.
fn search_snapshots(
    graph: &dyn MemoryTrait,
) -> Result<Vec<archon_memory::types::Memory>, PersistenceError> {
    let filter = SearchFilter {
        memory_type: Some(MemoryType::PersonalitySnapshot),
        ..SearchFilter::default()
    };
    let memories = graph.search_memories(&filter)?;
    Ok(memories)
}

/// Deserialize a [`PersonalitySnapshot`] from a memory's content field.
/// Returns `None` on failure after logging a warning.
fn try_deserialize(content: &str) -> Option<PersonalitySnapshot> {
    match serde_json::from_str::<PersonalitySnapshot>(content) {
        Ok(snap) => Some(snap),
        Err(e) => {
            warn!(
                error = %e,
                "failed to deserialize PersonalitySnapshot; skipping"
            );
            None
        }
    }
}

// ── public API ──────────────────────────────────────────────

/// Persist a [`PersonalitySnapshot`] to the memory graph.
///
/// The snapshot is serialized to JSON and stored as a
/// [`MemoryType::PersonalitySnapshot`] memory node with importance 90.0.
///
/// Returns the memory ID assigned by the graph.
pub fn save_snapshot(
    graph: &dyn MemoryTrait,
    snapshot: &PersonalitySnapshot,
) -> Result<String, PersistenceError> {
    let content = serde_json::to_string(snapshot)?;
    let title = format!("personality_snapshot:{}", snapshot.session_id);
    let tags = vec![
        "personality_snapshot".to_string(),
        format!("session:{}", snapshot.session_id),
    ];

    let id = graph.store_memory(
        &content,
        &title,
        MemoryType::PersonalitySnapshot,
        90.0,
        &tags,
        "personality_persistence",
        "",
    )?;

    Ok(id)
}

/// Load the most recent [`PersonalitySnapshot`] from the memory graph.
///
/// Returns `None` if no snapshots exist or if the newest snapshot fails
/// deserialization (graceful degradation with a `tracing::warn`).
pub fn load_latest_snapshot(
    graph: &dyn MemoryTrait,
) -> Result<Option<PersonalitySnapshot>, PersistenceError> {
    let mut memories = search_snapshots(graph)?;
    if memories.is_empty() {
        return Ok(None);
    }

    // Sort by created_at descending (newest first).
    memories.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(try_deserialize(&memories[0].content))
}

/// Prune old snapshots, keeping at most `limit` newest entries.
///
/// Returns the number of snapshots deleted.
pub fn prune_snapshots(graph: &dyn MemoryTrait, limit: u32) -> Result<usize, PersistenceError> {
    let mut memories = search_snapshots(graph)?;
    let count = memories.len();
    let limit = limit as usize;

    if count <= limit {
        return Ok(0);
    }

    // Sort by created_at ascending (oldest first).
    memories.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let to_delete = count - limit;
    let mut deleted = 0;

    for mem in memories.iter().take(to_delete) {
        match graph.delete_memory(&mem.id) {
            Ok(()) => deleted += 1,
            Err(e) => {
                warn!(id = %mem.id, error = %e, "failed to delete snapshot during prune");
            }
        }
    }

    Ok(deleted)
}

/// Compute personality trends across the most recent `window` sessions.
///
/// If no snapshots exist, returns zeroed defaults.
pub fn compute_trends(
    graph: &dyn MemoryTrait,
    window: usize,
) -> Result<PersonalityTrends, PersistenceError> {
    let mut memories = search_snapshots(graph)?;

    if memories.is_empty() {
        return Ok(PersonalityTrends {
            avg_confidence_end: 0.0,
            avg_corrections_per_session: 0.0,
            correction_trend: TrendDirection::Stable,
            persistent_struggles: Vec::new(),
            reliable_successes: Vec::new(),
            total_sessions: 0,
        });
    }

    // Sort by created_at descending, take most recent `window`.
    memories.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    memories.truncate(window);

    // Deserialize all snapshots (skip failures).
    let snapshots: Vec<PersonalitySnapshot> = memories
        .iter()
        .filter_map(|m| try_deserialize(&m.content))
        .collect();

    let n = snapshots.len();
    if n == 0 {
        return Ok(PersonalityTrends {
            avg_confidence_end: 0.0,
            avg_corrections_per_session: 0.0,
            correction_trend: TrendDirection::Stable,
            persistent_struggles: Vec::new(),
            reliable_successes: Vec::new(),
            total_sessions: 0,
        });
    }

    // Average confidence end.
    let avg_confidence_end: f32 = snapshots
        .iter()
        .map(|s| s.stats.confidence_end)
        .sum::<f32>()
        / n as f32;

    // Average corrections per session.
    let total_corrections: u32 = snapshots.iter().map(|s| s.stats.total_corrections).sum();
    let avg_corrections_per_session = total_corrections as f32 / n as f32;

    // Correction trend: compare first half vs second half.
    // Snapshots are newest-first, so reverse for chronological order.
    let chronological: Vec<&PersonalitySnapshot> = snapshots.iter().rev().collect();
    let correction_trend = if n < 2 {
        TrendDirection::Stable
    } else {
        let mid = n / 2;
        let first_half = &chronological[..mid];
        let second_half = &chronological[mid..];

        let first_avg = if first_half.is_empty() {
            0.0
        } else {
            first_half
                .iter()
                .map(|s| s.stats.total_corrections as f64)
                .sum::<f64>()
                / first_half.len() as f64
        };

        let second_avg = if second_half.is_empty() {
            0.0
        } else {
            second_half
                .iter()
                .map(|s| s.stats.total_corrections as f64)
                .sum::<f64>()
                / second_half.len() as f64
        };

        if second_avg > first_avg * 1.1 {
            TrendDirection::Rising
        } else if second_avg < first_avg * 0.9 {
            TrendDirection::Falling
        } else {
            TrendDirection::Stable
        }
    };

    // Persistent struggles: count how many sessions each struggle appears in.
    let mut struggle_counts: HashMap<String, u32> = HashMap::new();
    for snap in &snapshots {
        for s in &snap.stats.top_struggles {
            *struggle_counts.entry(s.clone()).or_insert(0) += 1;
        }
    }
    let mut persistent_struggles: Vec<(String, u32)> = struggle_counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .collect();
    persistent_struggles.sort_by(|a, b| b.1.cmp(&a.1));

    // Reliable successes: same logic.
    let mut success_counts: HashMap<String, u32> = HashMap::new();
    for snap in &snapshots {
        for s in &snap.stats.top_successes {
            *success_counts.entry(s.clone()).or_insert(0) += 1;
        }
    }
    let mut reliable_successes: Vec<(String, u32)> = success_counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .collect();
    reliable_successes.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(PersonalityTrends {
        avg_confidence_end,
        avg_corrections_per_session,
        correction_trend,
        persistent_struggles,
        reliable_successes,
        total_sessions: n as u32,
    })
}

/// Generate a human-readable personality briefing from trends and the
/// most recent snapshot.
///
/// The output is wrapped in `<personality_briefing>` XML tags suitable
/// for injection into a system prompt.
pub fn generate_briefing(trends: &PersonalityTrends, last: &PersonalitySnapshot) -> String {
    let trend_word = match trends.correction_trend {
        TrendDirection::Rising => "rising (worsening)",
        TrendDirection::Falling => "falling (improving)",
        TrendDirection::Stable => "stable",
    };

    let struggles_str = if trends.persistent_struggles.is_empty() {
        "none".to_string()
    } else {
        trends
            .persistent_struggles
            .iter()
            .take(3)
            .map(|(area, count)| format!("{area} ({count} sessions)"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let successes_str = if trends.reliable_successes.is_empty() {
        "none".to_string()
    } else {
        trends
            .reliable_successes
            .iter()
            .take(3)
            .map(|(area, count)| format!("{area} ({count} sessions)"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let rules_str = if last.rule_scores.is_empty() {
        "none".to_string()
    } else {
        let mut sorted = last.rule_scores.clone();
        sorted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted
            .iter()
            .take(3)
            .map(|r| format!("\"{}\" (score: {:.0})", r.rule_text, r.score))
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "<personality_briefing>\n\
         Sessions: {} total\n\
         Last session: confidence {:.1} -> {:.1} ({} corrections)\n\
         Trend: correction rate {} over last {} sessions\n\
         Persistent struggles: {}\n\
         Reliable strengths: {}\n\
         Top reinforced rules: {}\n\
         </personality_briefing>",
        trends.total_sessions,
        last.stats.confidence_start,
        last.stats.confidence_end,
        last.stats.total_corrections,
        trend_word,
        trends.total_sessions,
        struggles_str,
        successes_str,
        rules_str,
    )
}
