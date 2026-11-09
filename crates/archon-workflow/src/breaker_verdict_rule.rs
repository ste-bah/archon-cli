//! Deterministic precedence, text verification, and supplementary allocation.
use super::*;
use crate::breakage_proof::{MutantRun, ProofState, RunOutcome};
use crate::v2::acceptance_stage::coverage::{
    prd_requirement_texts, supplementary_id, supplementary_requirement,
};
use std::collections::BTreeSet;

pub fn requirement_text(text: &str, id: &str) -> Option<String> {
    let texts = prd_requirement_texts(text);
    let acceptance = crate::obligation_ids::acceptance_criteria(text);
    let get = |id: &str| texts.get(id).or_else(|| acceptance.get(id)).cloned();
    get(id).or_else(|| supplementary_requirement(id).and_then(get))
}
pub fn normalise(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let trimmed = word.trim_matches(|c: char| c.is_ascii_punctuation());
            if trimmed.is_empty() { word } else { trimmed }
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}
/// Longest contiguous requirement words present in the reply, preserving their spelling.
pub fn requirement_span(text: &str, breaks: &str) -> Option<String> {
    let words: Vec<_> = text.split_whitespace().collect();
    let reply = normalise(breaks);
    let reply_words: Vec<_> = reply.split_whitespace().collect();
    for len in (3..=words.len()).rev() {
        for span in words.windows(len) {
            let original = span.join(" ");
            let normalised = normalise(&original);
            let candidate: Vec<_> = normalised.split_whitespace().collect();
            if candidate.len() >= 3
                && reply_words
                    .windows(candidate.len())
                    .any(|window| window == candidate)
            {
                return Some(original);
            }
        }
    }
    None
}
pub fn verified_quote(text: &str, quote: &str) -> bool {
    requirement_span(text, quote).is_some_and(|span| normalise(&span) == normalise(quote))
}
pub fn survivor<'a>(id: &str, runs: impl IntoIterator<Item = &'a MutantRun>) -> bool {
    let runs: Vec<_> = runs.into_iter().filter(|r| r.mutant_id == id).collect();
    !runs
        .iter()
        .any(|r| matches!(r.outcome, RunOutcome::Unviable { .. }))
        && !runs
            .iter()
            .any(|r| r.counted && matches!(r.outcome, RunOutcome::Killed { .. }))
        && runs
            .iter()
            .any(|r| matches!(r.outcome, RunOutcome::Survived { .. }))
}
pub fn real_survivor(pair: &PairVerdict, mutant: &AuthoredMutant, distinct: bool) -> bool {
    mutant.kind == MutantKind::BodyGut
        || pair
            .equivalence
            .iter()
            .find(|m| m.mutant_id == mutant.mutant_id)
            .is_some_and(|m| {
                m.capped
                    || if distinct {
                        !m.marked
                            && m.first.answer != EquivalenceAnswer::Unavailable
                            && m.second.answer != EquivalenceAnswer::Unavailable
                    } else {
                        m.first.answer == EquivalenceAnswer::NotEquivalent
                    }
            })
}
pub struct Decision<'a> {
    pub refused: bool,
    pub has_text: bool,
    pub states: &'a [ProofState],
    pub pair: &'a PairVerdict,
    pub runs: &'a [MutantRun],
    pub distinct: bool,
}
pub fn decide(input: Decision<'_>) -> Verdict {
    let Decision {
        refused,
        has_text,
        states,
        pair,
        runs,
        distinct,
    } = input;
    if refused {
        return Verdict::Refused;
    }
    for (state, verdict) in [
        (ProofState::NoPassingCheck, Verdict::NoPassingCheck),
        (
            ProofState::NoCountableDeliverable,
            Verdict::NoCountableDeliverable,
        ),
    ] {
        if states.contains(&state) {
            return verdict;
        }
    }
    if !has_text {
        return Verdict::RequirementTextMissing;
    }
    let survivors: Vec<_> = pair
        .mutants
        .iter()
        .filter(|m| survivor(&m.mutant_id, runs))
        .collect();
    if survivors.iter().any(|m| real_survivor(pair, m, distinct)) {
        return Verdict::WeakTest;
    }
    if pair
        .relevance
        .iter()
        .any(|m| m.cause == Some(UnavailableCause::Budget))
        || pair.equivalence.iter().any(|m| {
            m.first.cause == Some(UnavailableCause::Budget)
                || m.second.cause == Some(UnavailableCause::Budget)
        })
        || pair.attempts.iter().any(|a| {
            matches!(
                a.outcome,
                AttemptOutcome::Unavailable {
                    cause: UnavailableCause::Budget,
                    ..
                }
            )
        })
        || runs
            .iter()
            .any(|r| matches!(r.outcome, RunOutcome::BudgetExhausted))
        || states.contains(&ProofState::BudgetExhausted)
    {
        return Verdict::BudgetExhausted;
    }
    if pair.mutants.is_empty() {
        if pair
            .attempts
            .iter()
            .any(|a| matches!(a.outcome, AttemptOutcome::Discarded { .. }))
        {
            return Verdict::BreakerUnusable;
        }
        if pair
            .attempts
            .iter()
            .flat_map(|a| &a.refusals)
            .any(|r| r == "view_too_large")
        {
            return Verdict::Unmutable;
        }
        return if pair
            .attempts
            .iter()
            .any(|a| matches!(a.outcome, AttemptOutcome::Unusable { .. }))
        {
            Verdict::BreakerUnusable
        } else {
            Verdict::BreakerUnavailable
        };
    }
    if pair.attempts.iter().any(|a| {
        matches!(
            a.outcome,
            AttemptOutcome::Unavailable {
                cause: UnavailableCause::Transient,
                ..
            }
        )
    }) {
        return Verdict::BreakerUnavailable;
    }
    if !pair.mutants.iter().any(|m| m.kind == MutantKind::Behaviour) {
        return Verdict::BreakerUnusable;
    }
    // A single identity cannot establish equivalence; keep this undecided.
    if !distinct
        && survivors.iter().any(|s| {
            pair.equivalence.iter().any(|m| {
                m.mutant_id == s.mutant_id
                    && matches!(
                        m.first.answer,
                        EquivalenceAnswer::Equivalent | EquivalenceAnswer::Unsure
                    )
            })
        })
    {
        return Verdict::UndecidedEquivalence;
    }
    let relevant = pair
        .relevance
        .iter()
        .any(|m| m.relevance == Relevance::Relevant);
    let gut = runs
        .iter()
        .any(|r| r.kind == MutantKind::BodyGut && r.counted);
    let unresolved_survivor = survivors.iter().any(|s| {
        pair.equivalence
            .iter()
            .find(|m| m.mutant_id == s.mutant_id)
            .is_none_or(|m| {
                m.first.answer == EquivalenceAnswer::Unavailable
                    || distinct && m.second.answer == EquivalenceAnswer::Unavailable
            })
            && !pair
                .equivalence
                .iter()
                .any(|m| m.mutant_id == s.mutant_id && m.capped)
    });
    let all_decided = pair.mutants.iter().all(|m| {
        runs.iter().any(|r| {
            r.mutant_id == m.mutant_id
                && matches!(
                    r.outcome,
                    RunOutcome::Killed { .. }
                        | RunOutcome::Survived { .. }
                        | RunOutcome::Unviable { .. }
                        | RunOutcome::Inapplicable { .. }
                )
        })
    });
    if !pair.survivors.is_empty() {
        return Verdict::HostUnrun;
    }
    if relevant && gut && !all_decided {
        return Verdict::HostUnrun;
    }
    if relevant && gut && !unresolved_survivor {
        return Verdict::DoneCandidate;
    }
    if pair
        .relevance
        .iter()
        .any(|m| m.relevance == Relevance::Unavailable)
        || survivors.iter().any(|s| {
            pair.equivalence
                .iter()
                .find(|m| m.mutant_id == s.mutant_id)
                .is_none_or(|m| {
                    m.first.answer == EquivalenceAnswer::Unavailable
                        || distinct && m.second.answer == EquivalenceAnswer::Unavailable
                })
        })
    {
        return Verdict::JudgeUnavailable;
    }
    for (state, verdict) in [
        (ProofState::BudgetExhausted, Verdict::BudgetExhausted),
        (ProofState::HostUnrun, Verdict::HostUnrun),
        (ProofState::Unverified, Verdict::Unverified),
    ] {
        if states.contains(&state) {
            return verdict;
        }
    }
    if !relevant
        && runs
            .iter()
            .any(|r| r.counted && r.kind == MutantKind::Behaviour)
    {
        return Verdict::UndecidedRelevance;
    }
    if runs.is_empty() {
        return Verdict::HostUnrun;
    }
    if pair
        .mutants
        .iter()
        .filter(|m| m.kind == MutantKind::Behaviour)
        .all(|m| {
            runs.iter().any(|r| {
                r.mutant_id == m.mutant_id
                    && matches!(
                        r.outcome,
                        RunOutcome::Unviable { .. } | RunOutcome::Inapplicable { .. }
                    )
            })
        })
    {
        return Verdict::NoViableBehaviourMutant;
    }
    if !gut {
        return Verdict::BodyGutUnavailable;
    }
    Verdict::UndecidedRelevance
}
pub fn mint(id: &str, used: &mut BTreeSet<String>) -> Result<String, String> {
    let base = supplementary_id(id);
    if supplementary_requirement(&base) != Some(id) {
        return Err(format!("sup_id_ambiguous: {id}"));
    }
    let mut candidate = base.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-R{n}");
        n += 1;
    }
    if supplementary_requirement(&candidate) != Some(id) {
        return Err(format!("sup_id_ambiguous: {id}"));
    }
    used.insert(candidate.clone());
    Ok(candidate)
}

/// Construct line counts and change ranges from the exact before/after text.
pub fn edit_patch(key: &str, before: &str, after: &str) -> String {
    let old = patch_path("a", key);
    let new = patch_path("b", key);
    format!(
        "diff --git {old} {new}\n{}",
        similar::TextDiff::from_lines(before, after)
            .unified_diff()
            .header(&old, &new)
    )
}
/// Encode header paths without allowing names to create protocol lines.
pub fn patch_path(side: &str, key: &str) -> String {
    use std::fmt::Write;
    let path = format!("{side}/{key}");
    if path
        .bytes()
        .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        return path;
    }
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        if byte == b'"' || byte == b'\\' {
            quoted.push('\\');
            quoted.push(char::from(byte));
        } else if byte.is_ascii_graphic() || byte == b' ' {
            quoted.push(char::from(byte));
        } else {
            write!(quoted, "\\{byte:03o}").expect("writing a string");
        }
    }
    quoted.push('"');
    quoted
}
