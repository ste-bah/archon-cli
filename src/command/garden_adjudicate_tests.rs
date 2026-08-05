use super::*;

fn pair(a: &str, b: &str) -> ReviewPair {
    ReviewPair {
        a_id: format!("id-{a}"),
        b_id: format!("id-{b}"),
        a_content: a.to_string(),
        b_content: b.to_string(),
    }
}

/// The prompt has to name the failure mode, because the model cannot infer that
/// merging is destructive from the question alone.
#[test]
fn prompt_states_the_asymmetric_cost_and_the_worked_example() {
    let prompt =
        build_adjudication_prompt(&[pair("Deploy to eu-west-2", "Deploy region eu-west-2")]);

    assert!(
        prompt.contains("If you are unsure, answer DIFFERENT"),
        "the prompt must break ties toward not merging"
    );
    assert!(
        prompt.contains("merging two different memories destroys one of them"),
        "the prompt must state why the tie breaks that way"
    );
    assert!(
        prompt.contains("\"never deploy to us-east-1\" are DIFFERENT"),
        "the prompt must carry the same-subject/different-claim example"
    );
    assert!(prompt.contains("Deploy to eu-west-2"));
}

#[test]
fn verdicts_are_matched_back_by_number() {
    let verdicts = parse_verdicts("1: SAME\n2: DIFFERENT\n3: SAME", 3);
    assert_eq!(
        verdicts,
        vec![
            Adjudication::SameClaim,
            Adjudication::Distinct,
            Adjudication::SameClaim
        ]
    );
}

/// Every ambiguity must fall through to "do not merge".
///
/// This is the property that makes an LLM tolerable in a path that supersedes:
/// a truncated reply, a missing line, or added commentary must never cause a
/// merge that was not explicitly asked for.
#[test]
fn anything_other_than_an_explicit_same_is_treated_as_distinct() {
    // Truncated: only the first pair answered.
    assert_eq!(
        parse_verdicts("1: SAME", 3),
        vec![
            Adjudication::SameClaim,
            Adjudication::Distinct,
            Adjudication::Distinct
        ]
    );
    // Empty, unparseable, and out-of-range answers.
    assert_eq!(parse_verdicts("", 2), vec![Adjudication::Distinct; 2]);
    assert_eq!(
        parse_verdicts("I think they're all the same really", 2),
        vec![Adjudication::Distinct; 2]
    );
    assert_eq!(
        parse_verdicts("9: SAME", 2),
        vec![Adjudication::Distinct; 2]
    );
    assert_eq!(
        parse_verdicts("0: SAME", 2),
        vec![Adjudication::Distinct; 2]
    );
}

/// A verdict of DIFFERENT and a reply we could not read both yield zero merges.
///
/// The count is what tells them apart in the log. Without it a broken
/// adjudicator is indistinguishable from a careful one, and the failure is
/// invisible precisely because the safe default is silence.
#[test]
fn understood_count_separates_a_real_verdict_from_an_unreadable_reply() {
    let (_, understood) = parse_verdicts_counted("1: DIFFERENT\n2: DIFFERENT", 2);
    assert_eq!(understood, 2, "explicit DIFFERENTs are understood answers");

    let (verdicts, understood) = parse_verdicts_counted("sorry, I can't help with that", 2);
    assert_eq!(
        understood, 0,
        "an unreadable reply must report nothing understood"
    );
    assert_eq!(
        verdicts,
        vec![Adjudication::Distinct; 2],
        "and must still default to not merging"
    );

    let (_, understood) = parse_verdicts_counted("1: SAME", 3);
    assert_eq!(
        understood, 1,
        "a truncated reply reports only what it answered"
    );
}

/// Models decorate. The parse must survive it without becoming permissive.
#[test]
fn tolerates_formatting_noise_around_a_real_verdict() {
    assert_eq!(
        parse_verdicts("  1.:  same claim\n  2:  DIFFERENT — distinct regions", 2),
        vec![Adjudication::SameClaim, Adjudication::Distinct]
    );
}

/// A long pasted memory must not blow up the prompt.
#[test]
fn pair_content_is_excerpted() {
    let long = "x".repeat(5_000);
    let prompt = build_adjudication_prompt(&[pair(&long, "short")]);
    assert!(
        prompt.len() < 2_000,
        "prompt grew to {} chars; content is not being excerpted",
        prompt.len()
    );
}
