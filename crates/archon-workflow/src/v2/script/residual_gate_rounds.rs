//! Issue-117/118: one pass's planned rounds at the final gate.

use std::collections::BTreeSet;
use std::path::Path;

use super::super::super::{
    WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2ResultStore, remediation_contract,
};
use super::super::dispositions::{
    Disposition, bare_id, disposition_of, same_disposed_gap, same_gap,
};
use super::super::gaps::gaps_of;
use super::super::{PlannedRound, Residual, RoundKind, accepted_verdict, finished};
use super::{ResidualVerdict, executed, latest, round_outcome, round_records};

/// Each round of `rounds`: resolved (a note, a discharged review unit, and
/// every gap it carries added to `resolved`), or every gap it carries
/// returned with why not -- weighed by the caller unless another round that
/// carried the same gap resolved it (Issue-118: a retried round).
pub(super) fn judge_rounds(
    rounds: &[PlannedRound],
    store: &WorkflowV2ResultStore,
    judges: &[&WorkflowV2CallRecord],
    repository_root: Option<&Path>,
    verdict: &mut ResidualVerdict,
    resolved: &mut BTreeSet<String>,
) -> Vec<(Residual, String, bool)> {
    let mut failed = Vec::new();
    for round in rounds {
        let own = round_records(store, &round.key);
        // No verifier agent of the round judged its (latest) fix, and no
        // later verifier recorded or kept open any gap it carries: only then
        // may the host's own tip run judge its gaps (`residual_gate_tip`).
        let recurs = recurred(round, &own, judges, store, repository_root);
        let unjudged = recurs.is_ok() && !own_verifier_judged(store, round, &own);
        match round_outcome(store, round, &own).and_then(|()| recurs) {
            Ok(()) => {
                verdict.notes.push(format!(
                    "host-planned {} round `{}` over {} resolved {}",
                    round.kind.as_str(),
                    round.key,
                    round.tasks.iter().cloned().collect::<Vec<_>>().join(", "),
                    described(round)
                ));
                resolved.extend(round.residuals.iter().map(Residual::key));
                if let Some(unit) = round
                    .unit_key
                    .as_ref()
                    .filter(|_| round.kind == RoundKind::Review)
                {
                    verdict.discharged.insert(unit.clone());
                }
            }
            Err(why) => {
                let why = format!(
                    "its host-planned {} round `{}` did not resolve it: {why}",
                    round.kind.as_str(),
                    round.key
                );
                failed.extend(
                    round
                        .residuals
                        .iter()
                        .map(|r| (r.clone(), why.clone(), unjudged)),
                );
                if round.kind == RoundKind::Review {
                    verdict.notes.push(format!(
                        "review unit {} was not completed by the host's ownership-expansion round: {why}",
                        round.unit_key.as_deref().unwrap_or_default()
                    ));
                }
            }
        }
    }
    failed
}

/// Whether a verifier AGENT of the round ran after its latest fix (for an
/// adjudication: at all).
fn own_verifier_judged(
    store: &WorkflowV2ResultStore,
    round: &PlannedRound,
    own: &[WorkflowV2CallRecord],
) -> bool {
    let verify = latest(store, own, "verify")
        .filter(|record| record.call.method != WorkflowV2HostMethod::Checkpoint);
    let Some(verify) = verify else {
        return false;
    };
    if round.kind == RoundKind::Adjudication {
        return true;
    }
    latest(store, own, "remediate")
        .is_none_or(|fix| executed(store, verify) >= executed(store, fix))
}

fn described(round: &PlannedRound) -> String {
    let gaps: Vec<String> = round.residuals.iter().map(Residual::label).collect();
    let mut text = if gaps.is_empty() {
        format!(
            "the refusal of review unit {}",
            round.unit_key.as_deref().unwrap_or_default()
        )
    } else {
        gaps.join(", ")
    };
    if !round.files.is_empty() {
        let files: Vec<&str> = round.files.iter().map(String::as_str).collect();
        text.push_str(&format!(" (granted {})", files.join(", ")));
    }
    text
}

