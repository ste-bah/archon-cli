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

#[path = "repeat_tool_novelty_json.rs"]
mod json;

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

    /// Still spinning AFTER the reminder: the reminder was delivered and the
    /// window is still full of the same answers. The reminder alone stopped
    /// nothing (Issue-213 C2d); this is what the runner's progress check reads
    /// to decide whether to end the session.
    pub(crate) fn stalled(&self) -> bool {
        self.warned && self.recent.len() == NOVELTY_WINDOW && self.distinct() < MIN_DISTINCT
    }
}

/// The prefix of the error a workflow session ends with when it kept getting
/// the same answers after being told so AND its worktree did not change
/// (Issue-213 C2d). `archon_workflow::error::NO_PROGRESS_STOP_MARKER` pins the
/// same text; the write layer reads it as a host interruption, so the partial
/// work is kept and re-asked rather than judged.
pub const NO_PROGRESS_STOP_MARKER: &str = "no-progress stop:";

/// The text a session stopped for no progress ends with.
///
/// Carries no free-running count: the transient-error classifier matches
/// digit strings such as `500` anywhere in an error, and a stop re-asked as a
/// provider blink would restart the very loop it ended.
pub fn no_progress_stop_message(rounds: u32) -> String {
    format!(
        "{NO_PROGRESS_STOP_MARKER} the last {NOVELTY_WINDOW} tool calls returned fewer than \
         {MIN_DISTINCT} distinct results after the agent was told so, and the working tree did \
         not change over the {rounds} tool rounds since"
    )
}

/// The text a session stopped for OSCILLATION ends with (Issue-213 C2): its
/// working tree kept returning to states it had already left. The same
/// marker as [`no_progress_stop_message`], so every consumer of the stop
/// treats both alike; the verdict word after it says which one tripped.
pub fn oscillation_stop_message(returns: u32) -> String {
    format!(
        "{NO_PROGRESS_STOP_MARKER} oscillation: the working tree returned to a state it had \
         already left {returns} times in a row without reaching a new one"
    )
}

/// Tokens that differ on every attempt while meaning the same thing.
///
/// An identity compared ACROSS attempts must have per-attempt entropy stripped
/// first, or two identical answers fingerprint differently and the detector
/// never fires — which is the same defect as the arguments-side one this
/// exists to fix, moved to the other side of the call. A temporary directory
/// carries a fresh id per run; a duration or a timestamp changes by itself.
fn strip_per_attempt_entropy(text: &str) -> String {
    let json_normalised = normalise_json_lines(text);
    let text = json_normalised.as_str();
    let mut out = String::with_capacity(text.len());
    let tokens: Vec<&str> = text.split_inclusive(char::is_whitespace).collect();
    let mut request_id_value = false;
    let mut consumed_through = 0;
    for (index, token) in tokens.iter().enumerate() {
        if index < consumed_through {
            continue;
        }
        // Date and clock are separate whitespace-delimited tokens in common
        // logs; consume both as one timestamp while keeping the separator.
        let current = token.trim_end();
        if is_month(current) {
            let (day_index, clock_index) = if tokens
                .get(index + 1)
                .is_some_and(|day| day.trim().is_empty())
            {
                (index + 2, index + 3)
            } else {
                (index + 1, index + 2)
            };
            if let (Some(day), Some(clock)) = (tokens.get(day_index), tokens.get(clock_index)) {
                let day_text = day.trim();
                let clock_text = clock
                    .trim()
                    .trim_matches(|ch| matches!(ch, '[' | ']' | '(' | ')'));
                if day_text.bytes().all(|b| b.is_ascii_digit()) && is_clock(clock_text) {
                    out.push_str("<volatile> <time>");
                    let clock_end = clock
                        .trim_end()
                        .trim_end_matches(['[', ']', '(', ')'])
                        .len();
                    out.push_str(&clock[clock_end..]);
                    consumed_through = clock_index + 1;
                    continue;
                }
            }
        }
        let date = current.trim_matches(|ch| matches!(ch, '[' | ']' | '(' | ')'));
        if is_date_only(date) {
            if let Some(next) = tokens.get(index + 1) {
                let next_core = next
                    .trim()
                    .trim_matches(|ch| matches!(ch, '[' | ']' | '(' | ')'));
                if is_clock(next_core) {
                    out.push_str("<volatile> <time>");
                    out.push_str(&next[next.trim().len()..]);
                    consumed_through = index + 2;
                    continue;
                }
            }
        }
        let trimmed = token.trim_end();
        let tail = &token[trimmed.len()..];
        if let Some(normalised) = json::normalise_compact_json(trimmed) {
            out.push_str(&normalised);
        } else if TEMP_MARKERS.iter().any(|marker| trimmed.contains(marker)) {
            request_id_value = false;
            out.push_str("<scratch>");
        } else if let Some(normalised) = normalise_timestamp_or_id(trimmed, &mut request_id_value) {
            out.push_str(&normalised);
        } else if is_elapsed_or_numeric(trimmed) {
            out.push_str("<n>");
        } else {
            out.push_str(trimmed);
        }
        out.push_str(tail);
        if tail.contains('\n') || tail.contains('\r') {
            request_id_value = false;
        }
    }
    out
}

fn normalise_json_lines(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let ending = &line[content.len()..];
        if let Some(normalised) = json::normalise_compact_json(content.trim()) {
            let leading = &content[..content.len() - content.trim_start().len()];
            let trailing = &content[content.trim_end().len()..];
            output.push_str(leading);
            output.push_str(&normalised);
            output.push_str(trailing);
        } else {
            output.push_str(content);
        }
        output.push_str(ending);
    }
    output
}

