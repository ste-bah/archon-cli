//! Reporting: what consolidation says about itself, and what the graph says
//! about its own shape.
//!
//! Split out of `garden.rs` at the 500-line gate. The seam is direction of
//! travel -- nothing here writes to the graph, while everything left behind
//! exists to change it.

use chrono::{DateTime, Utc};

use super::{GardenReport, PRUNEABLE_TYPES, get_memories_by_type};
use crate::access::MemoryTrait;
use crate::types::{MemoryError, MemoryType, SearchFilter};

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

pub(super) const BRIEFING_MEMORY_MAX_CHARS: usize = 800;
pub(super) const BRIEFING_TOTAL_MAX_CHARS: usize = 16_000;

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
        // Stated only when it happened, and stated as a gap rather than a
        // count. A zero on the duplicates row above is the answer this pass
        // could not give.
        if self.semantic_pass_unavailable {
            out.push_str("  Semantic pass:        unavailable (no vector search)\n");
        }
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
                m.memory_type,
                truncate_content(&m.content, BRIEFING_MEMORY_MAX_CHARS),
                m.importance
            ));
        }
    }
    out.push_str("</memory_briefing>");
    Ok(cap_briefing(out))
}

/// Truncate content to max_len chars, appending "..." if truncated.
pub(super) fn truncate_content(s: &str, max_len: usize) -> String {
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

pub(super) fn cap_briefing(mut out: String) -> String {
    if out.len() <= BRIEFING_TOTAL_MAX_CHARS {
        return out;
    }

    let marker = "\n[briefing truncated]\n</memory_briefing>";
    let body_limit = BRIEFING_TOTAL_MAX_CHARS.saturating_sub(marker.len());
    let mut end = body_limit.min(out.len());
    while end > 0 && !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
    out.push_str(marker);
    out
}

#[cfg(test)]
#[path = "consolidate_tests.rs"]
mod consolidate_tests;
