//! Prosody axes: periodic/running, voice, and register markedness.

use crate::lexical::latinate_germanic_ratio;
use crate::parataxis::SUBORDINATING_CONJ;
use crate::{AND_RE, WHITESPACE, char_len, clamp, count_markers, split_sentences, tokenize};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

// ── Data + regexes for the remaining axes ────────────────────────────────────
const FORMAL_MARKERS_ARR: &[&str] = &[
    "furthermore",
    "moreover",
    "nevertheless",
    "notwithstanding",
    "consequently",
    "subsequently",
    "henceforth",
    "whereby",
    "wherein",
    "therein",
    "thereof",
    "herein",
    "aforementioned",
    "heretofore",
    "thus",
    "hence",
    "accordingly",
    "indeed",
    "nonetheless",
];
static FORMAL_MARKERS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| FORMAL_MARKERS_ARR.iter().copied().collect());

static CONTRACTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[A-Za-z0-9_]+'[A-Za-z0-9_]+\b").unwrap());
static PASSIVE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:is|are|was|were|been|be)\s+[A-Za-z0-9_]+ed\b").unwrap());
static FILLER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:of course|as you know|it is|there is|there are|it was|it has been|in terms of|with respect to|in connection with|pursuant to|shall be|may be|provided that|in accordance)\b").unwrap()
});
static IMPERSONAL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?:The |It |This |These |That |Those |Such |An? )").unwrap());
static PARTICIPLE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Z][a-z]+(?:ing|ed)\b").unwrap());

static PERSONALITY_MARKERS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"(?i)\bI (?:believe|think|argue|contend|suggest|maintain|hold)\b",
        r"(?i)\b(?:my|our) (?:view|position|argument|contention|claim)\b",
        r"(?i)\bit (?:seems|appears) (?:to me|clear|evident)\b",
        r"(?i)\b(?:crucially|importantly|strikingly|remarkably|notably)\b",
        r"(?i)\b(?:indeed|surely|certainly|undoubtedly|plainly)\b",
        r"(?i)\bwe (?:shall|will|must|can|cannot)\b",
        r"(?i)\blet us\b",
        r"(?i)\b(?:never|always|forever)\b\s+\b(?:shall|will|must|can|cannot|again|forget)\b",
        r"(?i)\b(?:hear me|listen|mark my words|remember)\b",
        r"(?i)\b(?:she|he) (?:breathed|listened|watched|felt|saw|heard|tasted|smelled)\b",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

/// Outputs of the periodic/running axis.
pub struct PeriodicMetrics {
    pub periodic_running_ratio: f64,
    pub pre_main_verb_clause_count: f64,
}

pub fn analyze_periodic(text: &str) -> PeriodicMetrics {
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return PeriodicMetrics {
            periodic_running_ratio: 0.5,
            pre_main_verb_clause_count: 0.0,
        };
    }
    let mut periodic_signals = 0.0f64;
    let mut running_signals = 0.0f64;
    let mut total_pre_main = 0usize;
    for sent in &sentences {
        let words: Vec<&str> = WHITESPACE.split(sent).collect();
        if words.len() < 3 {
            running_signals += 1.0;
            continue;
        }
        let first_words: Vec<String> = words
            .iter()
            .take(5)
            .map(|w| {
                w.to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_lowercase())
                    .collect()
            })
            .collect();
        let starts_with_subord = first_words
            .iter()
            .any(|w| SUBORDINATING_CONJ.contains(w.as_str()));
        let starts_with_participle = PARTICIPLE_RE.is_match(words[0]);
        let mut early_commas = 0usize;
        let mut late_commas = 0usize;
        let midpoint = words.len() as f64 / 2.0;
        for (i, w) in words.iter().enumerate() {
            if w.ends_with(',') {
                if (i as f64) < midpoint {
                    early_commas += 1;
                } else {
                    late_commas += 1;
                }
            }
        }
        let is_short = words.len() < 10;
        let is_long = words.len() >= 30;
        let has_coord_chain = AND_RE.find_iter(sent).count() >= 3;
        if starts_with_subord || starts_with_participle || early_commas > late_commas {
            periodic_signals += 1.0;
            if starts_with_subord || starts_with_participle {
                total_pre_main += 1;
            }
        } else if is_short {
            running_signals += 1.2;
        } else if is_long && has_coord_chain {
            running_signals += 1.0;
        } else {
            running_signals += 1.0;
        }
    }
    let total = periodic_signals + running_signals;
    let periodic_running_ratio = if total > 0.0 {
        clamp(running_signals / total, 0.0, 1.0)
    } else {
        0.5
    };
    let pre_main_verb_clause_count = total_pre_main as f64 / sentences.len() as f64;
    PeriodicMetrics {
        periodic_running_ratio,
        pre_main_verb_clause_count,
    }
}

/// Outputs of the voice axis (entropy/contrast/terminal D1-D3 are computed-but-unused in TS, so omitted).
pub struct VoiceMetrics {
    pub voice_score: f64,
    pub dynamic_range: f64,
}

