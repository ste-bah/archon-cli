//! Issue-117/118: one pass's planned rounds at the final gate.

use std::collections::BTreeSet;
use std::path::Path;

use super::super::super::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use super::super::{PlannedRound, Residual, RoundKind};
use super::{ResidualVerdict, recurred, round_outcome, round_records};

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
) -> Vec<(Residual, String)> {
    let mut failed = Vec::new();
    for round in rounds {
        let own = round_records(store, &round.key);
        match round_outcome(store, round, &own)
            .and_then(|()| recurred(round, &own, judges, store, repository_root))
        {
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
                failed.extend(round.residuals.iter().map(|r| (r.clone(), why.clone())));
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
