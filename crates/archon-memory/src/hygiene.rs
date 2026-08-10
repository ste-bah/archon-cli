//! Repair for memories that were stored before ingest was bounded.
//!
//! The ingest caps and fingerprint dedupe in [`crate::extraction`] stop new
//! corruption, but they do nothing about what is already in the graph. A store
//! that accumulated pasted documents as `Rule` memories keeps rendering them
//! into the system prompt on every request until they are removed.
//!
//! This is deliberately a two-step, caller-driven repair rather than a
//! migration that runs at startup. Deleting memories is not reversible, the
//! selection rules are heuristics, and a user is entitled to see what is about
//! to go before it goes.

use std::collections::HashMap;

use crate::access::MemoryTrait;
use crate::extraction::{content_hash_tag, content_limit};
use crate::types::{Memory, MemoryError, MemoryType};

/// How much of a memory's content to show when reporting it.
const EXCERPT_CHARS: usize = 80;

/// One memory selected for removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedMemory {
    pub id: String,
    pub memory_type: MemoryType,
    /// Length in characters, so an oversized entry can be judged at a glance.
    pub length: usize,
    pub excerpt: String,
}

/// A set of memories with identical normalised content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// The copy that is kept.
    pub kept: PrunedMemory,
    /// The redundant copies.
    pub removed: Vec<PrunedMemory>,
}

/// What a prune would do, or did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Memories exceeding the ingest cap for their type.
    pub oversized: Vec<PrunedMemory>,
    /// Fingerprint-identical clusters, minus the copy being kept.
    pub duplicates: Vec<DuplicateGroup>,
    /// False for a plan, true once the deletions have been carried out.
    pub applied: bool,
}

impl PruneReport {
    /// Total memories this report would remove.
    pub fn removal_count(&self) -> usize {
        self.oversized.len()
            + self
                .duplicates
                .iter()
                .map(|group| group.removed.len())
                .sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.removal_count() == 0
    }

    /// Every id this report would remove.
    fn removal_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.oversized.iter().map(|m| m.id.clone()).collect();
        for group in &self.duplicates {
            ids.extend(group.removed.iter().map(|m| m.id.clone()));
        }
        ids
    }
}

/// A single-line preview of `content`.
///
/// Whitespace is collapsed rather than preserved: the entries most worth
/// reporting are pasted documents, and their newlines would break every row of
/// the report across several lines. Found by running the command against a real
/// store -- the fixtures were all single-line, so no test caught it.
fn excerpt(content: &str) -> String {
    let mut flattened = String::with_capacity(EXCERPT_CHARS);
    let mut chars = 0usize;
    let mut pending_space = false;
    for ch in content.trim().chars() {
        if ch.is_whitespace() {
            pending_space = !flattened.is_empty();
            continue;
        }
        if pending_space {
            flattened.push(' ');
            pending_space = false;
            chars += 1;
        }
        if chars >= EXCERPT_CHARS {
            return format!("{flattened}…");
        }
        flattened.push(ch);
        chars += 1;
    }
    flattened
}

fn describe(memory: &Memory) -> PrunedMemory {
    PrunedMemory {
        id: memory.id.clone(),
        memory_type: memory.memory_type,
        length: memory.content.chars().count(),
        excerpt: excerpt(&memory.content),
    }
}

/// Which copy of a duplicate cluster to keep.
///
/// Most-accessed first, then oldest. Access count is the better signal because
/// it reflects what recall actually surfaced; creation time only breaks ties,
/// and favouring the original keeps ids stable for anything referencing them.
fn keep_first(a: &Memory, b: &Memory) -> std::cmp::Ordering {
    b.access_count
        .cmp(&a.access_count)
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.id.cmp(&b.id))
}

/// Work out what should be removed, without removing anything.
pub fn plan_prune(graph: &dyn MemoryTrait) -> Result<PruneReport, MemoryError> {
    let total = graph.memory_count()?;
    let all = graph.list_recent(total.max(1))?;

    let mut report = PruneReport::default();
    let mut survivors: Vec<Memory> = Vec::with_capacity(all.len());

    for memory in all {
        if memory.content.chars().count() > content_limit(memory.memory_type) {
            report.oversized.push(describe(&memory));
        } else {
            survivors.push(memory);
        }
    }

    // Oversized entries are excluded from duplicate grouping on purpose: they
    // are already going, and listing one in both sections would double-count it
    // in `removal_count`.
    let mut clusters: HashMap<String, Vec<Memory>> = HashMap::new();
    for memory in survivors {
        clusters
            .entry(content_hash_tag(&memory.content))
            .or_default()
            .push(memory);
    }

    let mut groups: Vec<DuplicateGroup> = clusters
        .into_values()
        .filter(|cluster| cluster.len() > 1)
        .map(|mut cluster| {
            cluster.sort_by(keep_first);
            let kept = describe(&cluster[0]);
            let removed = cluster[1..].iter().map(describe).collect();
            DuplicateGroup { kept, removed }
        })
        .collect();

    // Stable ordering so two runs over an unchanged store read the same, and so
    // the biggest clusters are the first thing a reader sees.
    groups.sort_by(|a, b| {
        b.removed
            .len()
            .cmp(&a.removed.len())
            .then_with(|| a.kept.id.cmp(&b.kept.id))
    });
    report.duplicates = groups;
    // Descending by length, so `Reverse` rather than a bare key.
    report
        .oversized
        .sort_by_key(|entry| std::cmp::Reverse(entry.length));

    Ok(report)
}

/// Carry out the removals described by `plan`.
///
/// Returns the number of memories actually deleted. A memory that has already
/// gone is not an error: plans are made against a snapshot, and something else
/// may have removed it in between.
pub fn apply_prune(graph: &dyn MemoryTrait, plan: &PruneReport) -> Result<usize, MemoryError> {
    let mut deleted = 0usize;
    for id in plan.removal_ids() {
        match graph.delete_memory(&id) {
            Ok(()) => deleted += 1,
            Err(MemoryError::NotFound(_)) => {
                tracing::debug!(id, "prune: memory already gone, skipping");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(deleted)
}

/// Render a report for a human.
pub fn format_prune_report(report: &PruneReport) -> String {
    if report.is_empty() {
        return "\nNothing to prune: no oversized memories and no duplicate clusters.\n"
            .to_string();
    }

    let verb = if report.applied {
        "Removed"
    } else {
        "Would remove"
    };
    let mut out = format!("\n{verb} {} memories.\n", report.removal_count());

    if !report.oversized.is_empty() {
        out.push_str(&format!(
            "\nOversized ({}) — over the ingest cap for their type:\n",
            report.oversized.len()
        ));
        for memory in &report.oversized {
            out.push_str(&format!(
                "  [{}] {} — {} chars: {}\n",
                &memory.id[..8.min(memory.id.len())],
                memory.memory_type,
                memory.length,
                memory.excerpt,
            ));
        }
    }

    if !report.duplicates.is_empty() {
        out.push_str(&format!(
            "\nDuplicate clusters ({}) — one copy kept in each:\n",
            report.duplicates.len()
        ));
        for group in &report.duplicates {
            out.push_str(&format!(
                "  keep [{}] + drop {} copies: {}\n",
                &group.kept.id[..8.min(group.kept.id.len())],
                group.removed.len(),
                group.kept.excerpt,
            ));
        }
    }

    if !report.applied {
        out.push_str("\nRun `/memory prune apply` to carry this out.\n");
    }
    out
}

#[cfg(test)]
#[path = "hygiene_tests.rs"]
mod tests;
