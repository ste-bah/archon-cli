//! One judge call per check (#549).
//!
//! The judge is shown the files a check names under a byte limit. When one
//! call judged the whole set, the limit was shared in prompt order, so the
//! files of earlier checks starved later ones and the judge refuted sound
//! checks because their deciding files were "not supplied". Each check is
//! now judged in its own call with the whole limit for its own files, and
//! the calls run concurrently, bounded by the configured subagent
//! concurrency. A check's prompt depends only on that check and its files,
//! never on the other checks of the set or their order. Every call keeps the
//! complete-id and malformed-reply rules of a batch.
//!
//! A check whose own named files do not fit the limit is not sent to the
//! judge, and is never refuted on content the judge did not see. The author
//! chose those files, so the check goes back to its author with what it
//! names and the limit, like any other finding: no host stop, no pause.

use super::incomplete::JudgeIncompleteKind;
use super::*;
use futures_util::StreamExt;

/// The call id of a check the host did not judge. Its verdict is `refuted`
/// only so that it is never published; it is no refutation.
pub(crate) const NOT_JUDGED_CALL: &str = "acceptance-judge-not-judged:";

/// `roots` with the repository commit resolved when it is not given.
pub(super) fn with_commit(roots: &JudgeRoots) -> JudgeRoots {
    let mut resolved_roots = roots.clone();
    if resolved_roots.repository_commit.is_none() {
        if let Some(repository) = resolved_roots
            .repository
            .as_deref()
            .and_then(|path| path.canonicalize().ok())
        {
            let output = archon_shell::spawn::command("git")
                .arg("-C")
                .arg(repository)
                .args(["rev-parse", "HEAD"])
                .output();
            resolved_roots.repository_commit = output.ok().and_then(|output| {
                output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
            });
        }
    }
    resolved_roots
}

/// The named files of `criterion` that do not fit the judged-source byte
/// limit of one check, which the judge would not be shown.
pub(super) fn withheld_files(criterion: &AcceptanceCriterion, roots: &JudgeRoots) -> Vec<String> {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    named_files_with_budget(criterion, roots, &mut remaining)
        .into_iter()
        .filter_map(|file| {
            let path = file["path"].as_str()?.to_string();
            (file == named_files::over_budget(&path)).then_some(path)
        })
        .collect()
}

/// Record `entry` as not judged: its named files do not fit the limit of
/// one check. The reason is the note its author gets.
fn mark_not_judged(entry: &mut AcceptanceCriterion, roots: &JudgeRoots) {
    let sizes = named_files::named_file_sizes(entry, roots);
    let total: usize = sizes.iter().map(|(_, size)| size).sum();
    let listed = (sizes.iter())
        .map(|(path, size)| format!("{path} {size} bytes"))
        .collect::<Vec<_>>()
        .join(", ");
    entry.judgment.verdict = JudgeDecision::Refuted;
    entry.judgment.counterexample =
        "none: the host did not ask the judge about this check".to_string();
    entry.judgment.reason = format!(
        "check '{}' names {total} bytes of source ({listed}), more than the judge can read ({} bytes); name only the files whose content decides the criterion, or decide it with a command that does not need those files judged",
        entry.id,
        archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES
    );
    entry.judgment.host_call_id = format!("{NOT_JUDGED_CALL}{}", entry.id);
    entry.judgment.sampling = None;
}

/// The author note of `entry`, when the host did not judge it.
pub(crate) fn not_judged(entry: &AcceptanceCriterion) -> Option<&str> {
    (entry.judgment.verdict == JudgeDecision::Refuted
        && entry.judgment.host_call_id == format!("{NOT_JUDGED_CALL}{}", entry.id))
    .then_some(entry.judgment.reason.as_str())
}

/// What the author of a check that is not accepted is told: the note of a
/// check the host did not judge, else the judge's refutation.
pub(crate) fn author_note(entry: &AcceptanceCriterion) -> String {
    not_judged(entry).map_or_else(
        || {
            archon_workflow::task_set_contract::refuted_check_message(
                &entry.id,
                &entry.judgment.reason,
                &entry.judgment.counterexample,
            )
        },
        str::to_string,
    )
}