/// Replace recognized timestamp and request identity tokens while retaining
/// surrounding punctuation and whitespace. Numbers elsewhere remain intact.
fn normalise_timestamp_or_id(token: &str, request_id_value: &mut bool) -> Option<String> {
    let start = token
        .char_indices()
        .find(|(_, ch)| ch.is_ascii_alphanumeric())
        .map_or(token.len(), |(index, _)| index);
    let end = token
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_ascii_alphanumeric())
        .map_or(start, |(index, ch)| index + ch.len_utf8());
    let core = &token[start..end];
    if core.is_empty() {
        return None;
    }
    let lower = core.to_ascii_lowercase();
    if is_request_id_key(&lower) {
        *request_id_value = true;
        return Some(token.to_string());
    }
    if *request_id_value {
        *request_id_value = false;
        return Some(format!("{}<request-id>{}", &token[..start], &token[end..]));
    }
    // Inline labelled forms, including JSON's "key":"value" spelling.
    let labelled = lower
        .split_once('=')
        .or_else(|| lower.split_once(':'))
        .filter(|(key, _)| is_request_id_key(key.trim_matches('"')))
        .is_some();
    if labelled {
        return Some(format!("{}<request-id>{}", &token[..start], &token[end..]));
    }
    if is_timestamp(core) {
        return Some(format!("{}<volatile>{}", &token[..start], &token[end..]));
    }
    None
}

fn is_request_id_key(key: &str) -> bool {
    matches!(
        key,
        "request_id"
            | "request-id"
            | "requestid"
            | "x-request-id"
            | "trace_id"
            | "trace-id"
            | "traceid"
            | "correlation_id"
            | "correlation-id"
            | "correlationid"
            | "span_id"
            | "span-id"
            | "spanid"
    )
}

fn is_timestamp(token: &str) -> bool {
    is_iso_datetime(token) || is_common_log_timestamp(token)
}

fn is_iso_datetime(token: &str) -> bool {
    let bytes = token.as_bytes();
    if bytes.len() < 19
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't' | b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || !all_ascii_digits(&bytes[0..4])
        || !all_ascii_digits(&bytes[5..7])
        || !all_ascii_digits(&bytes[8..10])
        || !all_ascii_digits(&bytes[11..13])
        || !all_ascii_digits(&bytes[14..16])
        || !all_ascii_digits(&bytes[17..19])
    {
        return false;
    }

    let mut rest = &bytes[19..];
    if rest.first() == Some(&b'.') {
        let fraction_len = rest[1..].iter().take_while(|b| b.is_ascii_digit()).count();
        if fraction_len == 0 {
            return false;
        }
        rest = &rest[fraction_len + 1..];
    }
    match rest {
        [] => true,
        [b'Z' | b'z'] => true,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2]
            if sign.is_ascii()
                && all_ascii_digits(&[*h1, *h2])
                && all_ascii_digits(&[*m1, *m2]) =>
        {
            true
        }
        _ => false,
    }
}

fn is_common_log_timestamp(token: &str) -> bool {
    let Some((date, clock_and_zone)) = token.split_once(':') else {
        return false;
    };
    let Some((day, month_and_year)) = date.split_once('/') else {
        return false;
    };
    let Some((month, year)) = month_and_year.split_once('/') else {
        return false;
    };
    let (clock, zone) = match clock_and_zone.find(['+', '-']) {
        Some(index) => (&clock_and_zone[..index], Some(&clock_and_zone[index..])),
        None => (clock_and_zone, None),
    };
    let mut clock_fields = clock.split(':');
    let hour = clock_fields.next().unwrap_or_default();
    let minute = clock_fields.next().unwrap_or_default();
    let second = clock_fields.next().unwrap_or_default();
    day.len() == 2
        && all_ascii_digits(day.as_bytes())
        && is_month(month)
        && year.len() == 4
        && all_ascii_digits(year.as_bytes())
        && hour.len() == 2
        && minute.len() == 2
        && second.len() == 2
        && all_ascii_digits(hour.as_bytes())
        && all_ascii_digits(minute.as_bytes())
        && all_ascii_digits(second.as_bytes())
        && clock_fields.next().is_none()
        && zone.is_none_or(is_common_log_zone)
}

fn is_common_log_zone(zone: &str) -> bool {
    let bytes = zone.as_bytes();
    bytes.len() == 5 && matches!(bytes[0], b'+' | b'-') && all_ascii_digits(&bytes[1..])
}

fn all_ascii_digits(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

fn is_date_only(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
}

fn is_clock(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() >= 5
        && bytes.get(2) == Some(&b':')
        && bytes.get(0).is_some_and(u8::is_ascii_digit)
        && bytes.get(1).is_some_and(u8::is_ascii_digit)
        && (bytes.len() == 5 || (bytes.len() >= 8 && bytes.get(5) == Some(&b':')))
}

fn is_month(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "jan"
            | "feb"
            | "mar"
            | "apr"
            | "may"
            | "jun"
            | "jul"
            | "aug"
            | "sep"
            | "oct"
            | "nov"
            | "dec"
    )
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
         approach, or state what is blocking you and stop. In a workflow run \
         the host ends a session that keeps this up without changing any file."
    )
}

#[cfg(test)]
#[path = "repeat_tool_novelty_tests.rs"]
mod tests;
