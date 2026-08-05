//! Judging the consolidation review band.
//!
//! Semantic consolidation merges only what distance proves identical. Between
//! that and "unrelated" sits a band where two memories are clearly about the
//! same subject but may or may not make the same claim — measured on a real
//! store, restatements of one instruction and genuinely distinct claims about
//! the same subject overlap in cosine distance, so no threshold separates them.
//!
//! That band is what an adjudicator is for: something that can read both and
//! tell "the deploy region is eu-west-2" from "never deploy to us-east-1".
//!
//! Three properties keep an LLM acceptable in a path that deletes:
//!
//! 1. It only ever sees the review band. The unambiguous merges never reach it
//!    and neither do unrelated pairs, so cost scales with genuine ambiguity.
//! 2. Merges are reversible. The loser is marked superseded, not deleted, so a
//!    wrong verdict is undone by removing a tag.
//! 3. It defaults to NOT merging. An unparseable answer, a failed call, or any
//!    uncertainty leaves both memories intact. Silence must never destroy.

use std::sync::Arc;

use archon_memory::MemoryTrait;
use archon_memory::garden::{Adjudication, ReviewPair};

/// Most pairs judged in one pass.
///
/// A bound on cost, and on blast radius: a run that wants to merge more than
/// this has probably found a systemic problem rather than a set of duplicates,
/// and should be looked at before it reshapes the graph.
const MAX_PAIRS_PER_RUN: usize = 20;

/// Ask whether each pair states the same thing.
///
/// One call for the whole batch, numbered so the answer can be matched back
/// positionally.
pub(crate) fn build_adjudication_prompt(pairs: &[ReviewPair]) -> String {
    let mut out = String::from(
        "You are deduplicating an AI assistant's long-term memory.\n\n\
         For each numbered pair, decide whether the two statements record the \
         SAME claim, merely worded differently.\n\n\
         Answer SAME only if one could replace the other with no loss of meaning. \
         Answer DIFFERENT if either carries information the other does not, or if \
         they concern the same subject but assert different things — \
         \"deploy to eu-west-2\" and \"never deploy to us-east-1\" are DIFFERENT.\n\n\
         If you are unsure, answer DIFFERENT. Keeping a duplicate costs a little \
         space; merging two different memories destroys one of them.\n\n\
         Reply with one line per pair, formatted exactly as `<number>: SAME` or \
         `<number>: DIFFERENT`. No other text.\n\n",
    );
    for (i, pair) in pairs.iter().enumerate() {
        out.push_str(&format!(
            "{}.\n  A: {}\n  B: {}\n\n",
            i + 1,
            excerpt(&pair.a_content),
            excerpt(&pair.b_content),
        ));
    }
    out
}

fn excerpt(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(300).collect()
}

/// Parse the verdict lines back into one decision per pair.
///
/// Anything not explicitly `SAME` is `Distinct`: a truncated reply, a missing
/// line, or commentary the model added all fall through to "do not merge".
pub(crate) fn parse_verdicts(response: &str, pair_count: usize) -> Vec<Adjudication> {
    parse_verdicts_counted(response, pair_count).0
}

/// As [`parse_verdicts`], also reporting how many lines were actually understood.
///
/// The count matters because "the model said DIFFERENT" and "the reply was
/// unparseable" both produce zero merges, and without this they are
/// indistinguishable in the log -- a broken adjudicator would look exactly like
/// a careful one.
pub(crate) fn parse_verdicts_counted(
    response: &str,
    pair_count: usize,
) -> (Vec<Adjudication>, usize) {
    let mut understood = 0usize;
    let mut verdicts = vec![Adjudication::Distinct; pair_count];
    for line in response.lines() {
        let line = line.trim();
        let Some((number, rest)) = line.split_once(':') else {
            continue;
        };
        let Ok(index) = number.trim().trim_end_matches('.').parse::<usize>() else {
            continue;
        };
        if index == 0 || index > pair_count {
            continue;
        }
        let answer = rest.trim().to_uppercase();
        if answer.starts_with("SAME") {
            verdicts[index - 1] = Adjudication::SameClaim;
            understood += 1;
        } else if answer.starts_with("DIFFERENT") {
            understood += 1;
        }
    }
    (verdicts, understood)
}

/// Judge the review band and apply the verdicts.
///
/// Returns how many pairs were merged. Any failure yields zero merges and
/// leaves the store untouched.
pub(crate) async fn adjudicate_and_apply(
    client: Arc<dyn archon_pipeline::runner::LlmClient>,
    memory: Arc<dyn MemoryTrait>,
    pairs: Vec<ReviewPair>,
    model: String,
) -> usize {
    if pairs.is_empty() {
        return 0;
    }
    let pairs: Vec<ReviewPair> = pairs.into_iter().take(MAX_PAIRS_PER_RUN).collect();
    let prompt = build_adjudication_prompt(&pairs);

    let response = match client
        .send_message(
            vec![serde_json::json!({ "role": "user", "content": prompt })],
            vec![serde_json::json!({
                "type": "text",
                "text": "You compare memory statements. Reply only with numbered SAME/DIFFERENT lines."
            })],
            Vec::new(),
            &model,
        )
        .await
    {
        Ok(response) => response.content,
        Err(error) => {
            // Nothing merges. The pairs remain in the review band and the next
            // consolidation will offer them again.
            tracing::warn!(%error, "memory consolidation adjudication failed; nothing merged");
            return 0;
        }
    };

    let (verdicts, understood) = parse_verdicts_counted(&response, pairs.len());
    if understood < pairs.len() {
        // Every unparsed line silently became "do not merge". That is the safe
        // default, but it must be visible: an adjudicator returning garbage
        // produces the same zero-merge log line as one judging carefully.
        tracing::warn!(
            understood,
            expected = pairs.len(),
            response = %response.chars().take(400).collect::<String>(),
            "adjudicator reply was not fully parseable; unparsed pairs default to not merging"
        );
    }
    let decided: Vec<(ReviewPair, Adjudication)> = pairs.into_iter().zip(verdicts).collect();
    let same = decided
        .iter()
        .filter(|(_, v)| *v == Adjudication::SameClaim)
        .count();

    match archon_memory::garden::apply_adjudicated_merges(memory.as_ref(), &decided) {
        Ok(merged) => {
            tracing::info!(
                judged = decided.len(),
                same,
                merged,
                "memory consolidation adjudication complete"
            );
            merged
        }
        Err(error) => {
            tracing::warn!(%error, "applying adjudicated merges failed");
            0
        }
    }
}

#[cfg(test)]
#[path = "garden_adjudicate_tests.rs"]
mod tests;
