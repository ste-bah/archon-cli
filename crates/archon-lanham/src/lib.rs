//! Lanham prose analyzer — Rust port (POS-free axes, L2).
//!
//! Faithful port of the god-agent TypeScript analyzer (`lanham-shared.ts`).
//! Every public function is golden-tested against the TS reference (see `tests/`).
//! The POS tagger is behind the `tag_pos` seam (returns empty in L2; en-pos
//! reimplementation is L3).

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

pub mod render;

/// Character length, matching JS `String.length` for the BMP (tokenized words
/// are ASCII post-tokenization, so this equals byte length there).
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Clamp `v` into `[lo, hi]`. Mirrors TS `clamp(v, lo=0, hi=1)`.
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

// ── Tokenization ─────────────────────────────────────────────────────────────
// JS \w is ASCII ([A-Za-z0-9_]); spelled out so we keep ASCII-word semantics
// (Greek/non-ASCII letters are stripped, matching JS) while staying UTF-8-safe.
static NON_WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^A-Za-z0-9_\s'-]").unwrap());
static WHITESPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
static SENT_BOUNDARY: Lazy<Regex> = Lazy::new(|| Regex::new(r"([.!?])\s+").unwrap());

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
const BE_VERBS_ARR: &[&str] = &["is", "are", "was", "were", "been", "being", "am"];

const COMMON_VERBS_ARR: &[&str] = &[
    "is", "are", "was", "were", "been", "being", "am", "have", "has", "had", "do", "does", "did",
    "say", "said", "says", "make", "made", "makes", "go", "goes", "went", "gone", "take", "took",
    "taken", "come", "came", "give", "gave", "given", "find", "found", "think", "thought", "know",
    "knew", "known", "get", "got", "gotten", "see", "saw", "seen", "want", "wanted", "use", "used",
    "tell", "told", "ask", "asked", "work", "worked", "seem", "seemed", "try", "tried", "leave",
    "left", "call", "called", "need", "needed", "become", "became", "keep", "kept", "let", "begin",
    "began", "begun", "show", "showed", "shown", "hear", "heard", "play", "played", "run", "ran",
    "move", "moved", "live", "lived", "believe", "believed", "bring", "brought", "happen",
    "happened", "write", "wrote", "written", "provide", "provided", "sit", "sat", "stand", "stood",
    "lose", "lost", "pay", "paid", "meet", "met", "include", "included", "continue", "continued",
    "set", "learn", "learned", "change", "changed", "lead", "led", "understand", "understood",
    "watch", "watched", "follow", "followed", "stop", "stopped", "create", "created", "speak",
    "spoke", "spoken", "read", "allow", "allowed", "add", "added", "grow", "grew", "grown", "open",
    "opened", "walk", "walked", "win", "won", "offer", "offered", "remember", "remembered",
    "consider", "considered", "appear", "appeared", "buy", "bought", "serve", "served", "die",
    "died", "send", "sent", "build", "built", "stay", "stayed", "fall", "fell", "fallen", "cut",
    "reach", "reached", "kill", "killed", "remain", "remained", "suggest", "suggested", "raise",
    "raised", "pass", "passed", "sell", "sold", "require", "required", "report", "reported",
    "decide", "decided", "pull", "pulled", "develop", "developed", "argues", "argue", "argued",
    "contends", "contend", "contended", "claims", "claim", "claimed", "asserts", "assert",
    "asserted", "maintains", "maintain", "maintained", "observes", "observe", "observed", "notes",
    "note", "noted", "suggests", "indicates", "indicate", "indicated", "demonstrates",
    "demonstrate", "demonstrated", "reveals", "reveal", "revealed", "establishes", "establish",
    "established", "examines", "examine", "examined", "explores", "explore", "explored", "analyzes",
    "analyze", "analyzed", "investigates", "investigate", "investigated",
];

const NOMINALIZATION_SUFFIXES: &[&str] =
    &["tion", "sion", "ment", "ness", "ity", "ence", "ance", "ism", "ure"];
const LATINATE_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ance", "ence", "ity", "ous", "ive", "able", "ible", "al", "ual",
];
const ROUGH_STEM_SUFFIXES: &[&str] = &[
    "tion", "sion", "ment", "ness", "ity", "ence", "ance", "ing", "ed", "ly", "er", "est", "ous",
    "ive", "al", "es", "s",
];

const NOM_EXCLUSIONS_ARR: &[&str] = &[
    "question", "fortune", "nature", "culture", "adventure", "furniture", "picture", "mixture",
    "creature", "structure", "feature", "future", "capture", "lecture", "gesture", "posture",
    "moisture", "nation", "station", "fashion", "passion", "version", "tension", "mention",
    "attention", "position", "condition", "tradition", "opinion", "religion", "region", "union",
    "lesson", "reason", "season", "person",
];