/// The author finding for each check of `contract` the host did not judge.
pub(crate) fn not_judged_findings(
    contract_path: &Path,
    contract: &AcceptanceContract,
) -> Vec<GateFinding> {
    (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter_map(|entry| {
            Some(GateFinding::new(
                GateId::FreezeAcceptance,
                not_judged(entry)?.to_string(),
                entry.id.clone(),
                Some(contract_path.to_path_buf()),
                archon_workflow::RemediationScope::CandidateArtifact,
            ))
        })
        .collect()
}

/// `contract` with only the check `id`.
fn single(contract: &AcceptanceContract, id: &str) -> AcceptanceContract {
    let mut one = subset_with_ids(contract, &BTreeSet::from([id.to_string()]));
    one.gap_policy
        .permitted_acceptance_ids
        .retain(|permitted| permitted == id);
    one
}

/// Judge every check of `contract` in its own call with `judge_one`. A check
/// whose own named files do not fit the limit is marked not judged and no
/// call is made for it. When a call cannot complete, the result is one
/// [`JudgeIncomplete`] that carries every entry decided by any call; a host
/// or provider fault outranks a judge-authored one, so no check is sent to
/// its author for a fault that was not the judge's reply.
pub(crate) async fn judge_each<F, Fut>(
    contract: AcceptanceContract,
    roots: &JudgeRoots,
    judge_one: F,
) -> Result<AcceptanceContract>
where
    F: Fn(AcceptanceContract) -> Fut,
    Fut: std::future::Future<Output = Result<AcceptanceContract>>,
{
    let resolved_roots = with_commit(roots);
    let mut judged = contract.clone();
    let mut decided = Vec::new();
    let mut calls = Vec::new();
    for entry in judged
        .acceptance
        .iter_mut()
        .chain(&mut judged.supplementary)
    {
        if withheld_files(entry, &resolved_roots).is_empty() {
            calls.push((entry.id.clone(), single(&contract, &entry.id)));
        } else {
            mark_not_judged(entry, &resolved_roots);
            decided.push(entry.clone());
        }
    }
    let limit = crate::command::sona_workflow_shape_tuning::resolved_subagent_cap()
        .unwrap_or(1)
        .max(1);
    let answers = futures_util::stream::iter(calls.into_iter().map(|(id, one)| {
        let call = judge_one(one);
        async move { (id, call.await) }
    }))
    .buffered(limit)
    .collect::<Vec<_>>()
    .await;
    let mut failures = Vec::new();
    for (id, answer) in answers {
        match answer {
            Ok(answered) => {
                rounds::merge_judged_entries(&mut judged, &answered);
                decided.extend(
                    answered
                        .acceptance
                        .into_iter()
                        .chain(answered.supplementary),
                );
            }
            Err(error) => match JudgeIncomplete::caused(&error) {
                Some(incomplete) => {
                    decided.extend(incomplete.decided.iter().cloned());
                    rounds::merge_judged_entries(
                        &mut judged,
                        &AcceptanceContract {
                            acceptance: incomplete.decided.clone(),
                            supplementary: Vec::new(),
                            ..contract.clone()
                        },
                    );
                    let (kind, class) = (incomplete.kind, incomplete.fault_class);
                    failures.push((id, kind, class, incomplete.message.clone(), error));
                }
                None => return Err(error),
            },
        }
    }
    let Some(chosen) = (failures.iter())
        .position(|(_, kind, ..)| *kind == JudgeIncompleteKind::Provider)
        .or((!failures.is_empty()).then_some(0))
    else {
        return Ok(judged);
    };
    if contract.acceptance.len() + contract.supplementary.len() == 1 {
        // The only call: its own error is the whole answer, unchanged.
        return Err(failures.swap_remove(chosen).4);
    }
    let (kind, fault_class) = (failures[chosen].1, failures[chosen].2);
    let message = (failures.iter())
        .map(|(id, _, _, message, _)| format!("check '{id}': {message}"))
        .collect::<Vec<_>>()
        .join("; ");
    let mut incomplete = JudgeIncomplete::new(kind, message);
    incomplete.fault_class = fault_class;
    incomplete.decided = decided;
    Err(incomplete.into())
}

#[cfg(test)]
#[path = "workflow_task_set_judge_each_tests.rs"]
mod tests;
