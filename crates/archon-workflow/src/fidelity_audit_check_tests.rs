use super::super::{ClaimingTask, FidelityVerdict};
use super::{check_verdict, expand_supporting_quote};

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
fn elided_support_quote_requires_critic_confirmation_and_exact_quote_still_passes() {
    let body = task(TASK);
    let mut elided = verdict(
        true,
        "Make dataset writes atomic … never leaves a healthy registry entry pointing at missing artifacts",
    );
    elided.obligation_id = "REQ-DL-092".into();
    assert!(check_verdict(&mut elided, std::slice::from_ref(&body)).is_err());

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

#[test]
fn omitted_negation_is_not_accepted_as_supporting_evidence() {
    let body = task(
        "The writer guarantees atomic commits and never leaves a healthy registry entry pointing at missing artifacts.",
    );
    let mut answer = verdict(
        true,
        "writer guarantees atomic commits … leaves a healthy registry entry pointing at missing artifacts",
    );
    assert!(check_verdict(&mut answer, &[body]).is_err());
}

#[test]
fn weak_elision_across_sentences_is_rejected() {
    let body = task("The registry remains stable. The process writes and exits.");
    let mut answer = verdict(false, "the … and");
    assert!(check_verdict(&mut answer, &[body]).is_err());
}

#[test]
fn inline_code_backticks_are_preserved_and_weakening_elisions_are_rejected() {
    let body = task("The identifier is ``foo`bar`` and is unique.");
    let mut answer = verdict(false, "foobar");
    assert!(check_verdict(&mut answer, std::slice::from_ref(&body)).is_err());

    let mut weakening = verdict(false, "The identifier is ``foo`bar`` and is unique.");
    weakening
        .weakening_quotes
        .push("The identifier is … and is unique.".into());
    assert!(check_verdict(&mut weakening, &[body]).is_err());
}

#[test]
fn elision_expansion_returns_the_unique_contiguous_fixture_span() {
    let expanded = expand_supporting_quote(
        TASK,
        "Make dataset writes atomic … registry entry pointing at missing artifacts",
    )
    .expect("the fixture chain is unique in one paragraph");
    assert_eq!(
        expanded,
        "Make dataset writes atomic: an interrupted ingest never leaves a healthy\n  registry entry pointing at missing artifacts"
    );
    assert!(expanded.contains("never"));
}

#[test]
fn elision_expansion_rejects_cross_sentence_or_ambiguous_chains() {
    assert!(
        expand_supporting_quote(
            "The registry remains. The process writes and exits.",
            "the … and"
        )
        .is_none()
    );
    assert!(
        expand_supporting_quote(
            "Alpha begins and Omega ends. Alpha begins and Omega ends.",
            "Alpha … Omega"
        )
        .is_none()
    );
}

#[test]
fn literal_ellipsis_is_not_evidence_and_unpaired_emphasis_is_preserved() {
    let body = task("A written … pause is literal. An unpaired marker * remains.");
    let mut ellipsis = verdict(false, "written … pause");
    assert!(check_verdict(&mut ellipsis, std::slice::from_ref(&body)).is_err());

    let mut emphasis = verdict(false, "An unpaired marker remains.");
    assert!(check_verdict(&mut emphasis, &[body]).is_err());
}
