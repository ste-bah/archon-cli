//! Memory Garden — autonomous memory consolidation engine.
//!
//! Deduplicates, prunes stale memories, decays importance, merges fragments,
//! and generates session briefings.

use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

/// Memory types that are safe to prune, decay, merge, and deduplicate.
const PRUNEABLE_TYPES: [MemoryType; 5] = [
    MemoryType::Fact,
    MemoryType::Decision,
    MemoryType::Correction,
    MemoryType::Pattern,
    MemoryType::Preference,
];

/// Configuration for the memory garden consolidation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenConfig {
    pub auto_consolidate: bool,
    pub min_hours_between_runs: u32,
    pub dedup_similarity_threshold: f32,
    pub staleness_days: u32,
    pub staleness_importance_floor: f64,
    pub importance_decay_per_day: f64,
    pub max_memories: usize,
    pub briefing_limit: usize,
}

impl Default for GardenConfig {
    fn default() -> Self {
        Self {
            auto_consolidate: true,
            min_hours_between_runs: 24,
            dedup_similarity_threshold: 0.92,
            staleness_days: 30,
            staleness_importance_floor: 0.3,
            importance_decay_per_day: 0.01,
            max_memories: 5000,
            briefing_limit: 15,
        }
    }
}

/// Results of a consolidation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenReport {
    pub duplicates_merged: usize,
    pub stale_pruned: usize,
    pub importance_decayed: usize,
    pub fragments_merged: usize,
    pub overflow_pruned: usize,
    pub total_memories_before: usize,
    pub total_memories_after: usize,
    pub duration_ms: u64,
}

/// All memory types for stats enumeration.
const ALL_TYPES: [MemoryType; 7] = [
    MemoryType::Fact,
    MemoryType::Decision,
    MemoryType::Correction,
    MemoryType::Pattern,
    MemoryType::Preference,
    MemoryType::Rule,
    MemoryType::PersonalitySnapshot,
];

impl GardenReport {
    /// Format the consolidation report as a human-readable summary.
    pub fn format(&self) -> String {
        let mut out = String::new();
        out.push_str("Memory Garden — Consolidation Complete\n");
        out.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
        out.push_str(&format!(
            "  Duplicates merged:    {}\n",
            self.duplicates_merged
        ));
        out.push_str(&format!("  Stale pruned:         {}\n", self.stale_pruned));
        out.push_str(&format!(
            "  Importance decayed:   {}\n",
            self.importance_decayed
        ));
        out.push_str(&format!(
            "  Fragments merged:     {}\n",
            self.fragments_merged
        ));
        out.push_str(&format!(
            "  Overflow pruned:      {}\n",
            self.overflow_pruned
        ));
        out.push_str("  ──────────────────────────────\n");
        out.push_str(&format!(
            "  Before: {} memories\n",
            self.total_memories_before
        ));
        out.push_str(&format!(
            "  After:  {} memories\n",
            self.total_memories_after
        ));
        out.push_str(&format!("  Duration: {}ms\n", self.duration_ms));
        out
    }
}

/// Format memory garden statistics: type distribution, staleness, and top-N by importance.
pub fn format_garden_stats(graph: &dyn MemoryTrait, top_n: usize) -> Result<String, MemoryError> {
    let total = graph.memory_count()?;

    // Count by type.
    let mut type_rows: Vec<(MemoryType, usize)> = Vec::new();
    for mt in &ALL_TYPES {
        let count = get_memories_by_type(graph, *mt)?.len();
        if count > 0 {
            type_rows.push((*mt, count));
        }
    }

    // Staleness distribution — gather all memories.
    let all = graph.list_recent(total.max(1))?;
    let now = Utc::now();
    let mut bucket_7 = 0usize;
    let mut bucket_14 = 0usize;
    let mut bucket_30 = 0usize;
    let mut bucket_60 = 0usize;
    let mut bucket_over_60 = 0usize;
    for mem in &all {
        let accessed = mem.last_accessed.unwrap_or(mem.created_at);
        let days = (now - accessed).num_days();
        if days < 7 {
            bucket_7 += 1;
        } else if days < 14 {
            bucket_14 += 1;
        } else if days < 30 {
            bucket_30 += 1;
        } else if days < 60 {
            bucket_60 += 1;
        } else {
            bucket_over_60 += 1;
        }
    }

    // Top-N by importance.
    let mut sorted = all;
    sorted.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted.truncate(top_n);

    // Format output.
    let mut out = String::new();
    out.push_str("Memory Garden — Statistics\n");
    out.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    out.push_str(&format!("Total memories: {total}\n\n"));

    out.push_str("By type:\n");
    for (mt, count) in &type_rows {
        out.push_str(&format!("  {:<20} {}\n", format!("{mt}:"), count));
    }

    out.push_str("\nStaleness:\n");
    out.push_str(&format!("  < 7 days:          {bucket_7:>5}\n"));
    out.push_str(&format!("  7-14 days:         {bucket_14:>5}\n"));
    out.push_str(&format!("  14-30 days:        {bucket_30:>5}\n"));
    out.push_str(&format!("  30-60 days:        {bucket_60:>5}\n"));
    out.push_str(&format!("  > 60 days:         {bucket_over_60:>5}\n"));

    if !sorted.is_empty() {
        out.push_str(&format!("\nTop {} by importance:\n", sorted.len()));
        for m in &sorted {
            out.push_str(&format!(
                "  [{:.2}] [{}] {}\n",
                m.importance,
                m.memory_type,
                truncate_content(&m.content, 60),
            ));
        }
    }

    Ok(out)
}

