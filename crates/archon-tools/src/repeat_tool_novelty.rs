//! Noticing an agent that keeps calling and keeps learning nothing.
//!
//! The run counter beside this one catches a model reissuing a call
//! byte-for-byte. It cannot catch the failure that actually cost six hours: an
//! agent that varies its calls trivially and gets the same answer every time.
//! Observed live as `grep discount`, then `grep -i discount`, then
//! `grep "discount"` — three different argument strings, three resets of the
//! run counter, one piece of information. The guard never fired once.
//!
//! Normalising the arguments does not fix it. Quotes can be stripped, but `-i`
//! genuinely changes the call, and a rule that ignored flags would have to
//! decide that `rm` and `rm -rf` are the same thing. The arguments are the
//! wrong place to look.
//!
//! What separates exploring from spinning is whether the answers are new. So
//! this watches the RESULTS: over a window of recent calls, how many distinct
//! ones came back. Ten calls returning two distinct answers is spinning
//! whatever the arguments said, and an unchanged answer from a tool that
//! MUTATES something is the strongest form of it — an edit that is not taking.
//!
//! Deliberately no knowledge of any tool, language or project: it compares
//! digests of whatever came back.

use std::collections::VecDeque;

/// Calls to weigh together. Long enough that ordinary work — read, edit, test,
/// read — is not mistaken for a loop, short enough to notice within a stage.
pub(crate) const NOVELTY_WINDOW: usize = 8;

/// Distinct answers a full window must contain. Below this, the agent is
/// getting the same thing back however it phrases the question.
///
/// Two, not three: a legitimate read-then-verify cycle alternates between two
/// answers, and calling that a loop would fire on healthy work. Fewer than two
/// distinct answers in eight calls has no benign reading.
pub(crate) const MIN_DISTINCT: usize = 2;

/// Recent answers for one agent's chain.
#[derive(Debug, Default)]
pub(crate) struct ResultNovelty {
    recent: VecDeque<u64>,
    /// Set once per stretch of low novelty, so a spinning agent is told once
    /// rather than on every call for as long as it keeps spinning.
    warned: bool,
}

impl ResultNovelty {
    /// Record what came back, and say whether this is the moment to speak up.
    pub(crate) fn observe(&mut self, digest: u64) -> bool {
        if self.recent.len() == NOVELTY_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(digest);
        if self.recent.len() < NOVELTY_WINDOW {
            // Never judge a partial window. Doing so would fire on the first
            // few calls of every stage, which is when repetition is cheapest
            // and most likely to be deliberate.
            return false;
        }
        let distinct = self.distinct();
        if distinct >= MIN_DISTINCT {
            self.warned = false;
            return false;
        }
        if self.warned {
            return false;
        }
        self.warned = true;
        true
    }

    fn distinct(&self) -> usize {
        let mut seen: Vec<u64> = Vec::with_capacity(self.recent.len());
        for digest in &self.recent {
            if !seen.contains(digest) {
                seen.push(*digest);
            }
        }
        seen.len()
    }

    pub(crate) fn distinct_in_window(&self) -> usize {
        self.distinct()
    }
}

/// Tokens that differ on every attempt while meaning the same thing.
///
/// An identity compared ACROSS attempts must have per-attempt entropy stripped
/// first, or two identical answers fingerprint differently and the detector
/// never fires — which is the same defect as the arguments-side one this
/// exists to fix, moved to the other side of the call. A temporary directory
/// carries a fresh id per run; a duration or a timestamp changes by itself.
fn strip_per_attempt_entropy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        let tail = &token[trimmed.len()..];
        if TEMP_MARKERS.iter().any(|marker| trimmed.contains(marker)) {
            out.push_str("<scratch>");
        } else if is_elapsed_or_numeric(trimmed) {
            out.push_str("<n>");
        } else {
            out.push_str(trimmed);
        }
        out.push_str(tail);
    }
    out
}

/// Directories whose names embed a per-run identifier. Deliberately paths, not
/// project or language knowledge — every toolchain's scratch lands in one.
const TEMP_MARKERS: &[&str] = &["/tmp/", "/var/folders/", "/private/tmp/", "\\Temp\\"];

/// A token that reads as a measured quantity rather than an identity.
///
/// `finished in 3.86s` and `finished in 4.01s` are the same answer; left alone,
/// every suite result an agent re-reads looks new.
///
/// Deliberately NOT every number. A bare `12:` is a line number, and collapsing
/// it would make two different matches in one file read as the same answer —
/// over-collapsing costs a false stall on search-heavy work, which is worse
/// than missing one, because a detector that cries wolf gets ignored. So a
/// token qualifies only when it carries a unit suffix or a decimal point, which
/// is what distinguishes a duration from a position.
fn is_elapsed_or_numeric(token: &str) -> bool {
    let core = token.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let had_unit = core.len() < token.len();
    let numeric = !core.is_empty()
        && core
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ':' || c == '-')
        && core.chars().any(|c| c.is_ascii_digit());
    numeric && (had_unit || core.contains('.'))
}

/// A stable digest of what a call returned, with per-attempt entropy removed.
///
/// FNV-1a over the normalised bytes, matching the fingerprint the bash
/// heartbeat already uses, so two places that describe "the same output" agree
/// on what that means. Only equality is ever asked of it.
pub(crate) fn result_digest(text: &str) -> u64 {
    let normalised = strip_per_attempt_entropy(text);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normalised.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// What the model is told when its answers stop changing.
///
/// Names the count rather than accusing it of looping: the agent may have a
/// reason, and the reminder's job is to make the fact visible at the moment it
/// can still act on it.
pub(crate) fn novelty_reminder(tool: &str, distinct: usize) -> String {
    format!(
        "The last {NOVELTY_WINDOW} tool calls returned only {distinct} distinct \
         result(s); the most recent used {tool}. Varying the arguments is not \
         producing new information. Re-read what you already have, change \
         approach, or state what is blocking you and stop."
    )
}

#[cfg(test)]
#[path = "repeat_tool_novelty_tests.rs"]
mod tests;
