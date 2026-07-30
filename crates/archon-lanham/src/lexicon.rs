//! Tokenisation, word lists and lexical predicates shared by every Lanham axis.
//!
//! Split out of `lib.rs` to keep every file under the 500-line gate.

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

/// Character length, matching JS `String.length` for the BMP (tokenized words
/// are ASCII post-tokenization, so this equals byte length there).
pub(crate) fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Clamp `v` into `[lo, hi]`. Mirrors TS `clamp(v, lo=0, hi=1)`.
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

// ── Tokenization ─────────────────────────────────────────────────────────────
// JS \w is ASCII ([A-Za-z0-9_]); spelled out so we keep ASCII-word semantics
// (Greek/non-ASCII letters are stripped, matching JS) while staying UTF-8-safe.
pub(crate) static NON_WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^A-Za-z0-9_\s'-]").unwrap());
pub(crate) static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
pub(crate) static SENT_BOUNDARY: Lazy<Regex> = Lazy::new(|| Regex::new(r"([.!?])\s+").unwrap());

/// Tokenize: lowercase, strip non-word punctuation, split on whitespace.
pub fn tokenize(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let cleaned = NON_WORD.replace_all(&lowered, " ");
    WHITESPACE
        .split(&cleaned)
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// Split into sentences on `.!?` boundaries, dropping fragments of <=2 words.
pub fn split_sentences(text: &str) -> Vec<String> {
    let piped = text.replace('|', "<<PIPE>>");
    let marked = SENT_BOUNDARY.replace_all(&piped, "$1|");
    marked
        .split('|')
        .map(|s| s.replace("<<PIPE>>", "|").trim().to_string())
        .filter(|s| !s.is_empty() && s.split_whitespace().count() > 2)
        .collect()
}

// ── Word lists (verbatim from lanham-shared.ts) ──────────────────────────────
pub(crate) const BE_VERBS_ARR: &[&str] = &["is", "are", "was", "were", "been", "being", "am"];

pub(crate) const COMMON_VERBS_ARR: &[&str] = &[
    "is",
    "are",
    "was",
    "were",
    "been",
    "being",
    "am",
    "have",
    "has",
    "had",
    "do",
    "does",
    "did",
    "say",
    "said",
    "says",
    "make",
    "made",
    "makes",
    "go",
    "goes",
    "went",
    "gone",
    "take",
    "took",
    "taken",
    "come",
    "came",
    "give",
    "gave",
    "given",
    "find",
    "found",
    "think",
    "thought",
    "know",
    "knew",
    "known",
    "get",
    "got",
    "gotten",
    "see",
    "saw",
    "seen",
    "want",
    "wanted",
    "use",
    "used",
    "tell",
    "told",
    "ask",
    "asked",
    "work",
    "worked",
    "seem",
    "seemed",
    "try",
    "tried",
    "leave",
    "left",
    "call",
    "called",
    "need",
    "needed",
    "become",
    "became",
    "keep",
    "kept",
    "let",
    "begin",
    "began",
    "begun",
    "show",
    "showed",
    "shown",
    "hear",
    "heard",
    "play",
    "played",
    "run",
    "ran",
    "move",
    "moved",
    "live",
    "lived",
    "believe",
    "believed",
    "bring",
    "brought",
    "happen",
    "happened",
    "write",
    "wrote",
    "written",
    "provide",
    "provided",
    "sit",
    "sat",
    "stand",
    "stood",
    "lose",
    "lost",
    "pay",
    "paid",
    "meet",
    "met",
    "include",
    "included",
    "continue",
    "continued",
    "set",
    "learn",
    "learned",
    "change",
    "changed",
    "lead",
    "led",
    "understand",
    "understood",
    "watch",
    "watched",
    "follow",
    "followed",
    "stop",
    "stopped",
    "create",
    "created",
    "speak",
    "spoke",
    "spoken",
    "read",
    "allow",
    "allowed",
    "add",
    "added",
    "grow",
    "grew",
    "grown",
    "open",
    "opened",
    "walk",
    "walked",
    "win",
    "won",
    "offer",
    "offered",
    "remember",
    "remembered",
    "consider",
    "considered",
    "appear",
    "appeared",
    "buy",
    "bought",
    "serve",
    "served",
    "die",
    "died",
    "send",
    "sent",
    "build",
    "built",
    "stay",
    "stayed",
    "fall",
    "fell",
    "fallen",
    "cut",
    "reach",
    "reached",
    "kill",
    "killed",
    "remain",
    "remained",
    "suggest",
    "suggested",
    "raise",
    "raised",
    "pass",
    "passed",
    "sell",
    "sold",
    "require",
    "required",
    "report",
    "reported",
    "decide",
    "decided",
    "pull",
    "pulled",
    "develop",
    "developed",
    "argues",
    "argue",
    "argued",
    "contends",
    "contend",
    "contended",
    "claims",
    "claim",
    "claimed",
    "asserts",
    "assert",
    "asserted",
    "maintains",
    "maintain",
    "maintained",
    "observes",
    "observe",
    "observed",
    "notes",
    "note",
    "noted",
    "suggests",
    "indicates",
    "indicate",
    "indicated",
    "demonstrates",
    "demonstrate",
    "demonstrated",
    "reveals",
    "reveal",
    "revealed",
    "establishes",
    "establish",
    "established",
    "examines",
    "examine",
    "examined",
    "explores",
    "explore",
    "explored",
    "analyzes",
    "analyze",
    "analyzed",
    "investigates",
    "investigate",
    "investigated",
];

pub(crate) const NOMINALIZATION_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ness", "ity", "ence", "ance", "ism", "ure",
];
pub(crate) const LATINATE_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ance", "ence", "ity", "ous", "ive", "able", "ible", "al", "ual",
];
pub(crate) const ROUGH_STEM_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ness", "ity", "ence", "ance", "ing", "ed", "ly", "er", "est", "ous",
    "ive", "al", "es", "s",
];

