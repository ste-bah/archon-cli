//! Blocking-gap events: what a discarded wave looks like from `events.jsonl`.
//!
//! A run keeps two records of itself. `v2/results/` is the authoritative one
//! and is written at call granularity; `events.jsonl` is the watchable one and
//! is the only one a monitor, the TUI, or a human tailing a file actually sees
//! while the run is in flight. They disagreed about failure.
//!
//! When a write-capable fanout lost an entire wave, the call record said so in
//! full — `status: "failed"` plus `residual_gaps` entries carrying
//! `severity: "blocking"` that named the wave and the branch whose output was
//! rejected. The event stream carried a `stage_failed` for the call and nothing
//! that named a gap, so the one question a watcher needs answered — *which*
//! gap blocked, and is it blocking at all — could only be answered by opening
//! `v2/results/`. Worse, a call that ends `accepted` while still carrying a
//! blocking gap produced no non-accepted event whatsoever.
//!
//! This module closes that. Every residual gap marked blocking becomes one
//! [`WorkflowEventKind::BlockingGapDetected`] event carrying the gap's id and
//! description, so the event stream names the same blockers the result store
//! does.
//!
//! It is pure by design — it builds `(kind, detail)` pairs and returns them,
//! exactly as
//! [`super::write_coordination_events::build_write_coordination_events`] does —
//! so the mapping is assertable without a store, and the caller keeps ownership
//! of sequence allocation.

use serde::Serialize;
use serde_json::Value;

use super::WorkflowEventKind;
use crate::error::WorkflowResult;
use crate::v2::{WorkflowV2CallRecord, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status};

/// The `severity` a residual gap uses to say "this blocks downstream
/// acceptance". Written by the write layer (`branch_validation_failure_fields`,
/// `result_from_write_fanout`) and read here; it is the one severity that means
/// work was discarded rather than merely flagged.
pub const BLOCKING_SEVERITY: &str = "blocking";

/// `detail.event` discriminator, matching the `call_finished` / `call_failed` /
/// `branch_started` vocabulary the rest of the v2 event details already use.
pub const BLOCKING_GAP_EVENT: &str = "blocking_gap";

