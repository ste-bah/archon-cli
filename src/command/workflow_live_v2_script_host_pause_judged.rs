//! Issue 337: an unpublished host-command outcome a host-taken pause covers
//! replays while the content its gate reads is what it was AT THE PAUSE.
//!
//! A gate that takes a candidate (a freeze, a body landing) or judges the set
//! also reads the task root, and its call identity does not bind that
//! content. A refusal the task root caused must therefore not replay after
//! the operator repaired the task root. The content at the time of the call
//! is the wrong reference: the run goes on after a refusal and publishes the
//! contract, the skeleton and the bodies of other tasks (in parallel, while a
//! gate call runs), so that content is almost never there again, and every
//! resume would ask the judges again. A judge's new answer then changes the
//! history the script rebuilds (Issue 261).
//!
//! So the pause records, in its own lock section, the digest of what each
//! covered unpublished outcome's gate reads, as it is then
//! ([`digests_now`]). A resume compares it with the content as it is when the
//! resumed run starts ([`JudgedAtResume`]), before any call of that run can
//! change it: the run's own progress up to the pause never voids a replay,
//! an operator's change after it always does. A gate whose content the host
//! cannot digest (none known, a read that failed) has no entry, and its
//! outcome is asked again: that is safe, a new answer to a new question.

use std::collections::BTreeMap;

use super::workflow_live_v2_script_host_pause_credit::{
    CoveredAttempt, ScriptPauseRecord, pause_records,
};
use super::*;

type Executor = dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;

/// The covered records whose replay depends on the task root: the
/// unpublished host-command outcomes, with the request each one asked.
pub(super) fn unpublished_requests(
    records: &[WorkflowV2CallRecord],
    covered: &[CoveredAttempt],
) -> Vec<(String, archon_workflow::HostCommandRequest)> {
    records
        .iter()
        .filter(|record| {
            record.call.method == WorkflowV2HostMethod::HostCommand
                && record.result.data["publicationReceipt"].is_null()
                && covered
                    .iter()
                    .any(|attempt| attempt.call_id == record.call.id)
        })
        .filter_map(|record| {
            let request = record.call.options.host_command.clone()?;
            Some((record.call.id.clone(), request))
        })
        .collect()
}

/// What the gate of each request reads, as it is now. Called in the pause's
/// lock section. A read that fails is logged and leaves no entry.
pub(super) fn digests_now(
    executor: Option<&Executor>,
    requests: &[(String, archon_workflow::HostCommandRequest)],
) -> BTreeMap<String, String> {
    let Some(executor) = executor else {
        return BTreeMap::new();
    };
    requests
        .iter()
        .filter_map(|(call_id, request)| {
            let digest =
                crate::command::workflow_host_command_judged_inputs::read(executor, request)?;
            Some((call_id.clone(), digest))
        })
        .collect()
}

/// For each call: the attempt the LAST host-taken pause that covers it
/// covered, and whether what its gate reads is unchanged since that pause.
#[derive(Debug, Default, Clone)]
pub(in super::super) struct JudgedAtResume(BTreeMap<String, (CoveredAttempt, bool)>);

impl JudgedAtResume {
    /// Built from the pauses `pauses`, the slots `records` and the content
    /// the executor reads now.
    pub(super) fn build(
        pauses: &[ScriptPauseRecord],
        records: &[WorkflowV2CallRecord],
        executor: Option<&Executor>,
    ) -> Self {
        let Some(executor) = executor else {
            return Self::default();
        };
        let mut judged = BTreeMap::new();
        for (call_id, (pause, covered)) in last_host_pause_per_call(pauses) {
            let Some(at_pause) = pause.judged_at_pause.get(call_id) else {
                continue;
            };
            let Some(request) = records
                .iter()
                .find(|record| record.call.id == call_id)
                .and_then(|record| record.call.options.host_command.as_ref())
            else {
                continue;
            };
            let now = crate::command::workflow_host_command_judged_inputs::read(executor, request);
            let unchanged = now.as_deref() == Some(at_pause.as_str());
            if !unchanged {
                tracing::info!(
                    call_id,
                    pause_id = %pause.pause_id,
                    "the content this host command's gate reads changed since the pause; it is asked again"
                );
            }
            judged.insert(call_id.to_string(), (covered.clone(), unchanged));
        }
        Self(judged)
    }

    /// Whether `wanted` is the attempt the last pause covered and what its
    /// gate reads is unchanged since that pause.
    pub(super) fn unchanged_since_pause(&self, wanted: &CoveredAttempt) -> bool {
        self.0
            .get(&wanted.call_id)
            .is_some_and(|(covered, unchanged)| covered == wanted && *unchanged)
    }
}

/// The last host-taken pause (by generation, event, id) covering each call.
fn last_host_pause_per_call(
    pauses: &[ScriptPauseRecord],
) -> BTreeMap<&str, (&ScriptPauseRecord, &CoveredAttempt)> {
    let order =
        |pause: &ScriptPauseRecord| (pause.generation, pause.event_seq, pause.pause_id.clone());
    let mut last: BTreeMap<&str, (&ScriptPauseRecord, &CoveredAttempt)> = BTreeMap::new();
    for pause in pauses.iter().filter(|pause| pause.host_taken) {
        for covered in &pause.covered {
            let later = last
                .get(covered.call_id.as_str())
                .is_none_or(|(seen, _)| order(pause) > order(seen));
            if later {
                last.insert(covered.call_id.as_str(), (pause, covered));
            }
        }
    }
    last
}

impl WorkflowScriptHost {
    /// What a host-taken pause's unpublished outcomes judged, against the
    /// content as it is when this run starts: computed once, at the start
    /// (`run_on_current_thread`), before any call of this run can change it.
    /// An unreadable store is logged and replays none of them.
    pub(in super::super) fn judged_at_resume(&self) -> &JudgedAtResume {
        self.runner.judged_at_resume.get_or_init(|| {
            let built = pause_records(&self.runner.workflow_store, &self.runner.run_id).and_then(
                |pauses| {
                    if !pauses.iter().any(|pause| pause.host_taken) {
                        return Ok(JudgedAtResume::default());
                    }
                    let records = self.runner.v2_store.load_call_records()?;
                    Ok(JudgedAtResume::build(
                        &pauses,
                        &records,
                        self.runner.host_command_executor.as_deref(),
                    ))
                },
            );
            built.unwrap_or_else(|error| {
                tracing::warn!(%error, run_id = %self.runner.run_id, "pause records unreadable; every covered unpublished host-command outcome is asked again");
                JudgedAtResume::default()
            })
        })
    }
}
