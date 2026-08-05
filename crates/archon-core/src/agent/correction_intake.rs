//! One writer for corrections, two detectors feeding it.
//!
//! Corrections used to be written from two places that did not know about each
//! other: this keyword detector, firing every turn, and the periodic LLM
//! extractor, which also emitted `correction` memories. The same correction
//! therefore landed twice in different words -- the "exact-text twins" a
//! diagnostic run surfaced.
//!
//! Collapsing to one writer would normally mean losing whatever the other
//! caught. The keyword matcher is immediate and free but only recognises the
//! phrasings listed in [`classify_correction`]; "that's not what I meant" slips
//! straight through. The extractor is semantic and catches those, but runs only
//! every few turns.
//!
//! So neither owns detection and both feed one writer. The keyword pass records
//! immediately, which is what matters -- a correction exists to stop the NEXT
//! turn repeating the mistake, and a record that arrives five turns later has
//! already missed its purpose. The extractor then reports only corrections the
//! keyword pass did not already capture, and those are recorded through the
//! same path. Late beats never; duplicated beats neither.

use archon_consciousness::corrections::{CorrectionTracker, CorrectionType};
use archon_memory::MemoryTrait;
use archon_memory::types::MemoryType;
use std::sync::Arc;

use super::support::stored_correction_content;

/// Classify `user_input` as a correction, if its phrasing matches a known form.
///
/// Shared so both detectors agree on the taxonomy. `None` means "no keyword
/// match", which is a statement about this function's patterns and not about
/// whether the user corrected anything -- the semantic pass decides that.
pub(super) fn classify_correction(user_input: &str) -> Option<CorrectionType> {
    let lower = user_input.to_lowercase();
    if lower.starts_with("no,")
        || lower.starts_with("no ")
        || lower.starts_with("wrong")
        || lower.starts_with("that's wrong")
        || lower.starts_with("that is wrong")
    {
        Some(CorrectionType::FactualError)
    } else if lower.contains("i said")
        || lower.contains("i already told you")
        || lower.contains("i already asked")
        || lower.contains("as i mentioned")
    {
        Some(CorrectionType::RepeatedInstruction)
    } else if lower.starts_with("don't ")
        || lower.starts_with("do not ")
        || lower.starts_with("stop ")
        || lower.contains("never do that")
    {
        Some(CorrectionType::DidForbiddenAction)
    } else if lower.contains("didn't ask")
        || lower.contains("did not ask")
        || lower.contains("without permission")
        || lower.contains("without asking")
    {
        Some(CorrectionType::ActedWithoutPermission)
    } else if lower.contains("instead,")
        || lower.contains("should have")
        || lower.contains("better approach")
        || lower.contains("use this instead")
    {
        Some(CorrectionType::ApproachCorrection)
    } else {
        None
    }
}

/// Record corrections the semantic pass found and the keyword pass missed.
///
/// Called from the extractor's background task with the items it typed
/// `correction`. They go through [`CorrectionTracker`] rather than
/// `store_extracted` so that there is exactly one writer of correction content,
/// and so these inherit the same rule linking and scoring as the fast path.
///
/// Returns how many were recorded.
pub(super) fn record_extracted_corrections(
    graph: &Arc<dyn MemoryTrait>,
    corrections: &[archon_memory::extraction::ExtractedMemory],
    context: &str,
) -> usize {
    let tracker = CorrectionTracker::new(graph.as_ref());
    let mut recorded = 0usize;

    for item in corrections {
        if item.memory_type != MemoryType::Correction {
            continue;
        }
        // The extractor supplies no taxonomy. Re-run the keyword classifier over
        // its restatement -- which is likelier to match than the raw turn was,
        // since it is written as a plain instruction -- and fall back to the
        // general bucket rather than dropping a correction over a missing label.
        let correction_type =
            classify_correction(&item.content).unwrap_or(CorrectionType::ApproachCorrection);
        // Bounded on this path too. It is a different writer reaching the same
        // relation, and the reason corrections needed bounding at all was a
        // writer that trusted its input.
        let content = stored_correction_content(&item.content);
        match tracker.record_correction(correction_type, &content, context, None) {
            Ok(_) => recorded += 1,
            Err(error) => {
                tracing::warn!(%error, "recording an extractor-found correction failed")
            }
        }
    }

    recorded
}

#[cfg(test)]
#[path = "correction_intake_tests.rs"]
mod tests;