/// The statuses a consumer reads as "this call is fine".
///
/// This is the "accepted set" the verification procedure in issue #162 refers
/// to. It lives here rather than being spelled out at each call site so the
/// event stream and any agreement check share one definition.
pub fn is_accepted_status(status: WorkflowV2Status) -> bool {
    matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

/// Whether one residual gap is blocking.
///
/// Case- and whitespace-insensitive: severities arrive from agent output as
/// well as from this crate's own constructors, and a gap that says `"Blocking"`
/// blocks exactly as hard as one that says `"blocking"`.
pub fn gap_is_blocking(gap: &WorkflowV2ResidualGap) -> bool {
    gap.severity
        .as_deref()
        .map(str::trim)
        .is_some_and(|severity| severity.eq_ignore_ascii_case(BLOCKING_SEVERITY))
}

/// Every blocking gap on a result, first occurrence of each id kept.
///
/// De-duplication is by id because `attach_branch_evidence` lifts each branch's
/// residual gaps onto the aggregate result without filtering, so an aggregate
/// can legitimately carry the same gap twice (a branch gap and the wave gap
/// derived from it). Emitting one event per distinct blocker keeps the stream
/// countable; the full unfiltered list stays in `v2/results/`.
pub fn blocking_gaps(result: &WorkflowV2Result) -> Vec<&WorkflowV2ResidualGap> {
    let mut seen = std::collections::BTreeSet::new();
    result
        .residual_gaps
        .iter()
        .filter(|gap| gap_is_blocking(gap))
        .filter(|gap| seen.insert(gap.id.clone()))
        .collect()
}

/// The blocking gap ids a call record reports, in emission order.
///
/// The agreement check in issue #162 compares this set — taken from
/// `v2/results/` — against the gap ids present in `events.jsonl`.
pub fn blocking_gap_ids(result: &WorkflowV2Result) -> Vec<String> {
    blocking_gaps(result)
        .into_iter()
        .map(|gap| gap.id.clone())
        .collect()
}

#[derive(Serialize)]
struct BlockingGapPayload {
    event: &'static str,
    call_id: String,
    method: String,
    status: WorkflowV2Status,
    gap_id: String,
    gap_description: String,
    severity: String,
    gap_index: usize,
    gap_total: usize,
    result_path: String,
    summary: String,
}

/// Build one event per blocking gap the call record carries.
///
/// `result_path` is the on-disk location of the record, so a reader who wants
/// the full picture after seeing the event knows where to look — but does not
/// have to look, because the id and description travel in the event.
///
/// Returns an empty vector when nothing blocked, which is the overwhelmingly
/// common case; callers can emit unconditionally.
pub fn build_blocking_gap_events(
    record: &WorkflowV2CallRecord,
    result_path: &str,
) -> WorkflowResult<Vec<(WorkflowEventKind, Value)>> {
    let gaps = blocking_gaps(&record.result);
    let gap_total = gaps.len();
    let mut events = Vec::with_capacity(gap_total);
    for (index, gap) in gaps.into_iter().enumerate() {
        events.push((
            WorkflowEventKind::BlockingGapDetected,
            serde_json::to_value(BlockingGapPayload {
                event: BLOCKING_GAP_EVENT,
                call_id: record.call.id.clone(),
                method: record.call.method.as_str().to_string(),
                status: record.status,
                gap_id: gap.id.clone(),
                gap_description: gap.description.clone(),
                severity: gap
                    .severity
                    .clone()
                    .unwrap_or_else(|| BLOCKING_SEVERITY.to_string()),
                gap_index: index,
                gap_total,
                result_path: result_path.to_string(),
                summary: record.result.summary.clone(),
            })?,
        ));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

    fn gap(id: &str, severity: Option<&str>) -> WorkflowV2ResidualGap {
        WorkflowV2ResidualGap {
            id: id.to_string(),
            description: format!("{id} description"),
            severity: severity.map(str::to_string),
        }
    }

    fn record(status: WorkflowV2Status, gaps: Vec<WorkflowV2ResidualGap>) -> WorkflowV2CallRecord {
        let call = WorkflowV2HostCall {
            id: "remediation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let result = WorkflowV2Result {
            status,
            summary: "write-capable fanout 'remediation-wave-1' failed".to_string(),
            residual_gaps: gaps,
            ..WorkflowV2Result::default()
        };
        WorkflowV2CallRecord::new("wf-test", call, 1, "hash".to_string(), result, Vec::new())
    }

    #[test]
    fn a_blocking_gap_becomes_an_event_naming_the_gap() {
        let record = record(
            WorkflowV2Status::Failed,
            vec![
                gap("write_fanout_failed_remediation-wave-1", Some("blocking")),
                gap("write_fanout_review_other", Some("review")),
            ],
        );

        let events = build_blocking_gap_events(&record, "v2/results/remediation-wave-1.json")
            .expect("build events");

        assert_eq!(events.len(), 1, "only the blocking gap is an event");
        assert_eq!(events[0].0, WorkflowEventKind::BlockingGapDetected);
        let detail = &events[0].1;
        assert_eq!(detail["event"], BLOCKING_GAP_EVENT);
        assert_eq!(
            detail["gap_id"], "write_fanout_failed_remediation-wave-1",
            "the reader must learn WHICH gap blocked without opening v2/results/"
        );
        assert_eq!(
            detail["gap_description"],
            "write_fanout_failed_remediation-wave-1 description"
        );
        assert_eq!(detail["status"], "failed");
        assert_eq!(detail["call_id"], "remediation-wave-1");
        assert_eq!(detail["gap_total"], 1);
    }

    /// An accepted-looking call can still be carrying a blocker. That case used
    /// to produce no non-accepted event of any kind, which is the sharpest form
    /// of the bug: nothing anywhere in the stream said the run was not fine.
    #[test]
    fn an_accepted_call_carrying_a_blocking_gap_still_emits() {
        let record = record(
            WorkflowV2Status::Accepted,
            vec![gap("invalid_write_branch_output_i1", Some("blocking"))],
        );

        let events = build_blocking_gap_events(&record, "v2/results/x.json").expect("build events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1["status"], "accepted");
        assert_eq!(events[0].1["gap_id"], "invalid_write_branch_output_i1");
    }

    /// `attach_branch_evidence` lifts branch gaps onto the aggregate without
    /// filtering, so the same id can appear twice on one record.
    #[test]
    fn a_repeated_gap_id_produces_one_event() {
        let record = record(
            WorkflowV2Status::Failed,
            vec![
                gap("invalid_write_branch_output_i1", Some("blocking")),
                gap("invalid_write_branch_output_i1", Some("blocking")),
            ],
        );

        let events = build_blocking_gap_events(&record, "v2/results/x.json").expect("build events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1["gap_total"], 1);
    }

    #[test]
    fn severity_case_and_padding_do_not_hide_a_blocker() {
        assert!(gap_is_blocking(&gap("g", Some(" Blocking "))));
        assert!(!gap_is_blocking(&gap("g", Some("review"))));
        assert!(!gap_is_blocking(&gap("g", None)));
    }

    #[test]
    fn a_clean_call_emits_nothing() {
        let record = record(
            WorkflowV2Status::Accepted,
            vec![gap("write_fanout_review_x", Some("review"))],
        );

        assert!(
            build_blocking_gap_events(&record, "v2/results/x.json")
                .expect("build events")
                .is_empty()
        );
    }

    #[test]
    fn the_accepted_set_is_accepted_and_noop() {
        assert!(is_accepted_status(WorkflowV2Status::Accepted));
        assert!(is_accepted_status(WorkflowV2Status::Noop));
        for status in [
            WorkflowV2Status::Failed,
            WorkflowV2Status::Cancelled,
            WorkflowV2Status::Blocked,
            WorkflowV2Status::NeedsReview,
        ] {
            assert!(!is_accepted_status(status), "{status:?} is not accepted");
        }
    }
}
