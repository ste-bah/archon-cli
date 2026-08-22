//! The case the run counter cannot see.

use super::*;

fn observe_all(novelty: &mut ResultNovelty, digests: &[u64]) -> usize {
    digests
        .iter()
        .filter(|digest| novelty.observe(**digest))
        .count()
}

/// The live failure: three spellings of one search, the same answer each time.
/// The run counter reset on every one of them because the argument strings
/// differed; the answers did not.
#[test]
fn varied_calls_returning_one_answer_are_noticed() {
    let same = result_digest("matching line\nmatching line\n");
    let mut novelty = ResultNovelty::default();

    let fired = observe_all(&mut novelty, &[same; NOVELTY_WINDOW]);

    assert_eq!(fired, 1, "it must speak up, and only once");
    assert_eq!(novelty.distinct_in_window(), 1);
}

/// Ordinary work alternates between answers and must not be called a loop.
#[test]
fn alternating_answers_are_not_a_loop() {
    let a = result_digest("file contents");
    let b = result_digest("tests passed");
    let mut novelty = ResultNovelty::default();

    let fired = observe_all(&mut novelty, &[a, b, a, b, a, b, a, b, a, b]);

    assert_eq!(fired, 0);
}

/// A partial window is never judged: the first calls of any stage repeat
/// legitimately, and firing there would train the model to ignore this.
#[test]
fn a_partial_window_is_never_judged() {
    let same = result_digest("same");
    let mut novelty = ResultNovelty::default();

    let fired = observe_all(&mut novelty, &vec![same; NOVELTY_WINDOW - 1]);

    assert_eq!(fired, 0);
}

/// Said once per stretch, not once per call — a spinning agent that ignores the
/// first reminder must not have the transcript filled with copies of it.
#[test]
fn a_continuing_loop_is_reported_once() {
    let same = result_digest("same");
    let mut novelty = ResultNovelty::default();

    let fired = observe_all(&mut novelty, &vec![same; NOVELTY_WINDOW * 3]);

    assert_eq!(fired, 1);
}

/// And it re-arms once the agent starts learning again, so a second stall later
/// in the same stage is still reported.
#[test]
fn it_rearms_after_progress_resumes() {
    let same = result_digest("same");
    let mut novelty = ResultNovelty::default();
    assert_eq!(observe_all(&mut novelty, &vec![same; NOVELTY_WINDOW]), 1);

    // Enough new answers to refill the window, which clears the warning.
    let fresh: Vec<u64> = (0..NOVELTY_WINDOW)
        .map(|i| result_digest(&format!("new {i}")))
        .collect();
    assert_eq!(observe_all(&mut novelty, &fresh), 0);

    // Stalling again is reported again.
    assert_eq!(observe_all(&mut novelty, &vec![same; NOVELTY_WINDOW]), 1);
}

/// The digest is only ever compared for equality, but it must at least separate
/// different text — a constant would make every agent look stuck.
#[test]
fn different_output_digests_differently() {
    assert_ne!(result_digest("a"), result_digest("b"));
    assert_eq!(result_digest("a"), result_digest("a"));
    assert_ne!(result_digest(""), result_digest("a"));
}

/// The reminder has to carry the fact, not just an accusation.
#[test]
fn the_reminder_names_what_was_observed() {
    let text = novelty_reminder("Grep", 1);
    assert!(text.contains("Grep"));
    assert!(text.contains("1 distinct"));
}

/// Per-attempt entropy must not make one answer look like many.
///
/// This is the defect that would have made the whole detector inert: results
/// routinely carry a fresh temp path or a duration, so two identical answers
/// digest differently and the window never sees a repeat. It is the same
/// mistake as comparing raw arguments, moved to the other side of the call.
#[test]
fn a_temp_path_does_not_make_one_answer_look_new() {
    let first = result_digest("wrote /tmp/run-abc123/report.json ok");
    let second = result_digest("wrote /tmp/run-def456/report.json ok");
    assert_eq!(first, second, "a per-run scratch id must not count as news");
}

#[test]
fn an_elapsed_time_does_not_make_one_answer_look_new() {
    let first = result_digest("test result: ok. 334 passed; finished in 3.86s");
    let second = result_digest("test result: ok. 334 passed; finished in 4.02s");
    assert_eq!(first, second, "a duration must not count as news");
}

/// And a genuinely different answer still reads as different — the stripping
/// must not flatten everything into one digest, which would fire constantly.
#[test]
fn normalisation_does_not_collapse_real_differences() {
    let pass = result_digest("test result: ok. 334 passed; 0 failed");
    let fail = result_digest("test result: FAILED. 330 passed; 4 failed");
    assert_ne!(pass, fail);

    let one = result_digest("src/a.rs:12: match");
    let other = result_digest("src/b.rs:99: match");
    assert_ne!(one, other, "different files are different answers");
}

/// The live shape: a test suite re-run whose only change is its duration.
#[test]
fn a_rerun_suite_that_only_changed_duration_is_a_stall() {
    let mut novelty = ResultNovelty::default();
    let fired = (0..NOVELTY_WINDOW)
        .filter(|i| {
            novelty.observe(result_digest(&format!(
                "test result: ok. 1079 passed; 0 failed; finished in {}.{}s",
                3 + i,
                i * 7 % 100
            )))
        })
        .count();
    assert_eq!(fired, 1, "eight identical verdicts must be noticed");
}

/// A line number is an identity, not a measurement.
///
/// Collapsing it would make two different matches in one file read as the same
/// answer, and a detector that fires on healthy search work gets ignored — the
/// worse of the two failure directions.
#[test]
fn line_numbers_are_not_collapsed() {
    let first = result_digest("src/lib.rs:12: fn parse");
    let second = result_digest("src/lib.rs:99: fn parse");
    assert_ne!(first, second, "different matches are different answers");
}

/// And a count is an identity too: 334 passing is not 330 passing.
#[test]
fn counts_are_not_collapsed() {
    assert_ne!(
        result_digest("334 passed; 0 failed"),
        result_digest("330 passed; 4 failed")
    );
}
