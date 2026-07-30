//! Noun/verb and parataxis/hypotaxis axes.
//!
//! Split out of `lib.rs` to keep every file under the 500-line gate.

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::lexicon::*;

/// Outputs of the noun/verb axis.
pub struct NounVerbMetrics {
    pub noun_verb_ratio: f64,
    pub nominalization_density: f64,
    pub prepositional_phrase_density: f64,
    pub be_verb_ratio: f64,
}

/// TS `analyzeNounVerbAxis`, POS-free path: `tag_pos` returns empty in L2, so the
/// POS-confirmed verb set is empty and verb detection falls back to `is_verb`.
pub fn analyze_noun_verb(text: &str) -> NounVerbMetrics {
    let words = tokenize(text);
    let sentences = split_sentences(text);
    let word_count = words.len().max(1) as f64;

    let mut verb_count = 0usize;
    let mut noun_style_count = 0usize;
    let mut be_verb_count = 0usize;
    for w in &words {
        if is_verb(w) {
            verb_count += 1;
            if BE_VERBS.contains(w.as_str()) {
                be_verb_count += 1;
            }
        }
        if is_nominalization(w) {
            noun_style_count += 1;
        }
    }
    let nominalization_density = (noun_style_count as f64 / word_count) * 100.0;
    let be_verb_ratio = if verb_count > 0 {
        be_verb_count as f64 / verb_count as f64
    } else {
        0.0
    };

    // Prepositional phrases per sentence: scan word i in 0..len-1.
    let mut prep_phrase_count = 0usize;
    for sent in &sentences {
        let lower = sent.to_lowercase();
        let sw: Vec<&str> = WHITESPACE.split(&lower).collect();
        for i in 0..sw.len().saturating_sub(1) {
            let cleaned: String = sw[i].chars().filter(|c| c.is_ascii_lowercase()).collect();
            if PREPOSITIONS.contains(cleaned.as_str()) {
                prep_phrase_count += 1;
            }
        }
    }
    let prepositional_phrase_density = if !sentences.is_empty() {
        prep_phrase_count as f64 / sentences.len() as f64
    } else {
        0.0
    };

    let noun_signal = clamp(
        (nominalization_density / 6.0) * 0.45
            + be_verb_ratio * 0.25
            + clamp(prepositional_phrase_density / 5.0, 0.0, 1.0) * 0.30,
        0.0,
        1.0,
    );
    let action_verb_count = (verb_count - be_verb_count) as f64;
    let verb_signal = clamp(action_verb_count / word_count / 0.14, 0.0, 1.0);
    let noun_verb_ratio = clamp((verb_signal - noun_signal + 1.0) / 2.0, 0.0, 1.0);

    NounVerbMetrics {
        noun_verb_ratio,
        nominalization_density,
        prepositional_phrase_density,
        be_verb_ratio,
    }
}

pub(crate) const COORDINATING_CONJ_ARR: &[&str] = &["and", "but", "or", "nor", "for", "yet", "so"];
pub(crate) const SUBORDINATING_CONJ_ARR: &[&str] = &[
    "although",
    "because",
    "since",
    "unless",
    "while",
    "whereas",
    "when",
    "where",
    "if",
    "though",
    "after",
    "before",
    "until",
    "once",
    "whenever",
    "wherever",
    "whether",
    "provided",
    "supposing",
    "inasmuch",
    "insofar",
    "notwithstanding",
    "albeit",
    "lest",
];
pub(crate) const RELATIVE_WORDS_ARR: &[&str] =
    &["which", "who", "whom", "whose", "where", "whereby"];

pub(crate) static COORDINATING_CONJ: Lazy<HashSet<&'static str>> =
    Lazy::new(|| COORDINATING_CONJ_ARR.iter().copied().collect());
pub(crate) static SUBORDINATING_CONJ: Lazy<HashSet<&'static str>> =
    Lazy::new(|| SUBORDINATING_CONJ_ARR.iter().copied().collect());
pub(crate) static RELATIVE_WORDS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| RELATIVE_WORDS_ARR.iter().copied().collect());
// Case-sensitive: matches capitalized sentence-initial conjunctions.
pub(crate) static SENT_INIT_CONJ: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?:And|But|Or|So|Yet|Nor)\b").unwrap());

