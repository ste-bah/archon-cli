//! The review surface: deciding, applying and undoing garden proposals.
//!
//! An unattended consolidation pass raises proposals and stops. This is where a
//! person acts on them, and it is the only place in the codebase that can move a
//! proposal past `Pending`. Nothing on any automatic path reaches these
//! functions.
//!
//! `/garden proposals` lists what is waiting, `approve` and `reject` decide,
//! `apply` carries out what was approved, and `rollback` undoes an application.
//! Each maps to one lifecycle transition, and the transition is validated by the
//! governed store rather than here — so a second surface added later cannot
//! invent a shortcut from raised to applied.
//!
//! # Apply is the only thing that changes the memory store
//!
//! And every change it makes is reversible: retiring adds a status tag, and
//! consolidating writes a new memory. `rollback` removes the tag or withdraws
//! the written memory. Nothing here deletes.

use std::sync::Arc;

use archon_learning::garden_proposals::{
    GardenProposalKind, GardenProposalRecord, GardenProposalStatus, get_garden_proposal,
    list_garden_proposals, transition_garden_proposal,
};
use archon_memory::MemoryTrait;
use archon_memory::garden::{
    SemanticConsolidationCandidate, apply_memory_retirement, apply_rule_retirement,
    apply_semantic_consolidation, rollback_memory_retirement, rollback_rule_retirement,
    rollback_semantic_consolidation,
};
use archon_tui::app::TuiEvent;

use crate::command::garden_metrics::{
    GardenMetricContext, record_proposal_applied, record_proposal_decided,
    record_proposal_rolled_back,
};
use crate::command::registry::CommandContext;

/// Subcommands this module owns. Anything else falls through to `/garden`.
pub(crate) const SUBCOMMANDS: [&str; 5] = ["proposals", "approve", "reject", "apply", "rollback"];

/// Handle a proposal subcommand, or return `false` if it is not one.
pub(crate) fn handle(ctx: &mut CommandContext, sub: &str, args: &[String]) -> bool {
    if !SUBCOMMANDS.contains(&sub) {
        return false;
    }
    let Some(db) = ctx.governed_learning_db.clone() else {
        ctx.emit(TuiEvent::Error(
            "Memory garden proposals need the governed-learning store, which is \
             not open in this session."
                .to_string(),
        ));
        return true;
    };
    let Some(memory) = ctx.memory.clone() else {
        ctx.emit(TuiEvent::Error(
            "/garden dispatched without a memory handle".to_string(),
        ));
        return true;
    };

    let argument = args.get(1).map(String::as_str).unwrap_or("").trim();
    match sub {
        "proposals" => list(ctx, &db),
        "approve" => decide(ctx, &db, argument, true),
        "reject" => decide(ctx, &db, argument, false),
        "apply" => apply_approved(ctx, &db, &memory),
        "rollback" => rollback(ctx, &db, &memory, argument),
        _ => unreachable!("guarded by SUBCOMMANDS"),
    }
    true
}

fn list(ctx: &mut CommandContext, db: &cozo::DbInstance) {
    let mut out = String::from("\nMemory Garden — governed proposals\n");
    out.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    let mut any = false;
    for status in [
        GardenProposalStatus::Pending,
        GardenProposalStatus::Approved,
        GardenProposalStatus::Applied,
    ] {
        let rows = match list_garden_proposals(db, status) {
            Ok(rows) => rows,
            Err(error) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Listing proposals failed: {error}"
                )));
                return;
            }
        };
        if rows.is_empty() {
            continue;
        }
        any = true;
        out.push_str(&format!("\n{} ({}):\n", status.as_str(), rows.len()));
        for row in rows.iter().take(20) {
            out.push_str(&format!(
                "  {}  [{}]\n    {}\n    {}\n",
                row.proposal_id,
                row.proposal_kind.as_str(),
                truncate(&row.excerpt, 90),
                row.detail,
            ));
        }
        if rows.len() > 20 {
            out.push_str(&format!("  … and {} more\n", rows.len() - 20));
        }
    }
    if !any {
        out.push_str("\nNothing awaiting review.\n");
    } else {
        out.push_str(
            "\n/garden approve <id> · /garden reject <id> · /garden apply · \
             /garden rollback <id>\n",
        );
    }
    ctx.emit(TuiEvent::TextDelta(out));
}

