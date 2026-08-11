//! Tests for review-band adjudication.
//!
//! Split at the 500-line gate. This file keeps the doubles and fixtures, plus
//! the tests that need no running pass: prompt shape, verdict parsing, and the
//! per-run batch cap. [`background`] holds the ones that start the detached
//! pass. It is a CHILD module rather than a sibling so it still reaches the
//! private items of `garden_adjudicate` — and the fixtures here — by name.

use super::*;

mod background;

fn pair(a: &str, b: &str) -> ReviewPair {
    ReviewPair {
        a_id: format!("id-{a}"),
        b_id: format!("id-{b}"),
        a_content: a.to_string(),
        b_content: b.to_string(),
    }
}

/// Answers `SAME` for everything and keeps the prompts it was shown.
///
/// All-SAME on purpose: the interesting failures here are a batch that grew past
/// its cap and a call that should never have happened, and both are invisible
/// against a double that declines to merge.
#[derive(Default)]
struct RecordingClient {
    prompts: std::sync::Mutex<Vec<String>>,
}

impl RecordingClient {
    fn calls(&self) -> usize {
        self.prompts.lock().expect("prompts").len()
    }

    fn last_prompt(&self) -> String {
        self.prompts
            .lock()
            .expect("prompts")
            .last()
            .cloned()
            .expect("no adjudication call was made")
    }
}

#[async_trait::async_trait]
impl archon_pipeline::runner::LlmClient for RecordingClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        let prompt = messages
            .first()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        self.prompts.lock().expect("prompts").push(prompt);
        // Far more verdicts than any batch can contain. Out-of-range lines are
        // discarded by the parser, so this says SAME to every pair actually
        // asked about without the double needing to know how many there were.
        let verdicts = (1..=500)
            .map(|i| format!("{i}: SAME"))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(archon_pipeline::runner::LlmResponse {
            content: verdicts,
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

fn empty_store() -> Arc<dyn MemoryTrait> {
    Arc::new(archon_memory::MemoryGraph::in_memory().expect("in-memory graph"))
}

/// How many `N.` headers the prompt carries, i.e. how many pairs were judged.
fn numbered_pairs(prompt: &str) -> usize {
    prompt
        .lines()
        .filter(|line| {
            let line = line.trim();
            line.ends_with('.') && line.trim_end_matches('.').parse::<usize>().is_ok()
        })
        .count()
}

fn adjudicating(min_pairs: usize) -> archon_memory::garden::GardenConfig {
    archon_memory::garden::GardenConfig {
        auto_adjudicate_review_band: true,
        auto_adjudicate_min_pairs: min_pairs,
        ..archon_memory::garden::GardenConfig::default()
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

/// The per-run cap holds however large the band has grown.
///
/// It bounds cost and blast radius in one number, and a band left to accumulate
/// for weeks is exactly the input that would test it. The overflow is not lost:
/// the band writes nothing, so the unjudged pairs are re-derived by the next
/// consolidation and offered again.
#[tokio::test]
async fn no_more_than_max_pairs_per_run_are_judged() {
    let client = Arc::new(RecordingClient::default());
    let pairs: Vec<ReviewPair> = (0..MAX_PAIRS_PER_RUN * 3)
        .map(|i| pair(&format!("a{i}"), &format!("b{i}")))
        .collect();

    let merged = adjudicate_and_apply(
        client.clone(),
        empty_store(),
        pairs,
        "test-model".to_string(),
    )
    .await;

    assert_eq!(
        client.calls(),
        1,
        "the whole batch must cost exactly one round-trip"
    );
    assert_eq!(
        numbered_pairs(&client.last_prompt()),
        MAX_PAIRS_PER_RUN,
        "the batch must be truncated to the per-run cap"
    );
    assert_eq!(
        merged, 0,
        "these ids name no stored memory, so a SAME verdict must merge nothing"
    );
}
