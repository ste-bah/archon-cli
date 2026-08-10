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
//!
//! `/garden` adjudicates unconditionally. Automatic session-start consolidation
//! goes through [`spawn_review_band_adjudication`] instead, which is opt-in and
//! waits for the band to be worth a round-trip -- without it the review band has
//! no automatic resolution at all and grows for as long as nobody types
//! `/garden`.
//!
//! The automatic pass is DETACHED, not awaited. Consolidation runs during
//! session bootstrap, before the TUI is up and before the user can type;
//! awaiting an LLM round-trip there put the provider's latency between launching
//! Archon and the first prompt. Spawning instead costs the session nothing, and
//! nothing downstream depends on the verdict: the review band writes nothing, so
//! a pass that is slow, fails, or never returns leaves exactly the state a pass
//! that never ran would have left, and the next consolidation re-derives the
//! same pairs.

use std::sync::Arc;

use archon_memory::MemoryTrait;
use archon_memory::garden::{Adjudication, ReviewPair};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

/// Most pairs judged in one pass.
///
/// A bound on cost, and on blast radius: a run that wants to merge more than
/// this has probably found a systemic problem rather than a set of duplicates,
/// and should be looked at before it reshapes the graph.
const MAX_PAIRS_PER_RUN: usize = 20;

/// Longest a background adjudication may run before it is abandoned.
///
/// This no longer bounds any latency the user can feel — the automatic pass is
/// detached, so nothing waits on it. What it bounds is the task's own lifetime:
/// a provider that accepts the request and never answers would otherwise hold
/// the client, the store handle, and the pending batch alive for the rest of the
/// session. Abandoning the call loses nothing, because the band writes nothing:
/// the pairs are re-derived and offered again by the next consolidation.
///
/// Two minutes rather than the 45s this was when the call sat on the startup
/// path. Nobody is waiting, so cutting a slow-but-working provider off early
/// buys nothing and throws away a batch that was about to be judged.
const BACKGROUND_ADJUDICATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

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

#[cfg(test)]
pub(crate) fn parse_verdicts(response: &str, pair_count: usize) -> Vec<Adjudication> {
    parse_verdicts_counted(response, pair_count).0
}

/// Parse the verdict lines back into one decision per pair, also reporting how many lines were
/// actually understood.
///
/// Anything not explicitly `SAME` is `Distinct`: a truncated reply, a missing
/// line, or commentary the model added all fall through to "do not merge".
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
            // `understood` is logged on success as well as failure. Reporting
            // it only on a shortfall meant a healthy run was identified by the
            // ABSENCE of a warning, and absence is not evidence -- a filtered
            // log, a changed level, or a swallowed line all look like health.
            tracing::info!(
                judged = decided.len(),
                understood,
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

/// Start judging the review band in the background, if configured to.
///
/// SYNCHRONOUS on purpose, and returns before the provider has been asked
/// anything. The caller is session bootstrap; every `.await` it takes is time
/// the user spends looking at a splash screen instead of a prompt, and this is
/// the only thing on that path that would call a model at all.
///
/// Returns the detached task, or `None` when the trigger did not fire — which is
/// the default. Production drops the handle (dropping detaches; the task runs on
/// regardless); it is returned so tests can join the pass instead of racing it.
/// Separate from [`adjudicate_and_apply`] so the policy — opted in, and only
/// once the band is worth a round-trip — lives beside the cost it authorises
/// rather than inside the session bootstrap.
///
/// `notify` receives one line if and only if the pass actually merged something.
/// Merges reshape stored memories behind the user's back, and by the time a
/// verdict arrives the startup panel has already been drawn saying those pairs
/// were still outstanding; without this, the correction never lands anywhere the
/// user looks. A pass that merges nothing says nothing, because "the background
/// judged your memories and changed none of them" on every launch is a line
/// people learn to skip.
pub(crate) fn spawn_review_band_adjudication(
    garden: &archon_memory::garden::GardenConfig,
    client: Arc<dyn archon_pipeline::runner::LlmClient>,
    memory: Arc<dyn MemoryTrait>,
    pairs: Vec<ReviewPair>,
    model: String,
    notify: Option<TuiEventSender>,
) -> Option<tokio::task::JoinHandle<usize>> {
    if !archon_memory::garden::should_auto_adjudicate(garden, pairs.len()) {
        return None;
    }
    let pending = pairs.len();
    tracing::info!(
        pending,
        threshold = garden.auto_adjudicate_min_pairs,
        "garden: review band reached the automatic adjudication threshold; judging in the background"
    );
    Some(tokio::spawn(async move {
        let merged = match tokio::time::timeout(
            BACKGROUND_ADJUDICATION_TIMEOUT,
            adjudicate_and_apply(client, memory, pairs, model),
        )
        .await
        {
            Ok(merged) => merged,
            Err(_) => {
                tracing::warn!(
                    pending,
                    timeout_secs = BACKGROUND_ADJUDICATION_TIMEOUT.as_secs(),
                    "garden: background adjudication timed out; nothing merged"
                );
                0
            }
        };
        if merged > 0
            && let Some(tui) = notify
            && let Err(error) = tui
                .send_async(TuiEvent::TextDelta(format!(
                    "\nMemory garden: {merged} of {pending} pair(s) awaiting review \
                     merged after background judgement.\n"
                )))
                .await
        {
            // Logged rather than retried. The merges are already applied and
            // already in the tracing record; a closed TUI channel means the
            // session is going away, and there is nowhere left to say it.
            tracing::warn!(%error, "garden: background adjudication result not delivered to the TUI");
        }
        merged
    }))
}

#[cfg(test)]
#[path = "garden_adjudicate_tests/mod.rs"]
mod tests;
