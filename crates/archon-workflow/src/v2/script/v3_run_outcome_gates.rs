//! The acceptance-round gate of the authored run's terminal rule.

use super::*;

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
