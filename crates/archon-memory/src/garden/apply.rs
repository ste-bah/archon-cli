//! Applying an approved proposal, and undoing it.
//!
//! Everything a governed garden proposal can do resolves to one reversible
//! primitive: a withheld-status tag on a memory row. Retiring a memory adds it;
//! rolling that back removes it. The row is never deleted, so "undo" is not a
//! best-effort reconstruction — it is the removal of the exact tag the apply
//! step added, and the memory returns with its content, importance, tags and
//! access history untouched.
//!
//! That is the reason retirement is defined as tagging rather than deleting.
//! A reviewer approving a batch of ninety proposals will misread one; if apply
//! destroyed rows, the only honest thing this module could offer would be a
//! rollback that logs and returns success. It offers a real one instead.
//!
//! # Prompt rules ride the same primitive, and that is why it works
//!
//! Rules are stored as memories, and the rules engine reads them back through
//! the same search path every other reader uses — so a rule row carrying the
//! retired tag disappears from the prompt block without its text, score, trend
//! or existence being touched. Retirement is therefore not a rule mutation: the
//! rule is intact and one tag away from returning, with its accumulated score
//! still on it.
//!
//! An approved rule retirement is still a governed act with a human decision
//! behind it. The generation half — which decides a rule has gone quiet — holds
//! no store handle at all and cannot reach this module.
//!
//! # What apply cannot do
//!
//! Nothing here deletes. There is no `delete_memory` call in this file, and the
//! two retirement entry points refuse rows of the wrong kind, so a proposal that
//! names a rule cannot retire an ordinary memory through the memory path or the
//! reverse.

use tracing::info;

use super::consolidation::{SemanticConsolidationCandidate, derived_memory_tags};
use crate::access::MemoryTrait;
use crate::types::{MemoryError, MemoryType, RETIRED_TAG, RelType, is_retired};

/// Whether an apply or rollback actually changed anything.
///
/// Distinguished rather than collapsed into `()` because a retry must be able to
/// say "already done" without claiming to have done it. A governed record that
/// counts every replay as a fresh application would inflate the acceptance
/// numbers the promotion gate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOutcome {
    /// The store changed.
    Changed,
    /// The store was already in the requested state. Idempotent replay.
    AlreadyInPlace,
}

