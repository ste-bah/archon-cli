//! Which landing of a subject is its current body, and which recorded
//! answer a resumed script gets for a landing request it asks again.
//!
//! A resumed run replays its script from the top. A host command's call id
//! names the logic version of the capability it runs, so after a version
//! bump every landing the script asks again arrives under a new id, and
//! every replay path that looks a record up by id finds nothing. Live, the
//! call then gated an old round's candidate again and published it over the
//! newer body that later rounds had accepted.
//!
//! A landing request is answered for good once it is recorded: its findings
//! are what the next author round was given, and asking it again can only
//! give the script a different history. So a request a record answers is
//! history whatever key the record was written under, except the request of
//! the subject's current body, which may be judged again under new logic
//! (its bytes are what is on disk, so a pass lands nothing new and a refusal
//! publishes nothing).
//!
//! - An *answer* is a non-invalidated, non-interrupted record of the request.
//!   An *evaluation* is a request and its occurrence: an identical candidate
//!   submitted twice in one script execution is evaluated twice.
//! - An *accepted publication* is an answer with a publication receipt and
//!   no finding that sends the candidate back.
//! - A *rerun* is a second answer to one evaluation that started after an
//!   accepted publication of another request of the same subject, where that
//!   publication itself started after the evaluation was first answered: an
//!   old round asked again under another key after the subject had moved on.
//!   It is neither history nor authority.
//! - The *current body* of a subject is its latest accepted publication
//!   that is not a rerun.
//!
//! Nothing here names a phase, capability, PRD or provider: a subject is the
//! request's target task, and every rule reads only the records.
use std::collections::BTreeSet;

use crate::v2::host_command::HostCommandRequest;
use crate::{WorkflowV2CallRecord, WorkflowV2HostMethod};

use super::history_replay::interrupted;

/// The task a landing request targets.
pub fn subject_of(request: &HostCommandRequest) -> Option<&str> {
    request
        .target
        .as_ref()
        .map(|target| target.task_id.as_str())
}

fn request_of(record: &WorkflowV2CallRecord) -> Option<&HostCommandRequest> {
    (record.call.method == WorkflowV2HostMethod::HostCommand)
        .then_some(record.call.options.host_command.as_ref())
        .flatten()
}

fn answers(record: &WorkflowV2CallRecord) -> bool {
    record.invalidated_by.is_none() && !interrupted(record)
}

/// Whether the record's gate sent its candidate back: a finding of any scope
/// but `inherited_predecessor`, or an operational error.
pub fn landing_refused(record: &WorkflowV2CallRecord) -> bool {
    let envelope = &record.result.data["gateEnvelope"];
    if !envelope["operational_error"].is_null() {
        return true;
    }
    envelope["policy_findings"]
        .as_array()
        .is_some_and(|findings| {
            findings
                .iter()
                .any(|finding| finding["remediation_scope"] != "inherited_predecessor")
        })
}

/// An answer that published its candidate and was not sent back.
pub fn accepted_publication(record: &WorkflowV2CallRecord) -> bool {
    answers(record)
        && !record.result.data["publicationReceipt"].is_null()
        && !landing_refused(record)
}

/// The host's call-option key naming a repeated evaluation of identical
/// content within one script execution (`workflow_host_command_occurrence`
/// in the binary); absent on a first evaluation.
pub const OCCURRENCE_KEY: &str = "host_command_occurrence";

/// The evaluation a record answers: its request and which identical
/// evaluation of it within its script execution (1 for the first). An
/// identical candidate submitted twice is two evaluations; the same
/// evaluation asked again under another key is one.
pub fn occurrence_of(record: &WorkflowV2CallRecord) -> u64 {
    call_occurrence(&record.call)
}

/// The evaluation ordinal `call` carries (1 for the first).
pub fn call_occurrence(call: &crate::WorkflowV2HostCall) -> u64 {
    call.options
        .extra
        .get(OCCURRENCE_KEY)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
}

