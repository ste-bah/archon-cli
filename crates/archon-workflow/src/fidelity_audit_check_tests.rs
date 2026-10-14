use super::super::{ClaimingTask, FidelityVerdict};
use super::check_verdict;

const TASK: &str = include_str!("fixtures/TASK-DL-002.md");

fn verdict(necessarily_true: bool, quote: &str) -> FidelityVerdict {
    FidelityVerdict {
        obligation_id: "REQ-X-001".into(),
        necessarily_true,
        weakest_task_id: "TASK-X-001".into(),
        reason: "reason".into(),
        quoted_task_text: quote.into(),
        weakening_quotes: Vec::new(),
        supporting_task_id: if necessarily_true {
            "TASK-X-001".into()
        } else {
            String::new()
        },
        supporting_text: if necessarily_true {
            quote.into()
        } else {
            String::new()
        },
    }
}

fn task(text: &str) -> ClaimingTask {
    ClaimingTask {
        task_id: "TASK-X-001".into(),
        text: text.into(),
    }
}

#[test]
fn elided_support_quote_matches_real_task_body_and_exact_quote_still_passes() {
    let body = task(TASK);
    let mut elided = verdict(
        true,
        "Make dataset writes atomic … never leaves a healthy registry entry pointing at missing artifacts",
    );
    elided.obligation_id = "REQ-DL-092".into();
    assert!(check_verdict(&mut elided, std::slice::from_ref(&body)).is_ok());

    let mut exact = verdict(
        true,
        "Make dataset writes atomic: an interrupted ingest never leaves a healthy registry entry pointing at missing artifacts",
    );
    assert!(check_verdict(&mut exact, &[body]).is_ok());
}

#[test]
fn bracketed_emphasis_quote_matches_the_task_reference() {
    let body = task("**REQ-DL-034** — OpenBB/Polygon supplies the provider evidence.");
    let mut answer = verdict(false, "REQ-DL-034] — OpenBB/Polygon");
    assert!(check_verdict(&mut answer, &[body]).is_ok());
}

#[test]
fn quote_marks_dashes_and_wrapped_whitespace_are_normalized() {
    let body = task("**“One”** — two\nlines remain here.");
    let mut answer = verdict(false, "‘One’ - two lines");
    assert!(check_verdict(&mut answer, &[body]).is_ok());
}

#[test]
fn elided_pieces_must_appear_in_order() {
    let body = task("First the registry is written; later the report is emitted.");
    let mut answer = verdict(false, "report is emitted … registry is written");
    assert!(check_verdict(&mut answer, &[body]).is_err());
}

#[test]
fn an_elided_quote_piece_with_a_missing_word_is_rejected() {
    let body = task("The registry writes atomically.");
    let mut answer = verdict(false, "registry writes … missingword");
    assert!(check_verdict(&mut answer, &[body]).is_err());
}

#[test]
fn weakening_quotes_use_the_same_normalized_checker() {
    let body = task("**A registry entry** — remains stable.");
    let mut answer = verdict(false, "A registry entry — remains stable.");
    answer
        .weakening_quotes
        .push("`A registry entry` — remains stable.".into());
    assert!(check_verdict(&mut answer, &[body]).is_ok());
}