const STOP_ARR: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by", "from",
    "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does", "did",
    "will", "would", "shall", "should", "may", "might", "can", "could", "not", "no", "nor", "so",
    "yet", "this", "that", "these", "those", "it", "its", "he", "she", "they", "we", "i", "you",
    "me", "him", "her", "us", "them", "my", "your", "his", "our", "their", "which", "who", "whom",
    "what", "if", "then", "than", "as", "up", "about", "into", "through", "after", "before",
];

static BE_VERBS: Lazy<HashSet<&'static str>> = Lazy::new(|| BE_VERBS_ARR.iter().copied().collect());
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

const PREPOSITIONS_ARR: &[&str] = &[
    "of", "in", "to", "for", "with", "on", "at", "from", "by", "about", "as", "into", "through",
    "during", "before", "after", "above", "below", "between", "under", "along", "until", "without",
    "toward", "towards", "upon", "across", "against", "among", "behind", "beyond", "within",
    "throughout", "beside", "besides", "despite", "concerning", "regarding", "per", "via",
];
static PREPOSITIONS: Lazy<HashSet<&'static str>> =
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

const COORDINATING_CONJ_ARR: &[&str] = &["and", "but", "or", "nor", "for", "yet", "so"];
const SUBORDINATING_CONJ_ARR: &[&str] = &[
    "although", "because", "since", "unless", "while", "whereas", "when", "where", "if", "though",
    "after", "before", "until", "once", "whenever", "wherever", "whether", "provided", "supposing",
    "inasmuch", "insofar", "notwithstanding", "albeit", "lest",
];
const RELATIVE_WORDS_ARR: &[&str] = &["which", "who", "whom", "whose", "where", "whereby"];

static COORDINATING_CONJ: Lazy<HashSet<&'static str>> =
    Lazy::new(|| COORDINATING_CONJ_ARR.iter().copied().collect());
static SUBORDINATING_CONJ: Lazy<HashSet<&'static str>> =
    Lazy::new(|| SUBORDINATING_CONJ_ARR.iter().copied().collect());
static RELATIVE_WORDS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| RELATIVE_WORDS_ARR.iter().copied().collect());
// Case-sensitive: matches capitalized sentence-initial conjunctions.
static SENT_INIT_CONJ: Lazy<Regex> =
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

// ── Data + regexes for the remaining axes ────────────────────────────────────
const FORMAL_MARKERS_ARR: &[&str] = &[
    "furthermore", "moreover", "nevertheless", "notwithstanding", "consequently", "subsequently",
    "henceforth", "whereby", "wherein", "therein", "thereof", "herein", "aforementioned",
    "heretofore", "thus", "hence", "accordingly", "indeed", "nonetheless",
];
static FORMAL_MARKERS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| FORMAL_MARKERS_ARR.iter().copied().collect());

static CONTRACTION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[A-Za-z0-9_]+'[A-Za-z0-9_]+\b").unwrap());
static PASSIVE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:is|are|was|were|been|be)\s+[A-Za-z0-9_]+ed\b").unwrap());
static FILLER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(?:of course|as you know|it is|there is|there are|it was|it has been|in terms of|with respect to|in connection with|pursuant to|shall be|may be|provided that|in accordance)\b").unwrap());
static IMPERSONAL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?:The |It |This |These |That |Those |Such |An? )").unwrap());
static AND_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\band\b").unwrap());
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

fn count_markers(markers: &[Regex], text: &str) -> usize {
    markers.iter().map(|re| re.find_iter(text).count()).sum()
}

/// Outputs of the periodic/running axis.
pub struct PeriodicMetrics {
    pub periodic_running_ratio: f64,
    pub pre_main_verb_clause_count: f64,
}

pub fn analyze_periodic(text: &str) -> PeriodicMetrics {
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return PeriodicMetrics { periodic_running_ratio: 0.5, pre_main_verb_clause_count: 0.0 };
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
            .map(|w| w.to_lowercase().chars().filter(|c| c.is_ascii_lowercase()).collect())
            .collect();
        let starts_with_subord = first_words.iter().any(|w| SUBORDINATING_CONJ.contains(w.as_str()));
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
    let periodic_running_ratio = if total > 0.0 { clamp(running_signals / total, 0.0, 1.0) } else { 0.5 };
    let pre_main_verb_clause_count = total_pre_main as f64 / sentences.len() as f64;
    PeriodicMetrics { periodic_running_ratio, pre_main_verb_clause_count }
}

/// Outputs of the voice axis (entropy/contrast/terminal D1-D3 are computed-but-unused in TS, so omitted).
pub struct VoiceMetrics {
    pub voice_score: f64,
    pub dynamic_range: f64,
}