fn decide(ctx: &mut CommandContext, db: &cozo::DbInstance, id: &str, accepted: bool) {
    if id.is_empty() {
        ctx.emit(TuiEvent::Error(
            "Give the proposal id: /garden approve <id>".to_string(),
        ));
        return;
    }
    let next = if accepted {
        GardenProposalStatus::Approved
    } else {
        GardenProposalStatus::Rejected
    };
    match transition_garden_proposal(db, id, next, None, &chrono::Utc::now().to_rfc3339()) {
        Ok(updated) => {
            record_proposal_decided(&metric_context(ctx), &updated, accepted);
            ctx.emit(TuiEvent::TextDelta(format!(
                "\nProposal {id} is now {}.{}\n",
                next.as_str(),
                if accepted {
                    " Run `/garden apply` to carry it out."
                } else {
                    ""
                }
            )));
        }
        Err(error) => ctx.emit(TuiEvent::Error(format!("Decision refused: {error}"))),
    }
}

/// Carry out every approved proposal.
///
/// Per proposal rather than as a batch: one failure must not strand the others,
/// and each application is recorded the moment it succeeds so an interruption
/// leaves the governed record agreeing with the store.
fn apply_approved(ctx: &mut CommandContext, db: &cozo::DbInstance, memory: &Arc<dyn MemoryTrait>) {
    let approved = match list_garden_proposals(db, GardenProposalStatus::Approved) {
        Ok(rows) => rows,
        Err(error) => {
            ctx.emit(TuiEvent::Error(format!("Listing approved failed: {error}")));
            return;
        }
    };
    if approved.is_empty() {
        ctx.emit(TuiEvent::TextDelta(
            "\nNothing approved is waiting to be applied.\n".to_string(),
        ));
        return;
    }
    let context = metric_context(ctx);
    let mut applied = 0usize;
    let mut failed = 0usize;
    for proposal in &approved {
        match apply_one(memory.as_ref(), proposal) {
            Ok(applied_ref) => {
                match transition_garden_proposal(
                    db,
                    &proposal.proposal_id,
                    GardenProposalStatus::Applied,
                    Some(&applied_ref),
                    &chrono::Utc::now().to_rfc3339(),
                ) {
                    Ok(updated) => {
                        record_proposal_applied(&context, &updated, &applied_ref);
                        applied += 1;
                    }
                    Err(error) => {
                        // The store changed but the record did not. Said plainly
                        // rather than counted as a success, because the two
                        // disagreeing is exactly what a reviewer needs to know.
                        failed += 1;
                        tracing::error!(
                            %error,
                            proposal = %proposal.proposal_id,
                            "garden: change applied but the governed record was not updated"
                        );
                    }
                }
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(%error, proposal = %proposal.proposal_id, "garden: apply failed");
            }
        }
    }
    ctx.emit(TuiEvent::TextDelta(format!(
        "\nApplied {applied} of {} approved proposal(s){}.\n\
         Undo any of them with `/garden rollback <id>`.\n",
        approved.len(),
        if failed > 0 {
            format!("; {failed} failed, see the log")
        } else {
            String::new()
        }
    )));
}