impl ChangeOutcome {
    pub fn changed(self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// Retire an ordinary memory: hide it from reads without destroying it.
///
/// Refuses a prompt rule, so a mis-typed proposal cannot take a rule out of the
/// prompt through the memory path.
pub fn apply_memory_retirement(
    graph: &dyn MemoryTrait,
    memory_id: &str,
) -> Result<ChangeOutcome, MemoryError> {
    let memory = graph.inspect_memory(memory_id)?;
    if memory.memory_type == MemoryType::Rule {
        return Err(MemoryError::Database(format!(
            "{memory_id} is a prompt rule; rule retirement goes through \
             apply_rule_retirement so the two decisions stay distinct"
        )));
    }
    add_retired_tag(graph, memory_id)
}

/// Restore a retired memory.
pub fn rollback_memory_retirement(
    graph: &dyn MemoryTrait,
    memory_id: &str,
) -> Result<ChangeOutcome, MemoryError> {
    let memory = graph.inspect_memory(memory_id)?;
    if memory.memory_type == MemoryType::Rule {
        return Err(MemoryError::Database(format!(
            "{memory_id} is a prompt rule; use rollback_rule_retirement"
        )));
    }
    remove_retired_tag(graph, memory_id)
}

/// Retire a prompt rule: it leaves the prompt block, keeping its score.
///
/// The rules engine lists rules through the shared search path, which withholds
/// retired rows, so this is sufficient and no rule-mutating call is made. The
/// score, trend and text are exactly as they were, which is what makes the
/// rollback below a restoration rather than a re-creation.
pub fn apply_rule_retirement(
    graph: &dyn MemoryTrait,
    rule_id: &str,
) -> Result<ChangeOutcome, MemoryError> {
    require_rule(graph, rule_id)?;
    add_retired_tag(graph, rule_id)
}

/// Return a retired rule to the prompt, with its accumulated score intact.
pub fn rollback_rule_retirement(
    graph: &dyn MemoryTrait,
    rule_id: &str,
) -> Result<ChangeOutcome, MemoryError> {
    require_rule(graph, rule_id)?;
    remove_retired_tag(graph, rule_id)
}

/// Write the semantic memory a consolidation proposal describes.
///
/// Returns the new memory's id, which is what a rollback needs. The id is
/// derived from the candidate id so re-applying the same approved proposal
/// reaches the same row rather than minting a second one.
///
/// `DerivedFrom` edges are written from the new memory to each source, so the
/// corroboration the proposal claimed is inspectable afterwards rather than only
/// at review time. The sources themselves are NOT modified: consolidation adds a
/// claim, it does not retire the evidence. Retiring the sources would be a
/// separate proposal, with its own decision.
pub fn apply_semantic_consolidation(
    graph: &dyn MemoryTrait,
    candidate: &SemanticConsolidationCandidate,
    run_id: &str,
) -> Result<(String, ChangeOutcome), MemoryError> {
    let derived_id = derived_memory_id(&candidate.candidate_id);
    let tags = derived_memory_tags(run_id);
    let outcome = graph.store_memory_with_id_outcome(
        &derived_id,
        &candidate.proposed_content,
        &candidate.proposed_title,
        candidate.memory_type,
        candidate.proposed_importance,
        &tags,
        &candidate.source_type,
        &candidate.project_path,
    )?;
    if !outcome.created {
        // The proposal was already applied. Neither the row nor its edges are
        // rewritten: an applied consolidation is a fact, and re-running the
        // apply step must not quietly move it.
        return Ok((derived_id, ChangeOutcome::AlreadyInPlace));
    }
    for source in &candidate.sources {
        // Best effort per edge. A missing edge costs inspectability; failing the
        // whole apply after the memory exists would leave a derived row nothing
        // knows the provenance of, which is worse.
        if let Err(error) = graph.create_relationship(
            &derived_id,
            &source.memory_id,
            RelType::DerivedFrom,
            Some("garden semantic consolidation"),
            1.0,
        ) {
            tracing::warn!(
                %error,
                derived_id = %derived_id,
                source_id = %source.memory_id,
                "garden: consolidated memory stored without one of its provenance edges"
            );
        }
    }
    info!(
        derived_id = %derived_id,
        sources = candidate.sources.len(),
        "garden: semantic consolidation applied"
    );
    Ok((derived_id, ChangeOutcome::Changed))
}

/// Undo an applied consolidation by retiring the memory it created.
///
/// Retired rather than deleted, for consistency with every other undo here and
/// because the `DerivedFrom` edges point at this row: deleting it would leave
/// edges referencing a memory that no longer exists, which is the exact defect
/// that once made merges unrecoverable.
pub fn rollback_semantic_consolidation(
    graph: &dyn MemoryTrait,
    derived_memory_id: &str,
) -> Result<ChangeOutcome, MemoryError> {
    let memory = graph.inspect_memory(derived_memory_id)?;
    if !memory
        .tags
        .iter()
        .any(|tag| tag == super::provenance::DERIVED_TAG)
    {
        return Err(MemoryError::Database(format!(
            "{derived_memory_id} was not written by consolidation; refusing to \
             retire it as though it were"
        )));
    }
    add_retired_tag(graph, derived_memory_id)
}

/// The memory id an applied consolidation candidate writes to.
pub fn derived_memory_id(candidate_id: &str) -> String {
    format!("garden-derived:{candidate_id}")
}

fn require_rule(graph: &dyn MemoryTrait, rule_id: &str) -> Result<(), MemoryError> {
    let memory = graph.inspect_memory(rule_id)?;
    if memory.memory_type != MemoryType::Rule {
        return Err(MemoryError::Database(format!(
            "{rule_id} is not a prompt rule"
        )));
    }
    Ok(())
}

fn add_retired_tag(graph: &dyn MemoryTrait, memory_id: &str) -> Result<ChangeOutcome, MemoryError> {
    let memory = graph.inspect_memory(memory_id)?;
    if is_retired(&memory.tags) {
        return Ok(ChangeOutcome::AlreadyInPlace);
    }
    let mut tags = memory.tags.clone();
    tags.push(RETIRED_TAG.to_string());
    graph.update_memory(memory_id, None, Some(&tags))?;
    info!(memory_id, "garden: retired by approved proposal");
    Ok(ChangeOutcome::Changed)
}

fn remove_retired_tag(
    graph: &dyn MemoryTrait,
    memory_id: &str,
) -> Result<ChangeOutcome, MemoryError> {
    let memory = graph.inspect_memory(memory_id)?;
    if !is_retired(&memory.tags) {
        return Ok(ChangeOutcome::AlreadyInPlace);
    }
    let tags: Vec<String> = memory
        .tags
        .iter()
        .filter(|tag| *tag != RETIRED_TAG)
        .cloned()
        .collect();
    graph.update_memory(memory_id, None, Some(&tags))?;
    info!(memory_id, "garden: retirement rolled back");
    Ok(ChangeOutcome::Changed)
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod apply_tests;