pub fn analyze_voice(text: &str) -> VoiceMetrics {
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return VoiceMetrics { voice_score: 0.5, dynamic_range: 0.0 };
    }
    let n = sentences.len() as f64;
    let lengths: Vec<f64> = sentences.iter().map(|s| WHITESPACE.split(s).count() as f64).collect();
    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
    let variance = (lengths.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lengths.len() as f64).sqrt();
    let coeff = if mean > 0.0 { variance / mean } else { 0.0 };

    let personality_count = count_markers(&PERSONALITY_MARKERS, text);
    let personality_density = personality_count as f64 / n;
    let dynamic_range = clamp(coeff, 0.0, 1.0);
    let restriction_signal = if mean < 12.0 { clamp((12.0 - mean) / 8.0, 0.0, 1.0) } else { 0.0 };

    let openings: Vec<String> = sentences
        .iter()
        .map(|s| WHITESPACE.split(s).take(2).collect::<Vec<_>>().join(" ").to_lowercase())
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
    let impersonal_starts = sentences.iter().filter(|s| IMPERSONAL_RE.is_match(s.trim())).count();
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
    let has_rhetorical = personality_density > 0.05 || engagement_signal > 0.1 || dynamic_range > 0.3;
    let effective_repetition = if has_rhetorical { repetition_signal } else { repetition_signal * 0.3 };
    let positive_voice = clamp(
        dynamic_range * 0.30
            + clamp(personality_density / 0.2, 0.0, 1.0) * 0.25
            + restriction_signal * 0.15
            + effective_repetition * 0.15
            + engagement_signal * 0.15,
        0.0,
        1.0,
    );
    let voice_score = clamp((1.0 - unvoiced_signal) * 0.60 + positive_voice * 0.40, 0.0, 1.0);
    VoiceMetrics { voice_score, dynamic_range }
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
        sentences.iter().map(|s| WHITESPACE.split(s).count()).sum::<usize>() as f64 / sentences.len() as f64
    } else {
        15.0
    };
    let contraction_rate = CONTRACTION_RE.find_iter(text).count() as f64 / word_count;
    let formal_count = words.iter().filter(|w| FORMAL_MARKERS.contains(w.as_str())).count();
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
    let register_markedness_score = clamp(register_markedness * 0.70 + f_score_normalized * 0.30, 0.0, 1.0);
    RegisterMetrics { latinate_germanic_ratio: lgr, register_markedness_score }
}

// ── Opacity ──────────────────────────────────────────────────────────────────
const FUNCTION_WORDS_ARR: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
    "did", "will", "would", "shall", "should", "may", "might", "can", "could", "not", "no", "this",
    "that", "these", "those", "it", "its",
];
static FUNCTION_WORDS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| FUNCTION_WORDS_ARR.iter().copied().collect());

const TN_CENTROID: [f64; 4] = [22.5, 4.8, 0.045, 0.48];
const TN_GOLD_MEAN: [f64; 4] = [24.0, 4.7, 0.055, 0.46];
const TN_GOLD_STD: [f64; 4] = [10.0, 0.6, 0.025, 0.05];

static PUNCT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[,;:!?\-—–]").unwrap());
static SENT_TERM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[.!?]+").unwrap());

