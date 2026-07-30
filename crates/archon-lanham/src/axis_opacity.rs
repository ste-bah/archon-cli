//! Opacity axis: feature vector and deviation from the transparent-norm centroid.
//!
//! Split out of `lib.rs` to keep every file under the 500-line gate.

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;
use regex::Regex;

use crate::axis_style::*;
use crate::lexicon::*;

// ── Opacity ──────────────────────────────────────────────────────────────────
pub(crate) const FUNCTION_WORDS_ARR: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
    "did", "will", "would", "shall", "should", "may", "might", "can", "could", "not", "no", "this",
    "that", "these", "those", "it", "its",
];
pub(crate) static FUNCTION_WORDS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| FUNCTION_WORDS_ARR.iter().copied().collect());

pub(crate) const TN_CENTROID: [f64; 4] = [22.5, 4.8, 0.045, 0.48];
pub(crate) const TN_GOLD_MEAN: [f64; 4] = [24.0, 4.7, 0.055, 0.46];
pub(crate) const TN_GOLD_STD: [f64; 4] = [10.0, 0.6, 0.025, 0.05];

pub(crate) static PUNCT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[,;:!?\-—–]").unwrap());
pub(crate) static SENT_TERM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[.!?]+").unwrap());

pub(crate) static META_LINGUISTIC_MARKERS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"(?i)\bthe (?:word|term|phrase|expression|name|label|designation)\b",
        r"(?i)\bso[- ]called\b",
        r"(?i)\bqua\b",
        r"(?i)\bin (?:the )?(?:sense|way) (?:that|in which)\b",
        r"(?i)\bwhat (?:we|I) (?:mean|call|term)\b",
        r"(?i)\bas (?:it were|such)\b",
        r"(?i)\bin other words\b",
        r"(?i)\bthat is to say\b",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});
pub(crate) static OPACITY_CONTENT_MARKERS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"(?i)\b(?:prose|syntax|sentence|paragraph|diction|style|rhetoric|rhythm|cadence)\b",
        r"(?i)\b(?:language|discourse|text|narrative form|literary form|formal properties)\b",
        r"(?i)\b(?:metaphor|figure|trope|irony|allusion|echo|register|voice)\b.*\b(?:itself|here|this|own)\b",
        r"(?i)\b(?:reading|writing|composing|phrasing)\b.*\b(?:as|itself|own|practice)\b",
        r"(?i)\b(?:sound|acoustic|phonetic|alliterat|assonan|rhythm)\b",
        r"(?i)\b(?:foreground|self-conscious|self-referent|draws attention to)\b",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

pub(crate) fn opacity_feature_vector(
    text: &str,
    sentences: &[String],
    words: &[String],
) -> [f64; 4] {
    let word_count = words.len().max(1) as f64;
    let avg_sent_len = if !sentences.is_empty() {
        sentences
            .iter()
            .map(|s| WHITESPACE.split(s).count())
            .sum::<usize>() as f64
            / sentences.len() as f64
    } else {
        15.0
    };
    let avg_word_len = words.iter().map(|w| char_len(w)).sum::<usize>() as f64 / word_count;
    let punct_density = PUNCT_RE.find_iter(text).count() as f64 / word_count;
    let func_count = words
        .iter()
        .filter(|w| FUNCTION_WORDS.contains(w.as_str()))
        .count();
    [
        avg_sent_len,
        avg_word_len,
        punct_density,
        func_count as f64 / word_count,
    ]
}

pub(crate) fn opacity_deviation_from_norm(
    text: &str,
    sentences: &[String],
    words: &[String],
) -> f64 {
    let f = opacity_feature_vector(text, sentences, words);
    let mut dist_sq = 0.0f64;
    for i in 0..4 {
        let std = if TN_GOLD_STD[i] != 0.0 {
            TN_GOLD_STD[i]
        } else {
            1.0
        };
        let z_feature = (f[i] - TN_GOLD_MEAN[i]) / std;
        let z_centroid = (TN_CENTROID[i] - TN_GOLD_MEAN[i]) / std;
        dist_sq += (z_feature - z_centroid).powi(2);
    }
    (dist_sq.sqrt() / 4.0).min(1.0)
}

/// Base opacity (before the fullAnalysis tacit blend).
pub(crate) struct OpacityBase {
    pub(crate) opacity_score: f64,
    pub(crate) self_consciousness_score: f64,
}

pub(crate) fn analyze_opacity(text: &str) -> OpacityBase {
    let sentences = split_sentences(text);
    let sentence_count = sentences.len().max(1) as f64;
    let meta_ling_density = clamp(
        count_markers(&META_LINGUISTIC_MARKERS, text) as f64 / sentence_count / 0.12,
        0.0,
        1.0,
    );
    let content_opacity_density = clamp(
        count_markers(&OPACITY_CONTENT_MARKERS, text) as f64 / sentence_count / 0.25,
        0.0,
        1.0,
    );
    let words = tokenize(text);
    let content_words = get_content_words(&words);

    let mut alliteration_hits = 0usize;
    for i in 0..content_words.len().saturating_sub(2) {
        let a = content_words[i].chars().next();
        if a.is_some()
            && a == content_words[i + 1].chars().next()
            && a == content_words[i + 2].chars().next()
        {
            alliteration_hits += 1;
        }
    }
    let sound_density = clamp(alliteration_hits as f64 / sentence_count / 0.3, 0.0, 1.0);
    let self_consciousness_score = meta_ling_density;
    let polysyndeton_density = clamp(
        AND_RE.find_iter(text).count() as f64 / words.len().max(1) as f64 / 0.06,
        0.0,
        1.0,
    );

    let mut word_freqs: HashMap<&str, usize> = HashMap::new();
    for w in &content_words {
        *word_freqs.entry(w.as_str()).or_insert(0) += 1;
    }
    let repeated = word_freqs.values().filter(|&&c| c >= 3).count();
    let repetition_density = clamp(
        repeated as f64 / content_words.len().max(1) as f64 / 0.04,
        0.0,
        1.0,
    );

    let sent_lens: Vec<usize> = sentences
        .iter()
        .map(|s| WHITESPACE.split(s).count())
        .collect();
    let very_short = sent_lens.iter().filter(|&&l| l <= 5).count();
    let very_long = sent_lens.iter().filter(|&&l| l >= 40).count();
    let extremes_density = clamp(
        (very_short + very_long) as f64 / sentence_count / 0.25,
        0.0,
        1.0,
    );

    let deviation = opacity_deviation_from_norm(text, &sentences, &words);
    let opacity_score = clamp(
        sound_density * 0.15
            + polysyndeton_density * 0.10
            + repetition_density * 0.15
            + extremes_density * 0.10
            + meta_ling_density * 0.15
            + content_opacity_density * 0.10
            + deviation * 0.25,
        0.0,
        1.0,
    );
    OpacityBase {
        opacity_score,
        self_consciousness_score,
    }
}