pub fn analyze_voice(text: &str) -> VoiceMetrics {
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return VoiceMetrics {
            voice_score: 0.5,
            dynamic_range: 0.0,
        };
    }
    let n = sentences.len() as f64;
    let lengths: Vec<f64> = sentences
        .iter()
        .map(|s| WHITESPACE.split(s).count() as f64)
        .collect();
    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
    let variance =
        (lengths.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lengths.len() as f64).sqrt();
    let coeff = if mean > 0.0 { variance / mean } else { 0.0 };

    let personality_count = count_markers(&PERSONALITY_MARKERS, text);
    let personality_density = personality_count as f64 / n;
    let dynamic_range = clamp(coeff, 0.0, 1.0);
    let restriction_signal = if mean < 12.0 {
        clamp((12.0 - mean) / 8.0, 0.0, 1.0)
    } else {
        0.0
    };

    let openings: Vec<String> = sentences
        .iter()
        .map(|s| {
            WHITESPACE
                .split(s)
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for o in &openings {
        *counts.entry(o.clone()).or_insert(0) += 1;
    }
    let mut repeated_openings = 0usize;
    for c in counts.values() {
        if *c >= 2 {
            repeated_openings += c - 1;
        }
    }
    let repetition_signal = clamp(repeated_openings as f64 / n / 0.25, 0.0, 1.0);
    let engagement_signal = clamp(
        (text.matches('?').count() + text.matches('!').count()) as f64 / n / 0.2,
        0.0,
        1.0,
    );
    let passive_density = PASSIVE_RE.find_iter(text).count() as f64 / n;
    let filler_density = FILLER_RE.find_iter(text).count() as f64 / n;
    let impersonal_starts = sentences
        .iter()
        .filter(|s| IMPERSONAL_RE.is_match(s.trim()))
        .count();
    let impersonal_density = impersonal_starts as f64 / n;

    let unvoiced_signal = clamp(
        clamp(passive_density / 0.35, 0.0, 1.0) * 0.20
            + clamp(filler_density / 0.25, 0.0, 1.0) * 0.20
            + clamp(impersonal_density / 0.7, 0.0, 1.0) * 0.20
            + (1.0 - clamp(personality_density / 0.1, 0.0, 1.0)) * 0.20
            + (1.0 - engagement_signal) * 0.20,
        0.0,
        1.0,
    );
    let has_rhetorical =
        personality_density > 0.05 || engagement_signal > 0.1 || dynamic_range > 0.3;
    let effective_repetition = if has_rhetorical {
        repetition_signal
    } else {
        repetition_signal * 0.3
    };
    let positive_voice = clamp(
        dynamic_range * 0.30
            + clamp(personality_density / 0.2, 0.0, 1.0) * 0.25
            + restriction_signal * 0.15
            + effective_repetition * 0.15
            + engagement_signal * 0.15,
        0.0,
        1.0,
    );
    let voice_score = clamp(
        (1.0 - unvoiced_signal) * 0.60 + positive_voice * 0.40,
        0.0,
        1.0,
    );
    VoiceMetrics {
        voice_score,
        dynamic_range,
    }
}

/// Outputs of the register axis. F-score blend stubbed: `computeFScore` returns 50 with empty POS (L2).
pub struct RegisterMetrics {
    pub latinate_germanic_ratio: f64,
    pub register_markedness_score: f64,
}

pub fn analyze_register(text: &str) -> RegisterMetrics {
    let words = tokenize(text);
    let sentences = split_sentences(text);
    let word_count = words.len().max(1) as f64;
    let lgr = latinate_germanic_ratio(text);

    let mean = if !sentences.is_empty() {
        sentences
            .iter()
            .map(|s| WHITESPACE.split(s).count())
            .sum::<usize>() as f64
            / sentences.len() as f64
    } else {
        15.0
    };
    let contraction_rate = CONTRACTION_RE.find_iter(text).count() as f64 / word_count;
    let formal_count = words
        .iter()
        .filter(|w| FORMAL_MARKERS.contains(w.as_str()))
        .count();
    let formal_density = formal_count as f64 / word_count;
    let avg_word_len = words.iter().map(|w| char_len(w)).sum::<usize>() as f64 / word_count;
    let polysyllabic_ratio = words.iter().filter(|w| char_len(w) >= 8).count() as f64 / word_count;
    let semicolon_density = text.matches(';').count() as f64 / sentences.len().max(1) as f64;

    let high_signal = clamp(
        lgr * 0.25
            + clamp(polysyllabic_ratio / 0.12, 0.0, 1.0) * 0.20
            + formal_density * 10.0 * 0.15
            + clamp(mean / 30.0, 0.0, 1.0) * 0.20
            + clamp(semicolon_density / 0.15, 0.0, 1.0) * 0.10
            + clamp(avg_word_len / 6.5 - 0.3, 0.0, 1.0) * 0.10,
        0.0,
        1.0,
    );
    let low_signal = clamp(
        contraction_rate * 10.0 * 0.25
            + clamp(1.0 - mean / 12.0, 0.0, 1.0) * 0.25
            + clamp(1.0 - avg_word_len / 5.0, 0.0, 1.0) * 0.25
            + clamp((1.0 - lgr) * 1.5 - 0.5, 0.0, 1.0) * 0.25,
        0.0,
        1.0,
    );
    let register_markedness = clamp((high_signal - low_signal + 1.0) / 2.0, 0.0, 1.0);
    let f_score_normalized = 0.5; // computeFScore → 50 (empty POS) → /100
    let register_markedness_score = clamp(
        register_markedness * 0.70 + f_score_normalized * 0.30,
        0.0,
        1.0,
    );
    RegisterMetrics {
        latinate_germanic_ratio: lgr,
        register_markedness_score,
    }
}