static META_LINGUISTIC_MARKERS: Lazy<Vec<Regex>> = Lazy::new(|| {
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
static OPACITY_CONTENT_MARKERS: Lazy<Vec<Regex>> = Lazy::new(|| {
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

fn opacity_feature_vector(text: &str, sentences: &[String], words: &[String]) -> [f64; 4] {
    let word_count = words.len().max(1) as f64;
    let avg_sent_len = if !sentences.is_empty() {
        sentences.iter().map(|s| WHITESPACE.split(s).count()).sum::<usize>() as f64 / sentences.len() as f64
    } else {
        15.0
    };
    let avg_word_len = words.iter().map(|w| char_len(w)).sum::<usize>() as f64 / word_count;
    let punct_density = PUNCT_RE.find_iter(text).count() as f64 / word_count;
    let func_count = words.iter().filter(|w| FUNCTION_WORDS.contains(w.as_str())).count();
    [avg_sent_len, avg_word_len, punct_density, func_count as f64 / word_count]
}

fn opacity_deviation_from_norm(text: &str, sentences: &[String], words: &[String]) -> f64 {
    let f = opacity_feature_vector(text, sentences, words);
    let mut dist_sq = 0.0f64;
    for i in 0..4 {
        let std = if TN_GOLD_STD[i] != 0.0 { TN_GOLD_STD[i] } else { 1.0 };
        let z_feature = (f[i] - TN_GOLD_MEAN[i]) / std;
        let z_centroid = (TN_CENTROID[i] - TN_GOLD_MEAN[i]) / std;
        dist_sq += (z_feature - z_centroid).powi(2);
    }
    (dist_sq.sqrt() / 4.0).min(1.0)
}

/// Base opacity (before the fullAnalysis tacit blend).
struct OpacityBase {
    opacity_score: f64,
    self_consciousness_score: f64,
}

fn analyze_opacity(text: &str) -> OpacityBase {
    let sentences = split_sentences(text);
    let sentence_count = sentences.len().max(1) as f64;
    let meta_ling_density = clamp(count_markers(&META_LINGUISTIC_MARKERS, text) as f64 / sentence_count / 0.12, 0.0, 1.0);
    let content_opacity_density = clamp(count_markers(&OPACITY_CONTENT_MARKERS, text) as f64 / sentence_count / 0.25, 0.0, 1.0);
    let words = tokenize(text);
    let content_words = get_content_words(&words);

    let mut alliteration_hits = 0usize;
    for i in 0..content_words.len().saturating_sub(2) {
        let a = content_words[i].chars().next();
        if a.is_some() && a == content_words[i + 1].chars().next() && a == content_words[i + 2].chars().next() {
            alliteration_hits += 1;
        }
    }
    let sound_density = clamp(alliteration_hits as f64 / sentence_count / 0.3, 0.0, 1.0);
    let self_consciousness_score = meta_ling_density;
    let polysyndeton_density = clamp(AND_RE.find_iter(text).count() as f64 / words.len().max(1) as f64 / 0.06, 0.0, 1.0);

    let mut word_freqs: HashMap<&str, usize> = HashMap::new();
    for w in &content_words {
        *word_freqs.entry(w.as_str()).or_insert(0) += 1;
    }
    let repeated = word_freqs.values().filter(|&&c| c >= 3).count();
    let repetition_density = clamp(repeated as f64 / content_words.len().max(1) as f64 / 0.04, 0.0, 1.0);

    let sent_lens: Vec<usize> = sentences.iter().map(|s| WHITESPACE.split(s).count()).collect();
    let very_short = sent_lens.iter().filter(|&&l| l <= 5).count();
    let very_long = sent_lens.iter().filter(|&&l| l >= 40).count();
    let extremes_density = clamp((very_short + very_long) as f64 / sentence_count / 0.25, 0.0, 1.0);

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
    OpacityBase { opacity_score, self_consciousness_score }
}

// ── Tacit patterns ───────────────────────────────────────────────────────────
const CONSONANTS: &str = "bcdfghjklmnpqrstvwxyz";
const ANTITHESIS_PAIRS: &[(&str, &str)] = &[
    ("not", "but"), ("rather", "than"), ("neither", "nor"), ("less", "more"), ("few", "many"),
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

fn first_n_lower(s: &str, n: usize) -> String {
    WHITESPACE.split(s).take(n).collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn detect_tacit(text: &str) -> TacitPatterns {
    let words = tokenize(text);
    let sentences = split_sentences(text);
    let content_words = get_content_words(&words);
    let sentence_count = sentences.len().max(1) as f64;

    let mut alliteration_count = 0i64;
    for i in 0..content_words.len().saturating_sub(2) {
        if let Some(a) = content_words[i].chars().next() {
            if Some(a) == content_words[i + 1].chars().next()
                && Some(a) == content_words[i + 2].chars().next()
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
            if stems[i] == stems[j] && content_words[i] != content_words[j] && char_len(&stems[i]) > 3 {
                polyptoton_count += 1;
                break;
            }
        }
    }
    let polyptoton_density = polyptoton_count as f64 / sentence_count;

    let mut chiasmus_count = 0i64;
    for i in 0..sentences.len().saturating_sub(1) {
        let s1: Vec<String> = tokenize(&sentences[i]).iter().map(|w| rough_stem(w)).filter(|s| char_len(s) > 3).collect();
        let s2: Vec<String> = tokenize(&sentences[i + 1]).iter().map(|w| rough_stem(w)).filter(|s| char_len(s) > 3).collect();
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

// ── Labels + full analysis ───────────────────────────────────────────────────
#[derive(Clone, Default)]
pub struct Labels {
    pub noun_verb: String,
    pub parataxis_hypotaxis: String,
    pub periodic_running: String,
    pub voice: String,
    pub primary_register: String,
    pub register_mixed: bool,
    pub opacity: String,
}

#[derive(Clone)]
pub struct LanhamMetrics {
    pub noun_verb_ratio: f64,
    pub nominalization_density: f64,
    pub prepositional_phrase_density: f64,
    pub be_verb_ratio: f64,
    pub parataxis_hypotaxis_ratio: f64,
    pub coordinating_conjunction_density: f64,
    pub subordinating_conjunction_density: f64,
    pub periodic_running_ratio: f64,
    pub pre_main_verb_clause_count: f64,
    pub voice_score: f64,
    pub dynamic_range: f64,
    pub latinate_germanic_ratio: f64,
    pub register_markedness_score: f64,
    pub opacity_score: f64,
    pub self_consciousness_score: f64,
    pub tacit_patterns: TacitPatterns,
    pub labels: Labels,
}

// Academic genre thresholds (GENRE_THRESHOLDS["academic"]).
fn derive_labels(m: &LanhamMetrics) -> Labels {
    let band = |v: f64, lo: f64, hi: f64, a: &str, b: &str, c: &str| {
        (if v < lo { a } else if v > hi { b } else { c }).to_string()
    };
    let primary_register = if m.register_markedness_score >= 0.62 {
        "high"
    } else if m.register_markedness_score <= 0.38 {
        "low"
    } else {
        "middle"
    };
    let mut noun_verb = band(m.noun_verb_ratio, 0.35, 0.65, "predominantly noun-style", "predominantly verb-style", "balanced");
    if noun_verb == "balanced"
        && m.nominalization_density > 8.0
        && m.be_verb_ratio > 0.25
        && m.prepositional_phrase_density > 3.0
    {
        noun_verb = "predominantly noun-style".to_string();
    }
    Labels {
        noun_verb,
        parataxis_hypotaxis: band(m.parataxis_hypotaxis_ratio, 0.35, 0.65, "predominantly paratactic", "predominantly hypotactic", "mixed"),
        periodic_running: band(m.periodic_running_ratio, 0.35, 0.65, "predominantly periodic", "predominantly running", "mixed"),
        voice: band(m.voice_score, 0.30, 0.70, "unvoiced", "strongly voiced", "moderate voice"),
        primary_register: primary_register.to_string(),
        register_mixed: false,
        opacity: band(m.opacity_score, 0.25, 0.60, "transparent", "opaque", "mixed opacity"),
    }
}

/// Full analysis: all axes + the opacity-tacit blend + academic labels.
pub fn full_analysis(text: &str) -> LanhamMetrics {
    let nv = analyze_noun_verb(text);
    let pp = analyze_parataxis(text);
    let per = analyze_periodic(text);
    let vo = analyze_voice(text);
    let rg = analyze_register(text);
    let op = analyze_opacity(text);
    let tacit = detect_tacit(text);

    // Blend tacit pattern density into opacity (fullAnalysis).
    let sent_count = SENT_TERM_RE.find_iter(text).count().max(1) as f64;
    let tacit_total = (tacit.anaphora_count + tacit.chiasmus_count + tacit.antithesis_count
        + tacit.isocolon_count + tacit.climax_pattern_count) as f64
        / sent_count;
    let tacit_density = clamp(tacit_total / 0.2, 0.0, 1.0);
    let allit = clamp(tacit.alliteration_density / 0.15, 0.0, 1.0);
    let polyp = clamp(tacit.polyptoton_density / 0.1, 0.0, 1.0);
    let opacity_score = clamp(op.opacity_score * 0.50 + tacit_density * 0.25 + allit * 0.15 + polyp * 0.10, 0.0, 1.0);

    let mut m = LanhamMetrics {
        noun_verb_ratio: nv.noun_verb_ratio,
        nominalization_density: nv.nominalization_density,
        prepositional_phrase_density: nv.prepositional_phrase_density,
        be_verb_ratio: nv.be_verb_ratio,
        parataxis_hypotaxis_ratio: pp.parataxis_hypotaxis_ratio,
        coordinating_conjunction_density: pp.coordinating_conjunction_density,
        subordinating_conjunction_density: pp.subordinating_conjunction_density,
        periodic_running_ratio: per.periodic_running_ratio,
        pre_main_verb_clause_count: per.pre_main_verb_clause_count,
        voice_score: vo.voice_score,
        dynamic_range: vo.dynamic_range,
        latinate_germanic_ratio: rg.latinate_germanic_ratio,
        register_markedness_score: rg.register_markedness_score,
        opacity_score,
        self_consciousness_score: op.self_consciousness_score,
        tacit_patterns: tacit,
        labels: Labels::default(),
    };
    m.labels = derive_labels(&m);
    m
}

/// Result of training: the rendered output-style `.md` plus a few labels for a summary line.
pub struct TrainResult {
    pub md: String,
    pub voice: String,
    pub register: String,
    pub parataxis: String,
}

/// Train an output-style from raw sample prose: measure Lanham style + base sentence/tone
/// stats, assemble a profile, render to the Archon output-style `.md`. All-Rust, offline.
pub fn train_to_output_style(text: &str, name: &str, genre: &str) -> TrainResult {
    use crate::render::{
        ArgumentPatterns, Characteristics, ClaimStructure, Explanations, LabelsJson,
        LanhamMetricsJson, Metadata, Profile, Sentences, SuggestedTarget, Tone,
    };

    let m = full_analysis(text);

    // Base sentence stats.
    let sents = split_sentences(text);
    let lens: Vec<usize> = sents.iter().map(|s| s.split_whitespace().count()).collect();
    let n = sents.len().max(1) as f64;
    let avg_len = if lens.is_empty() { 0.0 } else { lens.iter().sum::<usize>() as f64 / lens.len() as f64 };
    let long_ratio = lens.iter().filter(|&&l| l > 25).count() as f64 / n;
    let complex_ratio = sents
        .iter()
        .filter(|s| s.contains(';') || s.contains(':') || s.matches(',').count() >= 3)
        .count() as f64
        / n;

    // Base tone stats (formality proxy: Latinate ratio stands in for the academic word-list).
    let words = tokenize(text);
    let wn = words.len().max(1) as f64;
    let first_person = ["i", "we", "my", "our", "me", "us"];
    let fp = words.iter().filter(|w| first_person.contains(&w.as_str())).count() as f64;
    let objectivity = (1.0 - fp / wn * 20.0).clamp(0.0, 1.0);
    let contractions = words.iter().filter(|w| w.contains('\'')).count() as f64;
    let formality = (0.3 + (1.0 - contractions / wn * 10.0) * 0.3 + latinate_germanic_ratio(text) * 0.4).clamp(0.0, 1.0);

    let profile = Profile {
        metadata: Metadata {
            name: Some(name.to_string()),
            description: Some(format!("Trained style profile {name}")),
            suggested_lanham_target: SuggestedTarget {
                derived_from: Some(genre.to_string()),
                register_target: None, // converter falls back to the MEASURED primaryRegister
                tacit_persuasion_level: Some("moderate".into()),
            },
        },
        characteristics: Characteristics {
            lanham_metrics: LanhamMetricsJson {
                labels: LabelsJson {
                    noun_verb: Some(m.labels.noun_verb.clone()),
                    parataxis_hypotaxis: Some(m.labels.parataxis_hypotaxis.clone()),
                    periodic_running: Some(m.labels.periodic_running.clone()),
                    voice: Some(m.labels.voice.clone()),
                    primary_register: Some(m.labels.primary_register.clone()),
                    opacity: Some(m.labels.opacity.clone()),
                },
                explanations: Explanations::default(),
            },
            sentences: Sentences {
                average_length: avg_len,
                long_sentence_ratio: long_ratio,
                complex_sentence_ratio: complex_ratio,
            },
            tone: Tone { formality_score: formality, objectivity_score: objectivity },
            argument_patterns: ArgumentPatterns { claim_structure: ClaimStructure::default() },
            common_transitions: Vec::new(),
        },
    };

    TrainResult {
        md: crate::render::render_output_style(&profile, name),
        voice: m.labels.voice,
        register: m.labels.primary_register,
        parataxis: m.labels.parataxis_hypotaxis,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn golden(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/{}", env!("CARGO_MANIFEST_DIR"), name))
            .unwrap_or_else(|_| panic!("{name} missing — run scripts/lanham-golden-gen.ts"))
    }

    #[derive(Deserialize)]
    struct Foundation {
        text: String,
        tokens: Vec<String>,
        sentences: Vec<String>,
    }

    #[test]
    fn foundation_matches_ts_reference() {
        let fixtures: Vec<Foundation> = serde_json::from_str(&golden("golden_foundation.json")).unwrap();
        assert!(!fixtures.is_empty());
        for fx in &fixtures {
            assert_eq!(tokenize(&fx.text), fx.tokens, "tokenize:\n  {}", fx.text);
            assert_eq!(split_sentences(&fx.text), fx.sentences, "split_sentences:\n  {}", fx.text);
        }
    }

    #[derive(Deserialize)]
    struct Lexical {
        words: Vec<String>,
        #[serde(rename = "isVerb")]
        is_verb: Vec<bool>,
        #[serde(rename = "isNominalization")]
        is_nominalization: Vec<bool>,
        #[serde(rename = "isLatinate")]
        is_latinate: Vec<bool>,
        #[serde(rename = "roughStem")]
        rough_stem: Vec<String>,
        #[serde(rename = "contentWords")]
        content_words: Vec<String>,
    }

    #[test]
    fn lexical_matches_ts_reference() {
        let lx: Lexical = serde_json::from_str(&golden("golden_lexical.json")).unwrap();
        for (i, w) in lx.words.iter().enumerate() {
            assert_eq!(is_verb(w), lx.is_verb[i], "is_verb({w})");
            assert_eq!(is_nominalization(w), lx.is_nominalization[i], "is_nominalization({w})");
            assert_eq!(is_latinate(w), lx.is_latinate[i], "is_latinate({w})");
            assert_eq!(rough_stem(w), lx.rough_stem[i], "rough_stem({w})");
        }
        assert_eq!(get_content_words(&lx.words), lx.content_words, "get_content_words");
    }

    #[derive(Deserialize)]
    struct AxisFixture {
        text: String,
        #[serde(rename = "latinateGermanicRatio")]
        latinate_germanic_ratio: f64,
        #[serde(rename = "nounVerbRatio")]
        noun_verb_ratio: f64,
        #[serde(rename = "nominalizationDensity")]
        nominalization_density: f64,
        #[serde(rename = "prepositionalPhraseDensity")]
        prepositional_phrase_density: f64,
        #[serde(rename = "beVerbRatio")]
        be_verb_ratio: f64,
        #[serde(rename = "parataxisHypotaxisRatio")]
        parataxis_hypotaxis_ratio: f64,
        #[serde(rename = "coordinatingConjunctionDensity")]
        coordinating_conjunction_density: f64,
        #[serde(rename = "subordinatingConjunctionDensity")]
        subordinating_conjunction_density: f64,
        #[serde(rename = "periodicRunningRatio")]
        periodic_running_ratio: f64,
        #[serde(rename = "preMainVerbClauseCount")]
        pre_main_verb_clause_count: f64,
        #[serde(rename = "voiceScore")]
        voice_score: f64,
        #[serde(rename = "dynamicRange")]
        dynamic_range: f64,
        #[serde(rename = "registerMarkednessScore")]
        register_markedness_score: f64,
    }

    #[test]
    fn axes_match_ts_reference() {
        let fixtures: Vec<AxisFixture> = serde_json::from_str(&golden("golden_axes.json")).unwrap();
        assert!(!fixtures.is_empty());
        let approx = |a: f64, b: f64, name: &str, t: &str| {
            assert!((a - b).abs() < 1e-9, "{name}: got {a}, want {b} for:\n  {t}");
        };
        for fx in &fixtures {
            approx(latinate_germanic_ratio(&fx.text), fx.latinate_germanic_ratio, "latinate_germanic_ratio", &fx.text);
            let nv = analyze_noun_verb(&fx.text);
            approx(nv.noun_verb_ratio, fx.noun_verb_ratio, "noun_verb_ratio", &fx.text);
            approx(nv.nominalization_density, fx.nominalization_density, "nominalization_density", &fx.text);
            approx(nv.prepositional_phrase_density, fx.prepositional_phrase_density, "prepositional_phrase_density", &fx.text);
            approx(nv.be_verb_ratio, fx.be_verb_ratio, "be_verb_ratio", &fx.text);
            let pp = analyze_parataxis(&fx.text);
            approx(pp.parataxis_hypotaxis_ratio, fx.parataxis_hypotaxis_ratio, "parataxis_hypotaxis_ratio", &fx.text);
            approx(pp.coordinating_conjunction_density, fx.coordinating_conjunction_density, "coordinating_conjunction_density", &fx.text);
            approx(pp.subordinating_conjunction_density, fx.subordinating_conjunction_density, "subordinating_conjunction_density", &fx.text);
            let per = analyze_periodic(&fx.text);
            approx(per.periodic_running_ratio, fx.periodic_running_ratio, "periodic_running_ratio", &fx.text);
            approx(per.pre_main_verb_clause_count, fx.pre_main_verb_clause_count, "pre_main_verb_clause_count", &fx.text);
            let vo = analyze_voice(&fx.text);
            approx(vo.voice_score, fx.voice_score, "voice_score", &fx.text);
            approx(vo.dynamic_range, fx.dynamic_range, "dynamic_range", &fx.text);
            approx(analyze_register(&fx.text).register_markedness_score, fx.register_markedness_score, "register_markedness_score", &fx.text);
        }
    }

    #[derive(Deserialize)]
    struct TacitFx {
        #[serde(rename = "alliterationDensity")] alliteration_density: f64,
        #[serde(rename = "polyptotonDensity")] polyptoton_density: f64,
        #[serde(rename = "chiasmusCount")] chiasmus_count: i64,
        #[serde(rename = "antithesisCount")] antithesis_count: i64,
        #[serde(rename = "anaphoraCount")] anaphora_count: i64,
        #[serde(rename = "isocolonCount")] isocolon_count: i64,
        #[serde(rename = "climaxPatternCount")] climax_pattern_count: i64,
    }
    #[derive(Deserialize)]
    struct LabelsFx {
        #[serde(rename = "nounVerb")] noun_verb: String,
        #[serde(rename = "parataxisHypotaxis")] parataxis_hypotaxis: String,
        #[serde(rename = "periodicRunning")] periodic_running: String,
        voice: String,
        #[serde(rename = "primaryRegister")] primary_register: String,
        #[serde(rename = "registerMixed")] register_mixed: bool,
        opacity: String,
    }
    #[derive(Deserialize)]
    struct FullFx {
        text: String,
        #[serde(rename = "nounVerbRatio")] noun_verb_ratio: f64,
        #[serde(rename = "nominalizationDensity")] nominalization_density: f64,
        #[serde(rename = "prepositionalPhraseDensity")] prepositional_phrase_density: f64,
        #[serde(rename = "beVerbRatio")] be_verb_ratio: f64,
        #[serde(rename = "parataxisHypotaxisRatio")] parataxis_hypotaxis_ratio: f64,
        #[serde(rename = "coordinatingConjunctionDensity")] coordinating_conjunction_density: f64,
        #[serde(rename = "subordinatingConjunctionDensity")] subordinating_conjunction_density: f64,
        #[serde(rename = "periodicRunningRatio")] periodic_running_ratio: f64,
        #[serde(rename = "preMainVerbClauseCount")] pre_main_verb_clause_count: f64,
        #[serde(rename = "voiceScore")] voice_score: f64,
        #[serde(rename = "dynamicRange")] dynamic_range: f64,
        #[serde(rename = "latinateGermanicRatio")] latinate_germanic_ratio: f64,
        #[serde(rename = "registerMarkednessScore")] register_markedness_score: f64,
        #[serde(rename = "opacityScore")] opacity_score: f64,
        #[serde(rename = "selfConsciousnessScore")] self_consciousness_score: f64,
        #[serde(rename = "tacitPatterns")] tacit_patterns: TacitFx,
        labels: LabelsFx,
    }

    #[test]
    fn full_analysis_matches_ts_reference() {
        let fixtures: Vec<FullFx> = serde_json::from_str(&golden("golden_full.json")).unwrap();
        assert!(!fixtures.is_empty());
        let approx = |a: f64, b: f64, name: &str, t: &str| {
            assert!((a - b).abs() < 1e-9, "{name}: got {a}, want {b} for:\n  {t}");
        };
        for fx in &fixtures {
            let m = full_analysis(&fx.text);
            approx(m.noun_verb_ratio, fx.noun_verb_ratio, "noun_verb_ratio", &fx.text);
            approx(m.nominalization_density, fx.nominalization_density, "nominalization_density", &fx.text);
            approx(m.prepositional_phrase_density, fx.prepositional_phrase_density, "prepositional_phrase_density", &fx.text);
            approx(m.be_verb_ratio, fx.be_verb_ratio, "be_verb_ratio", &fx.text);
            approx(m.parataxis_hypotaxis_ratio, fx.parataxis_hypotaxis_ratio, "parataxis_hypotaxis_ratio", &fx.text);
            approx(m.coordinating_conjunction_density, fx.coordinating_conjunction_density, "coord_density", &fx.text);
            approx(m.subordinating_conjunction_density, fx.subordinating_conjunction_density, "subord_density", &fx.text);
            approx(m.periodic_running_ratio, fx.periodic_running_ratio, "periodic_running_ratio", &fx.text);
            approx(m.pre_main_verb_clause_count, fx.pre_main_verb_clause_count, "pre_main_verb_clause_count", &fx.text);
            approx(m.voice_score, fx.voice_score, "voice_score", &fx.text);
            approx(m.dynamic_range, fx.dynamic_range, "dynamic_range", &fx.text);
            approx(m.latinate_germanic_ratio, fx.latinate_germanic_ratio, "latinate_germanic_ratio", &fx.text);
            approx(m.register_markedness_score, fx.register_markedness_score, "register_markedness_score", &fx.text);
            approx(m.opacity_score, fx.opacity_score, "opacity_score", &fx.text);
            approx(m.self_consciousness_score, fx.self_consciousness_score, "self_consciousness_score", &fx.text);
            let tp = &m.tacit_patterns;
            let fp = &fx.tacit_patterns;
            approx(tp.alliteration_density, fp.alliteration_density, "alliteration_density", &fx.text);
            approx(tp.polyptoton_density, fp.polyptoton_density, "polyptoton_density", &fx.text);
            assert_eq!(tp.chiasmus_count, fp.chiasmus_count, "chiasmus_count: {}", fx.text);
            assert_eq!(tp.antithesis_count, fp.antithesis_count, "antithesis_count: {}", fx.text);
            assert_eq!(tp.anaphora_count, fp.anaphora_count, "anaphora_count: {}", fx.text);
            assert_eq!(tp.isocolon_count, fp.isocolon_count, "isocolon_count: {}", fx.text);
            assert_eq!(tp.climax_pattern_count, fp.climax_pattern_count, "climax_pattern_count: {}", fx.text);
            let (lm, lf) = (&m.labels, &fx.labels);
            assert_eq!(lm.noun_verb, lf.noun_verb, "label nounVerb: {}", fx.text);
            assert_eq!(lm.parataxis_hypotaxis, lf.parataxis_hypotaxis, "label parataxis: {}", fx.text);
            assert_eq!(lm.periodic_running, lf.periodic_running, "label periodic: {}", fx.text);
            assert_eq!(lm.voice, lf.voice, "label voice: {}", fx.text);
            assert_eq!(lm.primary_register, lf.primary_register, "label register: {}", fx.text);
            assert_eq!(lm.register_mixed, lf.register_mixed, "label registerMixed: {}", fx.text);
            assert_eq!(lm.opacity, lf.opacity, "label opacity: {}", fx.text);
        }
    }

    #[test]
    fn clamp_basic() {
        assert_eq!(clamp(1.5, 0.0, 1.0), 1.0);
        assert_eq!(clamp(-0.2, 0.0, 1.0), 0.0);
    }
}