type Slot<'r> = (Option<&'r HostCommandRequest>, u64);

fn slot_of(record: &WorkflowV2CallRecord) -> Slot<'_> {
    (request_of(record), occurrence_of(record))
}

/// Every answer of one subject's landings, in start order, with its rerun
/// flag.
pub struct LandingLineage<'a> {
    answers: Vec<&'a WorkflowV2CallRecord>,
    reruns: BTreeSet<&'a str>,
}

impl<'a> LandingLineage<'a> {
    /// The lineage of `subject` among `records`, for requests of
    /// `command_id`.
    pub fn of(records: &'a [WorkflowV2CallRecord], command_id: &str, subject: &str) -> Self {
        let mut answers: Vec<&WorkflowV2CallRecord> = records
            .iter()
            .filter(|record| answers(record))
            .filter(|record| {
                request_of(record).is_some_and(|request| {
                    request.command_id == command_id && subject_of(request) == Some(subject)
                })
            })
            .collect();
        answers.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then(a.call.id.cmp(&b.call.id))
        });
        // A rerun asks an evaluation already answered again, after a body of
        // another request was accepted since that first answer.
        let mut reruns = BTreeSet::new();
        for (index, record) in answers.iter().enumerate() {
            let slot = slot_of(record);
            let first = answers[..index]
                .iter()
                .position(|earlier| slot_of(earlier) == slot);
            let Some(first) = first else { continue };
            let moved_on = answers[first + 1..index].iter().any(|between| {
                request_of(between) != slot.0
                    && !reruns.contains(between.call.id.as_str())
                    && accepted_publication(between)
            });
            if moved_on {
                reruns.insert(record.call.id.as_str());
            }
        }
        Self { answers, reruns }
    }

    pub fn is_rerun(&self, record: &WorkflowV2CallRecord) -> bool {
        self.reruns.contains(record.call.id.as_str())
    }

    /// The subject's current body: its latest accepted publication that is
    /// not a rerun.
    pub fn current(&self) -> Option<&'a WorkflowV2CallRecord> {
        self.answers
            .iter()
            .rev()
            .find(|record| !self.is_rerun(record) && accepted_publication(record))
            .copied()
    }

    /// The recorded answer a resumed script gets for evaluation
    /// `occurrence` of `request`, when that is a superseded round: answered
    /// by a record that is not a rerun (and that `skip` does not exclude),
    /// and not of the current body's request. The answer is the one the
    /// script went on from: the latest answer of that evaluation started
    /// before the first later answer of any other evaluation, or the latest
    /// one when none followed. Every other evaluation keeps its own answer,
    /// so repeated identical candidates replay in their recorded order.
    /// `None` leaves the call to the ordinary paths.
    pub fn superseded_answer(
        &self,
        request: &HostCommandRequest,
        occurrence: u64,
        skip: impl Fn(&WorkflowV2CallRecord) -> bool,
    ) -> Option<&'a WorkflowV2CallRecord> {
        if self
            .current()
            .is_some_and(|current| request_of(current) == Some(request))
        {
            return None;
        }
        let slot: Slot<'_> = (Some(request), occurrence);
        let first = self
            .answers
            .iter()
            .position(|record| slot_of(record) == slot && !self.is_rerun(record))?;
        let next = self.answers[first + 1..]
            .iter()
            .find(|record| slot_of(record) != slot && !self.is_rerun(record))
            .map(|record| record.started_at.as_str());
        self.answers[first..]
            .iter()
            .filter(|record| slot_of(record) == slot)
            .filter(|record| !self.is_rerun(record) && !skip(record))
            .filter(|record| next.is_none_or(|next| record.started_at.as_str() < next))
            .last()
            .copied()
    }

    /// Every answer, in start order.
    pub fn answers(&self) -> &[&'a WorkflowV2CallRecord] {
        &self.answers
    }
}

#[cfg(test)]
#[path = "landing_lineage_tests.rs"]
mod tests;
