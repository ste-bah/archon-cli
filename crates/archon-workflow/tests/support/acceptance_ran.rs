//! REM-13: the prelude runs the acceptance stage after every authored
//! script's last call, so a run's terminal rule is judged on the round its
//! last acceptance call recorded -- never on "no acceptance stage", which no
//! longer passes. Tests of other rules read that round here.
//!
//! Batch O2 (m6): as strict as the live host (`workflow_live_v3_run_end`):
//! a run with no acceptance round, or whose listed calls hold no acceptance
//! call, is `Missing` -- never the `NotRequired` pass a test could hide
//! behind -- and a round's `contract_present` is what the round recorded,
//! never a default.
#![allow(dead_code)]
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredCallFact, AuthoredCallRole, is_acceptance_stage_call,
};
use archon_workflow::*;

/// The last acceptance round the run recorded, as the terminal rule reads it;
/// none for a session run under a prelude that predates the stage (a
/// deployed fixture), which the host's rule then blocks (`Missing`).
pub struct AcceptanceRan(Option<Ran>);

struct Ran {
    gate: AuthoredAcceptanceGateV1,
    call_id: String,
}

impl AcceptanceRan {
    /// The run's last recorded acceptance round, if it recorded one.
    pub fn of(store: &WorkflowV2ResultStore) -> Self {
        let round = |record: &WorkflowV2CallRecord| {
            record.call.options.extra["round"].as_u64().unwrap_or(0)
        };
        let Some(record) = (store.load_call_records().unwrap().into_iter())
            .filter(|record| is_acceptance_stage_call(&record.call))
            .max_by_key(round)
        else {
            return Self(None);
        };
        let data = &record.result.data;
        let failing: Vec<String> = (data["failing"].as_array().into_iter().flatten())
            .filter_map(|check| check["check_id"].as_str().map(str::to_string))
            .collect();
        let errors: Vec<String> = (data["operational_errors"].as_array().into_iter().flatten())
            .filter_map(|error| error.as_str().map(str::to_string))
            .collect();
        Self(Some(Ran {
            gate: AuthoredAcceptanceGateV1 {
                final_round: round(&record) as u32,
                attempt: record.attempt,
                record_path: format!("v2/acceptance/{}", record.call.id),
                contract_present: data["contract_present"]
                    .as_bool()
                    .expect("the acceptance round records whether a contract was present"),
                failing_check_ids: failing,
                unowned_failing_check_ids: Vec::new(),
                operational_errors: errors,
            },
            call_id: record.call.id.clone(),
        }))
    }

    /// The gate fact the live host builds from that round and the run's
    /// listed `facts`: `Recorded` only when the round exists AND the run
    /// listed an acceptance call (the last one's id and status, as the live
    /// host reads them); `Missing` otherwise.
    pub fn fact<'a>(&'a self, facts: &'a [AuthoredCallFact]) -> AuthoredAcceptanceGateFact<'a> {
        let last = facts
            .iter()
            .rev()
            .find(|fact| fact.role == AuthoredCallRole::Acceptance);
        match (&self.0, last) {
            (Some(ran), Some(call)) => AuthoredAcceptanceGateFact::Recorded {
                gate: &ran.gate,
                record_call_id: &ran.call_id,
                last_call_id: &call.id,
                last_call_status: call.status,
            },
            _ => AuthoredAcceptanceGateFact::Missing,
        }
    }
}
