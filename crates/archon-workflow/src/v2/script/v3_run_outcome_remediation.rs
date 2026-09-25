//! Review remediation under the authored run's terminal rule: what the
//! script says it resolved, left open, or could not act on, each held to the
//! host's records.

use std::collections::BTreeSet;

use super::keys::TaskKeys;
use super::*;

/// Judge `blocked` and `review_remediation`; returns the remediation keys that
/// reached an outcome, for the findings check.
pub(super) fn check_remediation(
    accounting: &serde_json::Value,
    facts: &AuthoredRunFacts<'_>,
    keys: &TaskKeys<'_>,
    remediation_calls: &[AuthoredCallFact],
    v: &mut Verdict,
) -> BTreeSet<String> {
    let remediation = accounting.get("review_remediation");
    let reported = |field: &str| {
        array(remediation.and_then(|value| value.get(field)))
            .iter()
            .map(|entry| (keys.key(task_id(entry).unwrap_or("<unnamed>")), entry))
            .collect::<Vec<_>>()
    };
    let mut outcomes = BTreeSet::new();
    let mut resolved = BTreeSet::new();
    for (key, _) in reported("resolved") {
        outcomes.insert(key.clone());
        match remediation_backing(&key, keys, remediation_calls) {
            Ok(()) => {
                resolved.insert(key);
            }
            Err((clause, transport)) => v.block(
                format!("task {key} is reported resolved but {clause}"),
                transport,
            ),
        }
    }
    for entry in array(accounting.get("blocked")) {
        let task = keys.key(task_id(entry).unwrap_or("<unnamed>"));
        if resolved.contains(&task) {
            v.notes.push(format!(
                "blocked task {task} was finished by review remediation"
            ));
            continue;
        }
        let reason = text(entry.get("reason"));
        v.block(
            format!("task {task} is blocked: {}", clip(reason)),
            is_transport_failure_text(reason),
        );
    }
    for (key, entry) in reported("unresolved") {
        outcomes.insert(key.clone());
        let outcome = text(entry.get("outcome"));
        if outcome == NOT_TASK_ACTIONABLE_OUTCOME {
            not_task_actionable(&key, facts, keys, v);
            continue;
        }
        let reason = text(entry.get("reason"));
        v.block(
            format!(
                "task {key} review remediation is {}: {}",
                if outcome.is_empty() { "open" } else { outcome },
                clip(reason)
            ),
            is_transport_failure_text(reason),
        );
    }
    outcomes
}

/// `not_task_actionable` stands only when every task the key names is a
/// universe task with nothing it may write.
fn not_task_actionable(
    key: &str,
    facts: &AuthoredRunFacts<'_>,
    keys: &TaskKeys<'_>,
    v: &mut Verdict,
) {
    for part in keys.parts(key) {
        if keys.task(&part).is_none() {
            v.block(
                format!(
                    "task {key} is reported not_task_actionable, but {part} is not a task in the universe"
                ),
                false,
            );
            return;
        }
        if facts.writable_tasks.contains(&part) {
            v.block(
                format!(
                    "task {key} is reported not_task_actionable, but the task universe declares writable files for {part}"
                ),
                false,
            );
            return;
        }
    }
    v.notes
        .push(format!("task {key}: findings not task-actionable"));
}

/// Remediation the acceptance stage dispatched (`calls` from the first
/// acceptance round on). It is ONE bounded round per failing check, judged
/// in the end by the frozen checks themselves: a key whose last verifier
/// AGENT ran and rejected the fix holds the run, and so does a fix that died
/// on transport; a fix that landed nothing (no-patch checkpoint, or no
/// verifier) is discharged by a clean final full-contract round and otherwise
/// left to the failing gate, which already holds the run.
pub(super) fn check_acceptance_remediation(
    calls: &[AuthoredCallFact],
    keys: &TaskKeys<'_>,
    gate_clean: bool,
    v: &mut Verdict,
) {
    let touched: BTreeSet<String> = calls
        .iter()
        .filter_map(|call| match &call.role {
            AuthoredCallRole::RemediationFix { task, .. }
            | AuthoredCallRole::RemediationVerify { task, .. } => Some(keys.key(task)),
            _ => None,
        })
        .collect();
    for key in touched {
        let verified_by_agent = calls.iter().rev().find_map(|call| match &call.role {
            AuthoredCallRole::RemediationVerify { task, agent, .. } if keys.key(task) == key => {
                Some(*agent)
            }
            _ => None,
        });
        if verified_by_agent == Some(true) {
            if let Err((clause, transport)) = remediation_backing(&key, keys, calls) {
                v.block(
                    format!("acceptance-stage remediation of {key}: {clause}"),
                    transport,
                );
            }
            continue;
        }
        let last_fix = calls.iter().rev().find(|call| {
            matches!(&call.role, AuthoredCallRole::RemediationFix { task, .. } if keys.key(task) == key)
        });
        let died = last_fix.and_then(|fix| {
            keys.parts(&key)
                .iter()
                .filter_map(|task| fix.task(task).or_else(|| fix.outcome()))
                .find(|outcome| outcome.transport)
                .map(|_| fix.id.clone())
        });
        if let Some(fix) = died {
            v.block(
                format!("acceptance-stage remediation of {key}: fix `{fix}` failed on transport"),
                true,
            );
        } else if gate_clean {
            v.notes.push(format!(
                "acceptance-stage remediation of {key} landed no patch; discharged by the clean final round"
            ));
        }
    }
}

/// The host backing for "remediation resolved `key`": its LAST fix and its
/// LAST verify, same round, fix first, the verify a real agent rather than the
/// no-patch checkpoint, and both accepted for EVERY task the key names.
pub(super) fn remediation_backing(
    key: &str,
    keys: &TaskKeys<'_>,
    calls: &[AuthoredCallFact],
) -> Result<(), (String, bool)> {
    let mut fix = None;
    let mut verify = None;
    for (at, call) in calls.iter().enumerate() {
        match &call.role {
            AuthoredCallRole::RemediationFix { task, round } if keys.key(task) == key => {
                fix = Some((at, *round, call));
            }
            AuthoredCallRole::RemediationVerify { task, round, agent } if keys.key(task) == key => {
                verify = Some((at, *round, *agent, call));
            }
            _ => {}
        }
    }
    let Some((verify_at, verify_round, agent, verify)) = verify else {
        return Err((
            "the host has no remediation verify for it".to_string(),
            false,
        ));
    };
    let Some((fix_at, fix_round, fix)) = fix else {
        return Err(("the host has no remediation fix for it".to_string(), false));
    };
    if !agent {
        return Err((
            format!(
                "its last remediation verify `{}` is a no-patch checkpoint, not a verifier",
                verify.id
            ),
            false,
        ));
    }
    if fix_at > verify_at || fix_round != verify_round {
        return Err((
            format!(
                "its last remediation fix `{}` (round {fix_round}) was not verified by `{}` (round {verify_round})",
                fix.id, verify.id
            ),
            false,
        ));
    }
    for call in [fix, verify] {
        for task in keys.parts(key) {
            match call.task(&task).or_else(|| call.outcome()) {
                Some(outcome) if is_reusable_status(outcome.status) => {}
                Some(outcome) => {
                    return Err((
                        format!(
                            "its last round's `{}` is {:?} for {task}",
                            call.id, outcome.status
                        ),
                        outcome.transport,
                    ));
                }
                None => return Err((format!("`{}` has no host record", call.id), false)),
            }
        }
    }
    Ok(())
}
