//! Tacit rhetorical figure detection.
//!
//! Split out of `lib.rs` to keep every file under the 500-line gate.

use regex::Regex;

use crate::lexicon::*;

// ── Tacit patterns ───────────────────────────────────────────────────────────
pub(crate) const CONSONANTS: &str = "bcdfghjklmnpqrstvwxyz";
pub(crate) const ANTITHESIS_PAIRS: &[(&str, &str)] = &[
    ("not", "but"),
    ("rather", "than"),
    ("neither", "nor"),
    ("less", "more"),
    ("few", "many"),
    ("old", "new"),
];

#[derive(Clone)]
pub struct TacitPatterns {
    pub alliteration_density: f64,
    pub polyptoton_density: f64,
    pub chiasmus_count: i64,
    pub antithesis_count: i64,
    pub anaphora_count: i64,
    pub isocolon_count: i64,
    pub climax_pattern_count: i64,
}

pub(crate) fn first_n_lower(s: &str, n: usize) -> String {
    WHITESPACE
        .split(s)
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn detect_tacit(text: &str) -> TacitPatterns {
    let words = tokenize(text);
    let sentences = split_sentences(text);
    let content_words = get_content_words(&words);
    let sentence_count = sentences.len().max(1) as f64;

    let mut alliteration_count = 0i64;
    for i in 0..content_words.len().saturating_sub(2) {
        if let Some(a) = content_words[i].chars().next() {
            if content_words[i + 1].starts_with(a)
                && content_words[i + 2].starts_with(a)
                && CONSONANTS.contains(a)
            {
                alliteration_count += 1;
            }
        }
    }
    let alliteration_density = alliteration_count as f64 / sentence_count;

    let stems: Vec<String> = content_words.iter().map(|w| rough_stem(w)).collect();
    let mut polyptoton_count = 0i64;
    for i in 0..stems.len().saturating_sub(1) {
        let end = (i + 8).min(stems.len());
        for j in (i + 1)..end {
            if stems[i] == stems[j]
                && content_words[i] != content_words[j]
                && char_len(&stems[i]) > 3
            {
                polyptoton_count += 1;
                break;
            }
        }
    }
    let polyptoton_density = polyptoton_count as f64 / sentence_count;

    let mut chiasmus_count = 0i64;
    for i in 0..sentences.len().saturating_sub(1) {
        let s1: Vec<String> = tokenize(&sentences[i])
            .iter()
            .map(|w| rough_stem(w))
            .filter(|s| char_len(s) > 3)
            .collect();
        let s2: Vec<String> = tokenize(&sentences[i + 1])
            .iter()
            .map(|w| rough_stem(w))
            .filter(|s| char_len(s) > 3)
            .collect();
        for a in 0..s1.len().saturating_sub(1) {
            let end = (a + 5).min(s1.len());
            for b in (a + 1)..end {
                if s1[a] == s1[b] {
                    continue;
                }
                if let Some(idx_b) = s2.iter().position(|x| *x == s1[b]) {
                    if s2[idx_b + 1..].iter().any(|x| *x == s1[a]) {
                        chiasmus_count += 1;
                        break;
                    }
                }
            }
            if chiasmus_count > 0 && chiasmus_count > i as i64 {
                break;
            }
        }
    }

    let text_lower = text.to_lowercase();
    let mut antithesis_count = 0i64;
    for (a, b) in ANTITHESIS_PAIRS {
        let re = Regex::new(&format!(r"\b{}\b[^.]{{1,40}}\b{}\b", a, b)).unwrap();
        antithesis_count += re.find_iter(&text_lower).count() as i64;
    }

    let mut anaphora_count = 0i64;
    for i in 0..sentences.len().saturating_sub(2) {
        let o1 = first_n_lower(&sentences[i], 3);
        let o2 = first_n_lower(&sentences[i + 1], 3);
        let o3 = first_n_lower(&sentences[i + 2], 3);
        if o1 == o2 && o2 == o3 && char_len(&o1) > 4 {
            anaphora_count += 1;
        }
    }

    let slen = |s: &str| WHITESPACE.split(s).count() as f64;
    let mut isocolon_count = 0i64;
    for i in 0..sentences.len().saturating_sub(1) {
        let (l1, l2) = (slen(&sentences[i]), slen(&sentences[i + 1]));
        let max_len = l1.max(l2);
        if max_len > 0.0 && (l1 - l2).abs() / max_len <= 0.20 && i + 2 < sentences.len() {
            let l3 = slen(&sentences[i + 2]);
            if (l2 - l3).abs() / l2.max(l3) <= 0.20 {
                isocolon_count += 1;
            }
        }
    }

    let mut climax_pattern_count = 0i64;
    for i in 0..sentences.len().saturating_sub(2) {
        let (l1, l2, l3) = (
            WHITESPACE.split(&sentences[i]).count() as i64,
            WHITESPACE.split(&sentences[i + 1]).count() as i64,
            WHITESPACE.split(&sentences[i + 2]).count() as i64,
        );
        if l1 < l2 && l2 < l3 && (l3 - l1) >= 5 {
            climax_pattern_count += 1;
        }
    }

    TacitPatterns {
        alliteration_density,
        polyptoton_density,
        chiasmus_count,
        antithesis_count,
        anaphora_count,
        isocolon_count,
        climax_pattern_count,
    }
}