pub(crate) const NOM_EXCLUSIONS_ARR: &[&str] = &[
    "question",
    "fortune",
    "nature",
    "culture",
    "adventure",
    "furniture",
    "picture",
    "mixture",
    "creature",
    "structure",
    "feature",
    "future",
    "capture",
    "lecture",
    "gesture",
    "posture",
    "moisture",
    "nation",
    "station",
    "fashion",
    "passion",
    "version",
    "tension",
    "mention",
    "attention",
    "position",
    "condition",
    "tradition",
    "opinion",
    "religion",
    "region",
    "union",
    "lesson",
    "reason",
    "season",
    "person",
];

pub(crate) const STOP_ARR: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
    "did", "will", "would", "shall", "should", "may", "might", "can", "could", "not", "no", "nor",
    "so", "yet", "this", "that", "these", "those", "it", "its", "he", "she", "they", "we", "i",
    "you", "me", "him", "her", "us", "them", "my", "your", "his", "our", "their", "which", "who",
    "whom", "what", "if", "then", "than", "as", "up", "about", "into", "through", "after",
    "before",
];

pub(crate) static BE_VERBS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| BE_VERBS_ARR.iter().copied().collect());
pub(crate) static COMMON_VERBS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| COMMON_VERBS_ARR.iter().copied().collect());
pub(crate) static NOM_EXCLUSIONS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| NOM_EXCLUSIONS_ARR.iter().copied().collect());
pub(crate) static STOP: Lazy<HashSet<&'static str>> =
    Lazy::new(|| STOP_ARR.iter().copied().collect());

pub(crate) static IS_VERB_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z]+(?:ed|ing|es)$").unwrap());

// ── Lexical helpers ──────────────────────────────────────────────────────────
pub fn is_be_verb(word: &str) -> bool {
    BE_VERBS.contains(word.to_lowercase().as_str())
}

/// TS: COMMON_VERBS.has(w) || (/^[a-z]+(ed|ing|es)$/.test(w) && w.length > 4)
pub fn is_verb(word: &str) -> bool {
    let w = word.to_lowercase();
    if COMMON_VERBS.contains(w.as_str()) {
        return true;
    }
    char_len(&w) > 4 && IS_VERB_RE.is_match(&w)
}

/// TS: len>=6, not excluded, ends with a nominalization suffix.
pub fn is_nominalization(word: &str) -> bool {
    let w = word.to_lowercase();
    if char_len(&w) < 6 || NOM_EXCLUSIONS.contains(w.as_str()) {
        return false;
    }
    NOMINALIZATION_SUFFIXES.iter().any(|s| w.ends_with(s))
}

/// TS: len>=5, ends with a Latinate suffix.
pub fn is_latinate(word: &str) -> bool {
    let w = word.to_lowercase();
    char_len(&w) >= 5 && LATINATE_SUFFIXES.iter().any(|s| w.ends_with(s))
}

/// TS: strip the first matching suffix where word is long enough; first match wins.
pub fn rough_stem(word: &str) -> String {
    let mut w = word.to_lowercase();
    for s in ROUGH_STEM_SUFFIXES {
        if char_len(&w) > s.len() + 3 && w.ends_with(s) {
            w.truncate(w.len() - s.len());
            break;
        }
    }
    w
}

/// TS: drop stopwords (lowercased) and words of <=2 chars.
pub fn get_content_words(words: &[String]) -> Vec<String> {
    words
        .iter()
        .filter(|w| !STOP.contains(w.to_lowercase().as_str()) && char_len(w) > 2)
        .cloned()
        .collect()
}

// ── Axes (POS-free) ──────────────────────────────────────────────────────────
/// Latinate/Germanic ratio (register / diction). Pure-lexical, POS-free.
/// TS `analyzeRegister`: latinate / (latinate + short-germanic), else 0.5.
pub fn latinate_germanic_ratio(text: &str) -> f64 {
    let words = tokenize(text);
    let mut latinate = 0usize;
    let mut short_germanic = 0usize;
    for w in &words {
        if is_latinate(w) {
            latinate += 1;
        } else if char_len(w) <= 5 && char_len(w) >= 2 {
            short_germanic += 1;
        }
    }
    let denom = latinate + short_germanic;
    if denom > 0 {
        clamp(latinate as f64 / denom as f64, 0.0, 1.0)
    } else {
        0.5
    }
}

pub(crate) const PREPOSITIONS_ARR: &[&str] = &[
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
