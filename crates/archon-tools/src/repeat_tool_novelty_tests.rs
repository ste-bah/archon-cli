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

    let fired = observe_all(&mut novelty, &[same; NOVELTY_WINDOW - 1]);

    assert_eq!(fired, 0);
}

/// Said once per stretch, not once per call — a spinning agent that ignores the
/// first reminder must not have the transcript filled with copies of it.
#[test]
fn a_continuing_loop_is_reported_once() {
    let same = result_digest("same");
    let mut novelty = ResultNovelty::default();

    let fired = observe_all(&mut novelty, &[same; NOVELTY_WINDOW * 3]);

    assert_eq!(fired, 1);
}

/// And it re-arms once the agent starts learning again, so a second stall later
/// in the same stage is still reported.
#[test]
fn it_rearms_after_progress_resumes() {
    let same = result_digest("same");
    let mut novelty = ResultNovelty::default();
    assert_eq!(observe_all(&mut novelty, &[same; NOVELTY_WINDOW]), 1);

    // Enough new answers to refill the window, which clears the warning.
    let fresh: Vec<u64> = (0..NOVELTY_WINDOW)
        .map(|i| result_digest(&format!("new {i}")))
        .collect();
    assert_eq!(observe_all(&mut novelty, &fresh), 0);

    // Stalling again is reported again.
    assert_eq!(observe_all(&mut novelty, &[same; NOVELTY_WINDOW]), 1);
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
fn scratch_paths_inside_json_values_preserve_the_surrounding_message() {
    assert_ne!(
        result_digest(r#"{"msg":"/tmp/x: error E0308"}"#),
        result_digest(r#"{"msg":"/tmp/y: error E0599"}"#),
        "different diagnostics stay distinct when their paths change"
    );
    assert_eq!(
        result_digest(r#"{"msg":"/tmp/x: error E0308"}"#),
        result_digest(r#"{"msg":"/tmp/y: error E0308"}"#),
        "a scratch path alone does not count as news"
    );
}

#[test]
fn an_elapsed_time_does_not_make_one_answer_look_new() {
    let first = result_digest("test result: ok. 334 passed; finished in 3.86s");
    let second = result_digest("test result: ok. 334 passed; finished in 4.02s");
    assert_eq!(first, second, "a duration must not count as news");
}

#[test]
fn a_timestamp_does_not_make_one_answer_look_new() {
    let first = result_digest("completed at 2026-10-16T14:23:01.123Z");
    let second = result_digest("completed at 2026-10-17T09:05:44.987+01:00");
    assert_eq!(first, second, "an attempt timestamp must not count as news");
    assert_eq!(
        result_digest("[16/Oct/2026:14:23:01 +0000] finished"),
        result_digest("[17/Oct/2026:09:05:44 +0000] finished")
    );
    assert_eq!(
        result_digest("2026-10-16 14:23:01 completed"),
        result_digest("2026-10-17 09:05:44 completed")
    );
    assert_eq!(
        result_digest("Oct 16 14:23:01 service ready"),
        result_digest("Nov 17 09:05:44 service ready")
    );
}

#[test]
fn compact_json_timestamps_are_ignored() {
    assert_eq!(
        result_digest(r#"{"ts":"2026-10-16T14:23:01Z"}"#),
        result_digest(r#"{"ts":"2026-10-17T09:05:44Z"}"#)
    );
}

#[test]
fn distinct_urls_and_iso_date_paths_remain_novel() {
    assert_ne!(
        result_digest("requested https://example.com/a"),
        result_digest("requested https://example.com/b"),
        "different URL paths are different answers"
    );
    assert_ne!(
        result_digest("requested https://example.com/2026-10-16T14:23:01/a"),
        result_digest("requested https://example.com/2026-10-17T14:23:01/a"),
        "an ISO-looking date embedded in a URL path is meaningful"
    );
}

#[test]
fn a_request_id_does_not_make_one_answer_look_new() {
    assert_eq!(
        result_digest("request_id=550e8400-e29b-41d4-a716-446655440000 completed"),
        result_digest("request_id=6ba7b810-9dad-11d1-80b4-00c04fd430c8 completed")
    );
    assert_eq!(
        result_digest("request_id req_alpha123 completed"),
        result_digest("request_id req_beta456 completed")
    );
    assert_eq!(
        result_digest("request_id: req_alpha123 completed"),
        result_digest("request_id: req_beta456 completed")
    );
    assert_eq!(
        result_digest(r#"{"level":"info","request_id":"req_alpha123"}"#),
        result_digest(r#"{"level":"info","request_id":"req_beta456"}"#)
    );
    assert_eq!(
        result_digest("request_id: req_alpha123\nnext value"),
        result_digest("request_id: req_beta456\nnext value")
    );
    assert_ne!(
        result_digest("request_id:\nalpha content"),
        result_digest("request_id:\nbeta content"),
        "the flag replaces only the request id and preserves following content"
    );
    assert_eq!(
        result_digest("Oct  6 14:23:01 service ready"),
        result_digest("Nov 17 09:05:44 service ready")
    );
    assert_eq!(
        result_digest("[2026-10-16 14:23:01] finished"),
        result_digest("[2026-10-17 09:05:44] finished")
    );
}

#[test]
fn json_with_spaced_values_normalises_ids_and_scratch_paths() {
    assert_eq!(
        result_digest(
            r#"{"ts":"2026-10-16T14:23:01Z","request_id":"req_alpha123","msg":"service ready","path":"/private/tmp/ac-x-1234/output file"}"#
        ),
        result_digest(
            r#"{"ts":"2026-10-17T09:05:44Z","request_id":"req_beta456","msg":"service ready","path":"/private/tmp/ac-x-9876/output file"}"#
        )
    );
    assert_ne!(
        result_digest(r#"{"request_id":"req_alpha123","msg":"service ready"}"#),
        result_digest(r#"{"request_id":"req_beta456","msg":"service failed"}"#)
    );
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
    assert_ne!(
        result_digest("release 2026-10-16"),
        result_digest("release 2026-10-17"),
        "date-only values are meaningful"
    );
    assert_ne!(
        result_digest("object 550e8400-e29b-41d4-a716-446655440000"),
        result_digest("object 6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
        "unlabelled UUIDs are meaningful"
    );
    assert_ne!(
        result_digest("req_schema"),
        result_digest("req_data"),
        "unlabelled req words are meaningful"
    );
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

#[test]
fn a_changed_test_count_stays_novel() {
    assert_ne!(
        result_digest("test result: ok. 334 passed; 0 failed"),
        result_digest("test result: ok. 335 passed; 0 failed")
    );
}

/// Issue-213 C2d: a reminder is not a stop. `stalled` is what the runner's
/// progress check reads: true only after the reminder was given AND the window
/// is still full of one answer; clear again as soon as the answers vary.
#[test]
fn stalled_holds_only_after_the_reminder_and_while_the_answers_repeat() {
    let same = result_digest("same");
    let mut novelty = ResultNovelty::default();
    observe_all(&mut novelty, &[same; NOVELTY_WINDOW - 1]);
    assert!(!novelty.stalled(), "a partial window is never stalled");
    observe_all(&mut novelty, &[same]);
    assert!(novelty.stalled(), "reminded and still repeating");
    observe_all(&mut novelty, &[same; NOVELTY_WINDOW]);
    assert!(novelty.stalled());
    observe_all(&mut novelty, &[result_digest("new answer")]);
    assert!(!novelty.stalled(), "a new answer clears it at once");
}

#[test]
fn the_stop_message_carries_the_marker_and_the_facts() {
    let text = no_progress_stop_message(8);
    assert!(text.starts_with(NO_PROGRESS_STOP_MARKER));
    assert!(text.contains("8 tool rounds"), "{text}");
}
