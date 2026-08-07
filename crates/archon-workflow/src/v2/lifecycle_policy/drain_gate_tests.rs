use super::evaluate;
use crate::board_port::{DrainItem, DrainItemKind, DrainStatus};

fn item(id: &str, kind: DrainItemKind, status: DrainStatus, reason: Option<&str>) -> DrainItem {
    DrainItem {
        id: id.to_string(),
        title: format!("{id} title"),
        kind,
        status,
        decline_reason: reason.map(str::to_string),
    }
}

fn issue(id: &str, status: DrainStatus) -> DrainItem {
    item(id, DrainItemKind::Issue, status, None)
}

#[test]
fn the_three_drain_outcomes_pass() {
    let outcome = evaluate(
        "wf-1",
        &[
            issue("a", DrainStatus::Resolved),
            issue("b", DrainStatus::Promoted),
            item(
                "c",
                DrainItemKind::Issue,
                DrainStatus::Declined,
                Some("superseded by the write-coordinator rewrite"),
            ),
        ],
    );
    assert!(outcome.passed());
    assert_eq!(outcome.issues, 3);
}

#[test]
fn an_open_item_fails_and_is_named() {
    let outcome = evaluate(
        "wf-1",
        &[
            issue("a", DrainStatus::Resolved),
            issue("b-2", DrainStatus::Open),
        ],
    );
    assert!(!outcome.passed());
    let message = outcome.failure_message();
    assert!(message.contains("b-2"), "{message}");
    assert!(message.contains("b-2 title"), "{message}");
    assert!(message.contains("still open"), "{message}");
    assert!(!message.contains("\"a\""), "{message}");
}

#[test]
fn every_non_terminal_status_fails() {
    for status in [
        DrainStatus::Open,
        DrainStatus::Claimed,
        DrainStatus::InReview,
        DrainStatus::GapsRemain,
        // Escalated is a request for a decision nobody has made yet. Counting
        // it as drained would let a run ship an open question as an answer.
        DrainStatus::Escalated,
    ] {
        let outcome = evaluate("wf-1", &[issue("a", status)]);
        assert!(!outcome.passed(), "{status:?} should not drain");
    }
}

#[test]
fn a_decline_needs_a_reason() {
    let no_reason = evaluate(
        "wf-1",
        &[item("a", DrainItemKind::Issue, DrainStatus::Declined, None)],
    );
    assert!(!no_reason.passed());
    assert!(
        no_reason
            .failure_message()
            .contains("declined without a recorded reason"),
        "{}",
        no_reason.failure_message()
    );

    let blank = evaluate(
        "wf-1",
        &[item(
            "a",
            DrainItemKind::Issue,
            DrainStatus::Declined,
            Some("   "),
        )],
    );
    assert!(!blank.passed(), "whitespace is not a reason");
}

#[test]
fn notes_are_not_drained() {
    // A note is context that dies with the run. Requiring it to be drained
    // makes the gate fire on "looked at X, seemed fine", and the cheapest way
    // to keep a run green becomes to stop leaving notes at all.
    let outcome = evaluate(
        "wf-1",
        &[
            item("n1", DrainItemKind::Note, DrainStatus::Open, None),
            issue("i1", DrainStatus::Resolved),
        ],
    );
    assert!(outcome.passed());
    assert_eq!(outcome.inspected, 2);
    assert_eq!(outcome.issues, 1);
}

#[test]
fn an_empty_board_passes() {
    assert!(evaluate("wf-1", &[]).passed());
}