/// `Err` when a verifier judging at or after the round recorded a gap of
/// `round` again, or the round's own judge reported it `open`. The same id
/// or the same opening words is the same gap at ANY severity. A shared
/// resolved file alone is, only when the gap it records is not labelled
/// low-impact by its recorder ([`super::super::low_impact_label`]; such a
/// gap is still work of its own, weighed as MEDIUM), the verifier judged one of the
/// round's tasks, and the round's own judge -- its latest verifier agent,
/// the one that judged its fix -- did not report the gap `resolved` in its
/// structured dispositions: a gap so reported that another gap names the
/// same file of is a NEW gap, weighed at its own severity where it was
/// recorded, unless a verifier that did not accept records it; and its
/// opening words are compared without the paths they name. Two gaps of the
/// round under one id are never resolved by a disposition. A gap entry
/// carries no status field, so resolution is never read from its prose; a
/// disposition from any other verifier is not read.
pub(super) fn recurred(
    round: &PlannedRound,
    own: &[WorkflowV2CallRecord],
    judges: &[&WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    root: Option<&Path>,
) -> Result<(), String> {
    let Some(since) = own.iter().map(|record| executed(store, record)).min() else {
        return Ok(());
    };
    let judge = latest(store, own, "verify")
        .filter(|record| record.call.method != WorkflowV2HostMethod::Checkpoint);
    // Two gaps of the round under one id: one entry cannot tell which it
    // resolves, so neither is resolved by it.
    let shared = |id: &str| {
        round
            .residuals
            .iter()
            .filter(|other| bare_id(&other.id) == bare_id(id))
            .count()
            > 1
    };
    let mut disposed = BTreeSet::new();
    for original in &round.residuals {
        match judge.and_then(|judge| disposition_of(judge, &original.id)) {
            Some(Disposition::Open) => {
                return Err(format!(
                    "its judge `{}` reported {} open",
                    judge
                        .map(|judge| judge.call.id.as_str())
                        .unwrap_or_default(),
                    original.label()
                ));
            }
            Some(Disposition::Resolved) if !shared(&original.id) => {
                disposed.insert(original.key());
            }
            Some(Disposition::Resolved | Disposition::Unreadable) | None => {}
        }
    }
    for judge in judges.iter().filter(|judge| finished(judge) >= since) {
        let judged = super::super::super::remediation_escalation::judged_commit(&judge.result);
        let mut tasks: BTreeSet<String> = judge
            .dispatched_items
            .iter()
            .flat_map(|item| item.canonical_task_ids.iter().cloned())
            .collect();
        if let Some(contract) = remediation_contract(&judge.call) {
            tasks.extend(super::super::super::remediation_escalation::unit_task_ids(
                contract,
            ));
        }
        let judges_the_round = !tasks.is_disjoint(&round.tasks);
        for (id, description, severity) in gaps_of(judge) {
            // A file alone ties a gap to the round's only when its recorder
            // did not label it low-impact: such a note on the fixed file is
            // a gap of its own (planned and weighed as MEDIUM, Batch O), not
            // the round's gap again.
            let severity = if id.starts_with(crate::v2::verification::UNOWNED_PATH_GAP_PREFIX) {
                super::super::gaps::flagged_severity(&description).map(str::to_string)
            } else {
                severity
            };
            let weighty = !super::super::low_impact_label(severity.as_deref());
            let files = root.map_or_else(Vec::new, |root| {
                super::super::super::residual_paths::named_files_at(
                    &description,
                    root,
                    judged.as_deref(),
                )
            });
            let again = round.residuals.iter().find(|original| {
                // A gap its judge disposed of as resolved is named again only
                // by its id or its own words: a shared file, or a shared path
                // opening the text, is a new gap -- unless a verifier that
                // did not accept records it, which is weighed nowhere else.
                let disposed = disposed.contains(&original.key());
                let same = if disposed {
                    same_disposed_gap(original, &id, &description)
                        .unwrap_or_else(|| same_gap(original, &id, &description))
                } else {
                    same_gap(original, &id, &description)
                };
                same || (weighty
                    && judges_the_round
                    && !(disposed && accepted_verdict(judge))
                    && files.iter().any(|file| original.files.contains(file)))
            });
            if let Some(original) = again {
                return Err(format!(
                    "`{}` recorded {} again as `{id}`",
                    judge.call.id,
                    original.label()
                ));
            }
        }
    }
    Ok(())
}