/// Dispatch one approved proposal to its applier, returning the rollback target.
fn apply_one(memory: &dyn MemoryTrait, proposal: &GardenProposalRecord) -> anyhow::Result<String> {
    match proposal.proposal_kind {
        GardenProposalKind::MemoryRetirement => {
            apply_memory_retirement(memory, &proposal.subject_id)?;
            Ok(proposal.subject_id.clone())
        }
        GardenProposalKind::RuleRetirement => {
            apply_rule_retirement(memory, &proposal.subject_id)?;
            Ok(proposal.subject_id.clone())
        }
        GardenProposalKind::SemanticConsolidation => {
            let candidate: SemanticConsolidationCandidate =
                serde_json::from_str(&proposal.payload_json).map_err(|error| {
                    anyhow::anyhow!("consolidation payload unreadable: {error}")
                })?;
            let (derived, _) = apply_semantic_consolidation(memory, &candidate, &proposal.run_id)?;
            Ok(derived)
        }
    }
}

fn rollback(
    ctx: &mut CommandContext,
    db: &cozo::DbInstance,
    memory: &Arc<dyn MemoryTrait>,
    id: &str,
) {
    if id.is_empty() {
        ctx.emit(TuiEvent::Error(
            "Give the proposal id: /garden rollback <id>".to_string(),
        ));
        return;
    }
    let proposal = match get_garden_proposal(db, id) {
        Ok(Some(proposal)) => proposal,
        Ok(None) => {
            ctx.emit(TuiEvent::Error(format!("No proposal {id}.")));
            return;
        }
        Err(error) => {
            ctx.emit(TuiEvent::Error(format!("Reading {id} failed: {error}")));
            return;
        }
    };
    if let Err(error) = rollback_one(memory.as_ref(), &proposal) {
        ctx.emit(TuiEvent::Error(format!("Rollback failed: {error}")));
        return;
    }
    match transition_garden_proposal(
        db,
        id,
        GardenProposalStatus::RolledBack,
        None,
        &chrono::Utc::now().to_rfc3339(),
    ) {
        Ok(updated) => {
            record_proposal_rolled_back(&metric_context(ctx), &updated);
            ctx.emit(TuiEvent::TextDelta(format!(
                "\nProposal {id} rolled back. {}\n",
                match proposal.proposal_kind {
                    GardenProposalKind::MemoryRetirement => "The memory is readable again.",
                    GardenProposalKind::RuleRetirement =>
                        "The rule is back in the prompt with the score it had.",
                    GardenProposalKind::SemanticConsolidation =>
                        "The consolidated memory has been withdrawn; its sources were never touched.",
                }
            )));
        }
        Err(error) => ctx.emit(TuiEvent::Error(format!(
            "Store restored, but the governed record was not updated: {error}"
        ))),
    }
}

fn rollback_one(memory: &dyn MemoryTrait, proposal: &GardenProposalRecord) -> anyhow::Result<()> {
    match proposal.proposal_kind {
        GardenProposalKind::MemoryRetirement => {
            rollback_memory_retirement(memory, &proposal.subject_id)?;
        }
        GardenProposalKind::RuleRetirement => {
            rollback_rule_retirement(memory, &proposal.subject_id)?;
        }
        GardenProposalKind::SemanticConsolidation => {
            // The written memory, not the candidate: `applied_ref` is what the
            // apply step actually created.
            rollback_semantic_consolidation(memory, &proposal.applied_ref)?;
        }
    }
    Ok(())
}

fn metric_context(ctx: &CommandContext) -> GardenMetricContext {
    GardenMetricContext {
        working_dir: ctx
            .working_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(".")),
        model_id: ctx
            .default_model
            .clone()
            .unwrap_or_else(|| "unknown_model".to_string()),
        session_id: ctx
            .session_id
            .clone()
            .unwrap_or_else(|| "no_session".to_string()),
        // A slash command is not inside a turn. Zero is the honest value rather
        // than a guess, and the per-100-turns metrics do not read this kind.
        turn_number: 0,
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let clipped: String = value.chars().take(max).collect();
    format!("{clipped}…")
}

#[cfg(test)]
#[path = "garden_proposals_tests.rs"]
mod tests;
