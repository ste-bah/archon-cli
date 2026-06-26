//! Lexical helpers and word lists (verbatim from `lanham-shared.ts`).
//!
//! Pure-lexical, POS-free predicates plus the register (Latinate/Germanic) axis.

use crate::{char_len, clamp, tokenize};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

// ── Word lists (verbatim from lanham-shared.ts) ──────────────────────────────
const BE_VERBS_ARR: &[&str] = &["is", "are", "was", "were", "been", "being", "am"];

const COMMON_VERBS_ARR: &[&str] = &[
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

const NOMINALIZATION_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ness", "ity", "ence", "ance", "ism", "ure",
];
const LATINATE_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ance", "ence", "ity", "ous", "ive", "able", "ible", "al", "ual",
];
const ROUGH_STEM_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ness", "ity", "ence", "ance", "ing", "ed", "ly", "er", "est", "ous",
    "ive", "al", "es", "s",
];

const NOM_EXCLUSIONS_ARR: &[&str] = &[
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

const STOP_ARR: &[&str] = &[
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
static COMMON_VERBS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| COMMON_VERBS_ARR.iter().copied().collect());
static NOM_EXCLUSIONS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| NOM_EXCLUSIONS_ARR.iter().copied().collect());
static STOP: Lazy<HashSet<&'static str>> = Lazy::new(|| STOP_ARR.iter().copied().collect());

static IS_VERB_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z]+(?:ed|ing|es)$").unwrap());

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
