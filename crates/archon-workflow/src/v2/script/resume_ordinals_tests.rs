use serde_json::json;

use super::{UnitOrdinals, ordinal_of, unit_ordinals};
use crate::v2::script::{WorkflowV2CallRecord, WorkflowV2HostMethod};
use crate::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Result};

/// A record of the unit `unit` at `minute`, as the prelude files its calls.
fn record(id: &str, stage: &str, round: u64, unit: &str, minute: u32) -> WorkflowV2CallRecord {
    let method = if id.ends_with("-no-patch") {
        WorkflowV2HostMethod::Checkpoint
    } else if stage == "remediate" {
        WorkflowV2HostMethod::Fanout
    } else {
        WorkflowV2HostMethod::Parallel
    };
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        json!({"version": 1, "stage": stage, "taskId": "T", "round": round, "contest": unit}),
    );
    let call = WorkflowV2HostCall {
        id: id.into(),
        method,
        write_mode: None,
        options,
    };
    let mut record = WorkflowV2CallRecord::new(
        "run",
        call,
        1,
        "h".into(),
        WorkflowV2Result::accepted("ok"),
        vec![],
    );
    record.started_at = format!("2026-09-27T10:{minute:02}:00+00:00");
    record
}

#[test]
fn an_ordinal_is_the_numeric_tail_of_a_prelude_id_only() {
    assert_eq!(ordinal_of("review-remediate-t-1-81"), Some(81));
    assert_eq!(
        ordinal_of("verification-wave-review-verify-x-1-82"),
        Some(82)
    );
    assert_eq!(ordinal_of("review-verify-x-1-81-moved"), None);
    assert_eq!(ordinal_of("review-verify-x-1-81-moved-r1"), None);
    assert_eq!(ordinal_of("residual-abc-done"), None);
    assert_eq!(ordinal_of("plain"), None);
}

#[test]
fn a_unit_with_no_record_moves_nothing() {
    let records = vec![record("review-remediate-o-1-5", "remediate", 1, "other", 1)];
    assert_eq!(unit_ordinals(&records, "u"), UnitOrdinals::default());
}

#[test]
fn a_verified_round_resumes_after_its_verifier() {
    let records = vec![
        record("review-remediate-u-1-81", "remediate", 1, "u", 1),
        record(
            "verification-wave-review-verify-u-1-82",
            "verify",
            1,
            "u",
            2,
        ),
    ];
    let got = unit_ordinals(&records, "u");
    assert_eq!(got.fix_ordinal, Some(81));
    assert_eq!(got.resume_ordinal, Some(82));
}

#[test]
fn a_fix_that_landed_nothing_counts_the_log_after_it() {
    let records = vec![
        record("review-remediate-u-1-79", "remediate", 1, "u", 1),
        record("review-verify-u-1-no-patch", "verify", 1, "u", 2),
    ];
    assert_eq!(unit_ordinals(&records, "u").resume_ordinal, Some(80));
}

/// Live shape: a round first filed at -81 in one session was re-dispatched
/// at -79 in the next; the latest attempt is the one that places it.
#[test]
fn the_latest_attempt_places_the_unit() {
    let records = vec![
        record("review-remediate-u-1-81", "remediate", 1, "u", 1),
        record("review-remediate-u-1-79", "remediate", 1, "u", 30),
        record("review-verify-u-1-no-patch", "verify", 1, "u", 31),
    ];
    let got = unit_ordinals(&records, "u");
    assert_eq!(got.fix_ordinal, Some(79));
    assert_eq!(got.resume_ordinal, Some(80));
}

#[test]
fn a_two_round_unit_resumes_after_its_last_round() {
    let records = vec![
        record("review-remediate-u-1-10", "remediate", 1, "u", 1),
        record(
            "verification-wave-review-verify-u-1-11",
            "verify",
            1,
            "u",
            2,
        ),
        record("review-remediate-u-2-12", "remediate", 2, "u", 3),
        record("review-verify-u-2-no-patch", "verify", 2, "u", 4),
    ];
    let got = unit_ordinals(&records, "u");
    assert_eq!(
        got.fix_ordinal,
        Some(10),
        "a resumed unit starts at round 1"
    );
    assert_eq!(got.resume_ordinal, Some(13));
}