/// Truncate content to max_len chars, appending "..." if truncated.
fn truncate_content(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ── public API ───────────────────────────────────────────────

/// Run a full consolidation pass across all six phases.
pub fn consolidate(
    graph: &dyn MemoryTrait,
    config: &GardenConfig,
) -> Result<GardenReport, MemoryError> {
    let start = Instant::now();
    let total_before = graph.memory_count()?;

    let importance_decayed = phase_importance_decay(graph, config.importance_decay_per_day)?;
    info!(importance_decayed, "phase 1: importance decay complete");

    let stale_pruned = phase_staleness_prune(
        graph,
        config.staleness_days,
        config.staleness_importance_floor,
    )?;
    info!(stale_pruned, "phase 2: staleness prune complete");

    let duplicates_merged = phase_dedup(graph, config.dedup_similarity_threshold)?;
    info!(duplicates_merged, "phase 3: deduplication complete");

    let fragments_merged = phase_fragment_merge(graph)?;
    info!(fragments_merged, "phase 4: fragment merge complete");

    let overflow_pruned = phase_overflow_prune(graph, config.max_memories)?;
    info!(overflow_pruned, "phase 5: overflow prune complete");

    phase_record_timestamp(graph)?;
    info!("phase 6: timestamp recorded");

    let total_after = graph.memory_count()?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let report = GardenReport {
        duplicates_merged,
        stale_pruned,
        importance_decayed,
        fragments_merged,
        overflow_pruned,
        total_memories_before: total_before,
        total_memories_after: total_after,
        duration_ms,
    };
    info!(?report, "consolidation complete");
    Ok(report)
}

/// Check whether enough time has elapsed since the last consolidation.
pub fn should_auto_consolidate(
    graph: &dyn MemoryTrait,
    min_hours: u32,
) -> Result<bool, MemoryError> {
    let filter = SearchFilter {
        tags: vec!["garden:last_run".into()],
        require_all_tags: true,
        ..SearchFilter::default()
    };
    let results = graph.search_memories(&filter)?;
    let Some(mem) = results.first() else {
        return Ok(true);
    };
    let Ok(last_run) = mem.content.parse::<DateTime<Utc>>() else {
        warn!("could not parse garden:last_run timestamp, re-running");
        return Ok(true);
    };
    let hours_elapsed = (Utc::now() - last_run).num_hours();
    Ok(hours_elapsed >= i64::from(min_hours))
}

/// Generate a human-readable session briefing from the memory graph.
pub fn generate_briefing(graph: &dyn MemoryTrait, limit: usize) -> Result<String, MemoryError> {
    let total = graph.memory_count()?;

    // Count by type.
    let mut type_counts: Vec<(MemoryType, usize)> = Vec::new();
    for mt in &PRUNEABLE_TYPES {
        let count = get_memories_by_type(graph, *mt)?.len();
        type_counts.push((*mt, count));
    }
    let rule_count = get_memories_by_type(graph, MemoryType::Rule)?.len();
    type_counts.push((MemoryType::Rule, rule_count));
    let snap_count = get_memories_by_type(graph, MemoryType::PersonalitySnapshot)?.len();
    type_counts.push((MemoryType::PersonalitySnapshot, snap_count));

    // Top-N by importance.
    let mut all = graph.list_recent(total.max(1))?;
    all.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all.truncate(limit);

    // Last garden run.
    let last_run_text = {
        let filter = SearchFilter {
            tags: vec!["garden:last_run".into()],
            require_all_tags: true,
            ..SearchFilter::default()
        };
        let results = graph.search_memories(&filter)?;
        match results
            .first()
            .and_then(|m| m.content.parse::<DateTime<Utc>>().ok())
        {
            Some(ts) => {
                let hours = (Utc::now() - ts).num_hours();
                if hours < 1 {
                    "less than an hour ago".to_string()
                } else if hours < 24 {
                    format!("{hours} hours ago")
                } else {
                    format!("{} days ago", hours / 24)
                }
            }
            None => "never".to_string(),
        }
    };

    // Format type summary.
    let type_summary: Vec<String> = type_counts
        .iter()
        .filter(|(_, c)| *c > 0)
        .map(|(t, c)| format!("{c} {t}s"))
        .collect();

    let mut out = String::new();
    out.push_str("<memory_briefing>\n");
    out.push_str(&format!(
        "Memory graph: {total} memories ({})\n",
        type_summary.join(", ")
    ));
    out.push_str(&format!("Last consolidated: {last_run_text}\n"));
    if !all.is_empty() {
        out.push_str("Key memories:\n");
        for m in &all {
            out.push_str(&format!(
                "- [{}] {} (importance: {:.2})\n",
                m.memory_type, m.content, m.importance
            ));
        }
    }
    out.push_str("</memory_briefing>");
    Ok(out)
}

// ── phase implementations (private) ──────────────────────────

fn phase_importance_decay(
    graph: &dyn MemoryTrait,
    decay_per_day: f64,
) -> Result<usize, MemoryError> {
    let now = Utc::now();
    let mut count = 0;
    for mt in &PRUNEABLE_TYPES {
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in memories {
            let accessed = mem.last_accessed.unwrap_or(mem.created_at);
            let days = (now - accessed).num_days();
            if days < 1 {
                continue;
            }
            let new_imp = (mem.importance - (days as f64 * decay_per_day)).max(0.0);
            if new_imp < mem.importance {
                if let Err(e) = graph.update_importance(&mem.id, new_imp) {
                    warn!(id = %mem.id, error = %e, "failed to decay importance");
                } else {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn phase_staleness_prune(
    graph: &dyn MemoryTrait,
    staleness_days: u32,
    importance_floor: f64,
) -> Result<usize, MemoryError> {
    let now = Utc::now();
    let threshold = chrono::Duration::days(i64::from(staleness_days));
    let mut count = 0;
    for mt in &PRUNEABLE_TYPES {
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in memories {
            let accessed = mem.last_accessed.unwrap_or(mem.created_at);
            if (now - accessed) > threshold && mem.importance < importance_floor {
                if let Err(e) = graph.delete_memory(&mem.id) {
                    warn!(id = %mem.id, error = %e, "failed to prune stale memory");
                } else {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn phase_dedup(graph: &dyn MemoryTrait, similarity_threshold: f32) -> Result<usize, MemoryError> {
    let mut merged = 0;
    let mut deleted_ids: HashSet<String> = HashSet::new();
    for mt in &PRUNEABLE_TYPES {
        if merged >= 50 {
            break;
        }
        let memories = get_memories_by_type(graph, *mt)?;
        for i in 0..memories.len() {
            if merged >= 50 {
                break;
            }
            if deleted_ids.contains(&memories[i].id) {
                continue;
            }
            let words_i = word_set(&memories[i].content);
            for j in (i + 1)..memories.len() {
                if merged >= 50 {
                    break;
                }
                if deleted_ids.contains(&memories[j].id) {
                    continue;
                }
                let words_j = word_set(&memories[j].content);
                if jaccard(&words_i, &words_j) <= f64::from(similarity_threshold) {
                    continue;
                }
                // Keep higher importance; if tied, keep newer.
                let (survivor, victim) = if memories[i].importance > memories[j].importance
                    || (memories[i].importance == memories[j].importance
                        && memories[i].created_at >= memories[j].created_at)
                {
                    (&memories[i], &memories[j])
                } else {
                    (&memories[j], &memories[i])
                };
                // Merge tags from victim into survivor.
                let mut merged_tags: Vec<String> = survivor.tags.clone();
                for t in &victim.tags {
                    if !merged_tags.contains(t) {
                        merged_tags.push(t.clone());
                    }
                }
                if let Err(e) = graph.update_memory(&survivor.id, None, Some(&merged_tags)) {
                    warn!(id = %survivor.id, error = %e, "failed to merge tags into survivor");
                }
                if let Err(e) = graph.create_relationship(
                    &survivor.id,
                    &victim.id,
                    RelType::Supersedes,
                    None,
                    1.0,
                ) {
                    warn!(id = %survivor.id, error = %e, "failed to create supersedes relationship");
                }
                if let Err(e) = graph.delete_memory(&victim.id) {
                    warn!(id = %victim.id, error = %e, "failed to delete duplicate");
                } else {
                    deleted_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }
    Ok(merged)
}

fn phase_fragment_merge(graph: &dyn MemoryTrait) -> Result<usize, MemoryError> {
    let mut merged = 0;
    let mut deleted_ids: HashSet<String> = HashSet::new();
    for mt in &PRUNEABLE_TYPES {
        if merged >= 20 {
            break;
        }
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in &memories {
            if merged >= 20 {
                break;
            }
            if deleted_ids.contains(&mem.id) {
                continue;
            }
            let related = match graph.get_related_memories(&mem.id, 1) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for rel in &related {
                if merged >= 20 {
                    break;
                }
                if deleted_ids.contains(&rel.id) {
                    continue;
                }
                if rel.memory_type != mem.memory_type {
                    continue;
                }
                if mem.content.len() + rel.content.len() + 3 > 500 {
                    continue;
                }
                // Keep the one with higher importance.
                let (survivor, victim) = if mem.importance >= rel.importance {
                    (mem, rel)
                } else {
                    (rel, mem)
                };
                let combined = format!("{} | {}", survivor.content, victim.content);
                let mut merged_tags: Vec<String> = survivor.tags.clone();
                for t in &victim.tags {
                    if !merged_tags.contains(t) {
                        merged_tags.push(t.clone());
                    }
                }
                if let Err(e) =
                    graph.update_memory(&survivor.id, Some(&combined), Some(&merged_tags))
                {
                    warn!(id = %survivor.id, error = %e, "failed to merge fragment into survivor");
                }
                if let Err(e) = graph.delete_memory(&victim.id) {
                    warn!(id = %victim.id, error = %e, "failed to delete fragment");
                } else {
                    deleted_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }
    Ok(merged)
}

fn phase_overflow_prune(
    graph: &dyn MemoryTrait,
    max_memories: usize,
) -> Result<usize, MemoryError> {
    let total = graph.memory_count()?;
    if total <= max_memories {
        return Ok(0);
    }
    let to_remove = total - max_memories;
    // Gather all pruneable memories, sort by importance ASC then created_at ASC.
    let mut candidates: Vec<Memory> = Vec::new();
    for mt in &PRUNEABLE_TYPES {
        candidates.extend(get_memories_by_type(graph, *mt)?);
    }
    candidates.sort_by(|a, b| {
        a.importance
            .partial_cmp(&b.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    let mut count = 0;
    for mem in candidates.iter().take(to_remove) {
        if let Err(e) = graph.delete_memory(&mem.id) {
            warn!(id = %mem.id, error = %e, "failed to prune overflow memory");
        } else {
            count += 1;
        }
    }
    Ok(count)
}

fn phase_record_timestamp(graph: &dyn MemoryTrait) -> Result<(), MemoryError> {
    let now_str = Utc::now().to_rfc3339();
    let filter = SearchFilter {
        tags: vec!["garden:last_run".into()],
        require_all_tags: true,
        ..SearchFilter::default()
    };
    let results = graph.search_memories(&filter)?;
    if let Some(existing) = results.first() {
        graph.update_memory(&existing.id, Some(&now_str), None)?;
    } else {
        graph.store_memory(
            &now_str,
            "garden:last_run",
            MemoryType::Fact,
            1.0,
            &["garden:last_run".into()],
            "garden",
            "",
        )?;
    }
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────

fn get_memories_by_type(
    graph: &dyn MemoryTrait,
    memory_type: MemoryType,
) -> Result<Vec<Memory>, MemoryError> {
    let filter = SearchFilter {
        memory_type: Some(memory_type),
        ..SearchFilter::default()
    };
    graph.search_memories(&filter)
}

/// Build a set of normalised words from content for Jaccard comparison.
fn word_set(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Jaccard similarity between two word sets.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}
