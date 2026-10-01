//! The acceptance-round gate of the authored run's terminal rule.

use super::*;

pub(super) fn acceptance_verdict(fact: AuthoredAcceptanceGateFact<'_>, v: &mut Verdict) {
    let (gate, record_call_id, last_call_id, last_call_status) = match fact {
        AuthoredAcceptanceGateFact::NotRequired => {
            // REM-13: acceptance decides every authored run. The prelude runs
            // the stage for a script that returned without it, so a run that
            // still recorded none never ran its checks: never a pass.
            v.block(
                "no acceptance round ran: the script returned without the acceptance stage and none was recorded after it; an authored run completes only on frozen checks that ran".to_string(),
                false,
            );
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
        let clause = if !gate.failing_check_ids.is_empty() {
            format!(
                "acceptance round {} has failing checks: {}",
                gate.final_round,
                gate.failing_check_ids.join(", ")
            )
        } else if !gate.operational_errors.is_empty() {
            format!(
                "acceptance round {} could not evaluate: {}",
                gate.final_round,
                gate.operational_errors.join("; ")
            )
        } else {
            // A8: nothing passes without a contract.
            format!(
                "acceptance round {} ran no acceptance contract; an authored run completes only on frozen checks that ran",
                gate.final_round
            )
        };
        v.block(clause, false);
    } else {
        v.notes
            .push(format!("acceptance round {} passed", gate.final_round));
    }
}
