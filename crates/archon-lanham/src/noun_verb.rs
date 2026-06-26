//! Noun/verb axis (nominalization, be-verbs, prepositional-phrase density).

use crate::lexical::{BE_VERBS, is_nominalization, is_verb};
use crate::{WHITESPACE, clamp, split_sentences, tokenize};
use once_cell::sync::Lazy;
use std::collections::HashSet;

const PREPOSITIONS_ARR: &[&str] = &[
    "of",
    "in",
    "to",
    "for",
    "with",
    "on",
    "at",
    "from",
    "by",
    "about",
    "as",
    "into",
    "through",
    "during",
    "before",
    "after",
    "above",
    "below",
    "between",
    "under",
    "along",
    "until",
    "without",
    "toward",
    "towards",
    "upon",
    "across",
    "against",
    "among",
    "behind",
    "beyond",
    "within",
    "throughout",
    "beside",
    "besides",
    "despite",
    "concerning",
    "regarding",
    "per",
    "via",
];
pub(crate) static PREPOSITIONS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| PREPOSITIONS_ARR.iter().copied().collect());

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
