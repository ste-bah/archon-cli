//! The two host-record gates of the authored run's terminal rule: the
//! backing a review-remediation resolution needs, and the acceptance round.

use super::*;

/// The host backing for "review remediation resolved `task`": its LAST fix
/// and its LAST verify, same round, fix first, both accepted, and the verify a
/// real agent rather than the no-patch checkpoint.
pub(super) fn remediation_backing(
    task: &str,
    calls: &[AuthoredCallFact],
) -> Result<(), (String, bool)> {
    let mut fix = None;
    let mut verify = None;
    for (at, call) in calls.iter().enumerate() {
        match &call.role {
            AuthoredCallRole::RemediationFix { task: t, round } if t == task => {
                fix = Some((at, *round, call));
            }
            AuthoredCallRole::RemediationVerify {
                task: t,
                round,
                agent,
            } if t == task => verify = Some((at, *round, *agent, call)),
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
        let outcome = call.task(task).or_else(|| call.outcome());
        match outcome {
            Some(outcome) if is_reusable_status(outcome.status) => {}
            Some(outcome) => {
                return Err((
                    format!("its last round's `{}` is {:?}", call.id, outcome.status),
                    outcome.transport,
                ));
            }
            None => return Err((format!("`{}` has no host record", call.id), false)),
        }
    }
    Ok(())
}

pub(super) fn acceptance_verdict(fact: AuthoredAcceptanceGateFact<'_>, v: &mut Verdict) {
    let (gate, record_call_id, last_call_id, last_call_status) = match fact {
        AuthoredAcceptanceGateFact::NotRequired => {
            v.notes
                .push("no acceptance stage (script predates the rule)".to_string());
            return;
        }
        AuthoredAcceptanceGateFact::Missing => {
            v.block("the acceptance stage recorded no round".to_string(), false);
            return;
        }
        AuthoredAcceptanceGateFact::Recorded {
            gate,
            record_call_id,
            last_call_id,
            last_call_status,
        } => (gate, record_call_id, last_call_id, last_call_status),
    };
    if record_call_id != last_call_id {
        v.block(
            format!(
                "the acceptance record belongs to `{record_call_id}`, not to `{last_call_id}`, the last round this run executed"
            ),
            false,
        );
    } else if !last_call_status.is_some_and(is_reusable_status) {
        v.block(
            format!(
                "acceptance call `{last_call_id}` recorded {}",
                last_call_status.map_or("no result".to_string(), |status| format!("{status:?}"))
            ),
            false,
        );
    }
    if gate.blocks_completion() {
        let clause = if gate.failing_check_ids.is_empty() {
            format!(
                "acceptance round {} could not evaluate: {}",
                gate.final_round,
                gate.operational_errors.join("; ")
            )
        } else {
            format!(
                "acceptance round {} has failing checks: {}",
                gate.final_round,
                gate.failing_check_ids.join(", ")
            )
        };
        v.block(clause, false);
    } else {
        v.notes.push(format!(
            "acceptance round {} passed{}",
            gate.final_round,
            if gate.contract_present {
                ""
            } else {
                " (no contract)"
            }
        ));
    }
}
