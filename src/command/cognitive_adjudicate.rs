//! `archon cognitive adjudicate` — the human verdict on a causal attribution.
//!
//! R2's promotion gate rests on accepted-link precision, and precision is
//! undefined until somebody says what the right answer was. The engine writes
//! `adjudicated_causal_candidate_id = pending_adjudication:*` precisely so that
//! it cannot mark its own homework; this is the only way that field ever gets a
//! real value.
//!
//! Two verbs, deliberately separate. Listing shows what is waiting and the
//! candidates the engine ranked. Recording takes a correction id and either the
//! candidate that actually caused it or `--no-cause`, which is a real verdict
//! and the one that makes an accepted link wrong.

use anyhow::{Context, Result, bail};
use archon_cognitive::PersistentCognitiveStore;
use archon_cognitive::attribution::adjudication::{
    AttributionVerdict, PendingAttribution, list_pending, pending_for, record_adjudication,
};

/// Default number of pending attributions listed.
const DEFAULT_LIST_LIMIT: usize = 20;

pub(crate) fn handle_adjudicate(
    cwd: &std::path::Path,
    correction: Option<&str>,
    candidate: Option<&str>,
    no_cause: bool,
    adjudicator: Option<&str>,
    note: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<()> {
    let root = cwd.join(".archon").join("cognitive");
    let store = PersistentCognitiveStore::open(&root)
        .with_context(|| format!("open cognitive store at {}", root.display()))?;

    let Some(correction_id) = correction else {
        let limit = if limit == 0 {
            DEFAULT_LIST_LIMIT
        } else {
            limit
        };
        let pending = list_pending(store.db(), &root, limit)?;
        return print_pending(&pending, json);
    };

    // A verdict has to be one thing or the other. Defaulting either way would
    // put an answer nobody gave into the precision numerator.
    if candidate.is_some() == no_cause {
        bail!("choose exactly one of --candidate <id> or --no-cause");
    }
    let Some(adjudicator) = adjudicator else {
        bail!("--adjudicator is required: an anonymous verdict is not evidence");
    };

    let Some(pending) = pending_for(store.db(), &root, correction_id)? else {
        bail!(
            "no attribution is pending adjudication for correction `{correction_id}` \
             (already adjudicated, or never attributed)"
        );
    };
    if let Some(candidate) = candidate
        && !pending
            .ranked_candidate_ids
            .iter()
            .any(|id| id == candidate)
    {
        // The adjudicator picks from what the engine considered. A free-text id
        // would join to nothing and count as an incorrect link for the wrong
        // reason.
        bail!(
            "`{candidate}` was not among the candidates considered for this correction; \
             offered: {}",
            pending.ranked_candidate_ids.join(", ")
        );
    }

    let verdict = AttributionVerdict {
        adjudicated_candidate_id: candidate.map(str::to_string),
        adjudicator: adjudicator.to_string(),
        note: note.unwrap_or_default().to_string(),
    };
    let outcome = record_adjudication(store.db(), &root, &pending, &verdict, chrono::Utc::now())?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "correction_id": pending.correction_id,
                "adjudication_id": pending.adjudication_id(),
                "proposed_candidate_id": pending.proposed_candidate_id,
                "adjudicated_candidate_id": candidate.unwrap_or("none"),
                "adjudication_scope": pending.attribution_cohort,
                "outcome": format!("{outcome:?}"),
            })
        );
    } else {
        println!(
            "adjudicated {} ({}): proposed {} -> {} [{outcome:?}]",
            pending.correction_id,
            pending.attribution_cohort,
            pending.proposed_candidate_id,
            candidate.unwrap_or("no cause"),
        );
    }
    Ok(())
}

fn print_pending(pending: &[PendingAttribution], json: bool) -> Result<()> {
    if json {
        let rows: Vec<serde_json::Value> = pending
            .iter()
            .map(|item| {
                serde_json::json!({
                    "correction_id": item.correction_id,
                    "session_id": item.session_id,
                    "turn_number": item.turn_number,
                    "attribution_cohort": item.attribution_cohort,
                    "rationale_code": item.rationale_code,
                    "cause_action_class": item.cause_action_class,
                    "proposed_candidate_id": item.proposed_candidate_id,
                    "candidates": item.ranked_candidate_ids,
                    "lesson_id": item.lesson_id,
                    "recorded_at": item.recorded_at.to_rfc3339(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    if pending.is_empty() {
        println!("No attributions are awaiting adjudication.");
        println!(
            "Until some are adjudicated, causal_attribution_precision has no eligible \
             population and reports nothing rather than passing."
        );
        return Ok(());
    }

    println!("Attributions awaiting adjudication ({}):", pending.len());
    for item in pending {
        println!();
        println!(
            "  {}  session={} turn={} verdict={} ({})",
            item.correction_id,
            item.session_id,
            item.turn_number,
            item.attribution_cohort,
            item.rationale_code,
        );
        println!("    proposed cause: {}", item.proposed_candidate_id);
        if item.ranked_candidate_ids.is_empty() {
            println!("    candidates considered: none");
        } else {
            for candidate in &item.ranked_candidate_ids {
                println!("    candidate: {candidate}");
            }
        }
    }
    println!();
    println!(
        "Record a verdict with:\n  archon cognitive adjudicate --correction <id> \
         (--candidate <id> | --no-cause) --adjudicator <name>"
    );
    Ok(())
}