/// Outputs of the parataxis/hypotaxis axis.
pub struct ParataxisMetrics {
    pub parataxis_hypotaxis_ratio: f64,
    pub coordinating_conjunction_density: f64,
    pub subordinating_conjunction_density: f64,
}

/// TS `analyzeParataxisHypotaxis`, POS-free path: `tag_pos` empty → "that" is never
/// counted as a subordinator (defaults to DT), and relative pronouns are confirmed
/// only via an `is_verb` lookahead. No participial detection (Tier 1).
pub fn analyze_parataxis(text: &str) -> ParataxisMetrics {
    let words = tokenize(text);
    let sentences = split_sentences(text);
    let word_count = words.len().max(1) as f64;

    let mut coord_count = 0usize;
    let mut subord_count = 0usize;
    for w in &words {
        if COORDINATING_CONJ.contains(w.as_str()) {
            coord_count += 1;
        }
        if SUBORDINATING_CONJ.contains(w.as_str()) {
            subord_count += 1;
        }
    }

    // Implicit (relative-clause) subordination, POS-free.
    let mut implicit_subord = 0usize;
    for sent in &sentences {
        let raw: Vec<&str> = WHITESPACE.split(sent).filter(|w| !w.is_empty()).collect();
        let lower: Vec<String> = raw
            .iter()
            .map(|w| {
                w.to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_lowercase() || *c == '\'')
                    .collect::<String>()
            })
            .collect();
        for i in 0..lower.len().saturating_sub(1) {
            if RELATIVE_WORDS.contains(lower[i].as_str()) {
                let end = (i + 4).min(lower.len());
                let has_verb = lower[i + 1..end].iter().any(|lw| is_verb(lw));
                if has_verb {
                    implicit_subord += 1;
                }
            }
            // "that" → DT default → not a subordinator in L2 (no-op).
        }
    }

    // Prepositional-phrase nesting (consecutive prepositions).
    let mut pp_nesting_signal = 0usize;
    for sent in &sentences {
        let lower = sent.to_lowercase();
        let mut consecutive = 0usize;
        let mut max_chain = 0usize;
        for w in WHITESPACE.split(&lower) {
            let cleaned: String = w.chars().filter(|c| c.is_ascii_lowercase()).collect();
            if PREPOSITIONS.contains(cleaned.as_str()) {
                consecutive += 1;
                max_chain = max_chain.max(consecutive);
            } else {
                consecutive = 0;
            }
        }
        if max_chain >= 2 {
            pp_nesting_signal += 1;
        }
    }
    let pp_nesting_density = if !sentences.is_empty() {
        pp_nesting_signal as f64 / sentences.len() as f64
    } else {
        0.0
    };

    // Graded evidence ladder (participial = 0 at Tier 1).
    let high_conf = subord_count + implicit_subord;
    let weighted_total = high_conf as f64; // + 0 * 0.3
    let nesting_bonus_allowed = high_conf >= 1;
    let hypotaxis_boost = if nesting_bonus_allowed {
        clamp(pp_nesting_density / 0.5, 0.0, 1.0) * 0.08
    } else {
        0.0
    };

    let coordinating_conjunction_density = coord_count as f64 / word_count;
    let subordinating_conjunction_density = weighted_total / word_count;

    let mut sent_initial_conj = 0usize;
    for sent in &sentences {
        if SENT_INIT_CONJ.is_match(sent.trim()) {
            sent_initial_conj += 1;
        }
    }
    let n_sent = sentences.len().max(1) as f64;
    let sent_init_conj_density = sent_initial_conj as f64 / n_sent;

    let total_words: usize = sentences.iter().map(|s| WHITESPACE.split(s).count()).sum();
    let avg_sent_len = total_words as f64 / n_sent;
    let short_sent_signal = clamp((15.0 - avg_sent_len) / 10.0, 0.0, 1.0);

    let total_conj = coord_count as f64 + weighted_total;
    let parataxis_hypotaxis_ratio = if total_conj > 0.0 {
        clamp(
            weighted_total / total_conj + hypotaxis_boost
                - sent_init_conj_density * 0.15
                - short_sent_signal * 0.10,
            0.0,
            1.0,
        )
    } else {
        clamp(0.5 + hypotaxis_boost - short_sent_signal * 0.10, 0.0, 1.0)
    };

    ParataxisMetrics {
        parataxis_hypotaxis_ratio,
        coordinating_conjunction_density,
        subordinating_conjunction_density,
    }
}
