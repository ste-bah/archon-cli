//! The canonical `task_hash` — a stable key for *the kind of work*, not the
//! instance of it.
//!
//! # Why this lives here
//!
//! `agent_performance_ledger.task_hash`
//! (`archon-learning/src/schema.rs`) records *who* did the work and how it
//! went. The milestone 2 topology corpus records *what shape* the work ran in.
//! Neither is useful alone: the question worth answering is "for this class of
//! task, which topology outperformed which", and that is a join on
//! `task_hash`. Until this module existed the column was a caller-supplied
//! `Option<String>` with no derivation anywhere in the tree, so the join had no
//! key.
//!
//! It lives in `archon-topology` because that crate is downstream of nothing:
//! it has no `cozo`, no `archon-core`, and no learning dependency, so both
//! corpora can call it. This module is deliberately outside the `trace` and
//! `workflow` feature gates — it depends on nothing beyond `core` — so
//! `archon-core`, which takes this crate with `default-features = false`, can
//! adopt it too.
//!
//! # What "stable" means
//!
//! Two runs of *equivalent* work must produce the same hash. That forces three
//! properties on the derivation:
//!
//! 1. **No volatile identifiers.** Run ids, session ids, uuids, timestamps,
//!    absolute paths, and bare numbers are stripped before hashing. A task
//!    description that mentions `wf-6f1c…` or `C:\repo\src\lib.rs` must hash
//!    the same as one that mentions `wf-91ab…` or `/home/x/src/lib.rs`.
//! 2. **Order-insensitive.** Tokens are deduplicated and sorted, so "fix the
//!    crash in the parser" and "the parser crash, fix it" agree.
//! 3. **Class-separated.** The class is both the visible prefix and part of the
//!    hashed body, so the same words under a different class can never collide.
//!
//! # What it is not
//!
//! Not cryptographic. This is a corpus bucket key; an adversary who wants a
//! collision can have one, and it would buy them nothing. FNV-1a is used
//! because it is a dozen lines and fully specified — `std`'s `DefaultHasher` is
//! explicitly documented as *not* stable across Rust releases, which would
//! silently re-bucket the entire corpus on a toolchain bump.

use std::collections::BTreeSet;
use std::fmt;

/// The class of work a task represents.
///
/// These five are the axis milestone 5 conditions on. They are not a taxonomy
/// of everything an agent can be asked to do — they are the coarsest split for
/// which "a diamond with three verifiers beats a linear chain" plausibly has a
/// different answer per bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskClass {
    /// Behaviour-preserving change: extract, simplify, restructure, deduplicate.
    Refactor,
    /// Find and fix a defect: crash, panic, regression, failing test.
    BugHunt,
    /// Move something from one form to another: upgrade, port, backport, bump.
    Migration,
    /// Assess without changing: audit, inspect, critique.
    Review,
    /// Build something that does not exist yet. Also the fallback.
    Greenfield,
}

impl TaskClass {
    /// Stable wire form. Used as the hash prefix and stored verbatim, so these
    /// strings are part of the corpus schema — changing one invalidates every
    /// row already written.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refactor => "refactor",
            Self::BugHunt => "bug-hunt",
            Self::Migration => "migration",
            Self::Review => "review",
            Self::Greenfield => "greenfield",
        }
    }

    /// Inverse of [`TaskClass::as_str`].
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "refactor" => Some(Self::Refactor),
            "bug-hunt" => Some(Self::BugHunt),
            "migration" => Some(Self::Migration),
            "review" => Some(Self::Review),
            "greenfield" => Some(Self::Greenfield),
            _ => None,
        }
    }

    /// Every class, in declaration order. Declaration order is also the
    /// tie-break order used by [`classify_task`].
    #[must_use]
    pub fn all() -> [Self; 5] {
        [
            Self::Refactor,
            Self::BugHunt,
            Self::Migration,
            Self::Review,
            Self::Greenfield,
        ]
    }
}

impl fmt::Display for TaskClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Marker substrings that vote for a class.
///
/// Matched as substrings of a normalized token, not as whole words, so
/// `refactoring` / `refactored` / `refactor` all hit `refactor`. Kept short and
/// unambiguous for that reason: a marker that is a prefix of an unrelated word
/// costs a misclassification.
const REFACTOR_MARKERS: &[&str] = &[
    "refactor",
    "cleanup",
    "tidy",
    "simplif",
    "restructur",
    "reorganiz",
    "deduplicat",
    "dedupe",
    "extract",
    "inline",
    "rename",
    "decompos",
    "untangle",
];

const BUG_HUNT_MARKERS: &[&str] = &[
    "bug",
    "defect",
    "crash",
    "panic",
    "regress",
    "repro",
    "hang",
    "deadlock",
    "flake",
    "leak",
    "corrupt",
    "broken",
    "failing",
    "misbehav",
    "traceback",
    "stacktrace",
    "debug",
    "diagnos",
    "triage",
];

const MIGRATION_MARKERS: &[&str] = &[
    "migrat",
    "upgrade",
    "downgrade",
    "backport",
    "port",
    "bump",
    "deprecat",
    "backfill",
    "convert",
    "transition",
    "cutover",
    "rollout",
];

const REVIEW_MARKERS: &[&str] = &[
    "review",
    "audit",
    "inspect",
    "critique",
    "assess",
    "evaluat",
    "survey",
    "appraise",
    "walkthrough",
];

const GREENFIELD_MARKERS: &[&str] = &[
    "implement",
    "introduce",
    "scaffold",
    "greenfield",
    "prototype",
    "bootstrap",
    "author",
    "design",
    "build",
];

/// Words carrying no class signal and no identity signal. Dropped before
/// hashing so filler phrasing does not perturb the key.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "onto", "over", "under", "then",
    "than", "when", "where", "which", "while", "should", "would", "could", "must", "will", "shall",
    "can", "may", "please", "just", "also", "very", "make", "made", "does", "did", "done", "has",
    "have", "had", "was", "were", "are", "its", "it's", "our", "your", "their", "them", "there",
    "here", "all", "any", "some", "each", "both", "not", "but", "out", "off", "via", "per", "you",
    "we", "us", "so", "do", "be", "to", "of", "in", "on", "at", "by", "as", "is", "it", "or", "if",
    "an", "a",
];

/// Pick a class from free text.
///
/// Scoring rather than first-match: "fix the migration script" contains markers
/// for two classes, and a first-match rule would silently make the answer
/// depend on the order the marker tables happen to be declared in. Every marker
/// hit is one vote; the highest total wins; ties break toward the earliest
/// [`TaskClass`] variant so the result is deterministic. Zero votes anywhere
/// falls to [`TaskClass::Greenfield`], because "build something new" is the
/// least presumptuous default — it asserts nothing about an existing artifact.
#[must_use]
pub fn classify_task(text: &str) -> TaskClass {
    let tokens = normalize_task_text(text);
    let mut best = TaskClass::Greenfield;
    let mut best_score = 0usize;

    for class in TaskClass::all() {
        let markers = match class {
            TaskClass::Refactor => REFACTOR_MARKERS,
            TaskClass::BugHunt => BUG_HUNT_MARKERS,
            TaskClass::Migration => MIGRATION_MARKERS,
            TaskClass::Review => REVIEW_MARKERS,
            TaskClass::Greenfield => GREENFIELD_MARKERS,
        };
        let score = tokens
            .iter()
            .filter(|token| markers.iter().any(|marker| token.contains(marker)))
            .count();
        // Strictly greater keeps the earliest variant on a tie.
        if score > best_score {
            best_score = score;
            best = class;
        }
    }

    best
}

/// Derive the canonical `task_hash` for free text, classifying it first.
///
/// The returned form is `"<class>:<16 lowercase hex digits>"`, e.g.
/// `"bug-hunt:0f3c9a1b2d4e5f60"`. The class is visible on purpose: a corpus
/// query can bucket by class with a prefix match instead of re-deriving the
/// classification, and a human reading a ledger row can tell what it keys.
#[must_use]
pub fn task_hash(text: &str) -> String {
    task_hash_for_class(classify_task(text), text)
}

/// Derive the canonical `task_hash` with the class supplied by the caller.
///
/// Use this when the class is already known from structure rather than from
/// prose — a `/workflow` spec that declares its intent, for instance — so the
/// keyword heuristic cannot override a fact.
#[must_use]
pub fn task_hash_for_class(class: TaskClass, text: &str) -> String {
    let tokens = normalize_task_text(text);
    let mut hasher = Fnv1a::new();
    // The class is hashed as well as prefixed. Prefixing alone would let a
    // consumer that compares only the hex half see a false match across
    // classes, and "differs across task classes" is a property the corpus
    // depends on rather than a presentational nicety.
    hasher.write(class.as_str().as_bytes());
    hasher.write(b"\0");
    for token in &tokens {
        hasher.write(token.as_bytes());
        hasher.write(b"\n");
    }
    format!("{}:{:016x}", class.as_str(), hasher.finish())
}

/// Reduce free text to the sorted, deduplicated token set the hash is taken
/// over.
///
/// Exposed because the normalization *is* the contract — a caller debugging why
/// two descriptions hashed differently needs to see what survived. Volatile
/// material is removed here and nowhere else:
///
/// - path-shaped tokens collapse to their final segment, so
///   `C:\repo\src\lib.rs` and `/home/x/src/lib.rs` both become `lib.rs`;
/// - uuid-shaped tokens are dropped entirely;
/// - `wf-…` / `run-…` / `session-…` style identifiers are dropped;
/// - date- and time-shaped tokens are dropped;
/// - tokens that are all digits, or that contain a run of six or more digits,
///   are dropped — issue numbers, line numbers, byte counts, and epoch stamps
///   all live in that shape and none of them describe the *kind* of work;
/// - long hexadecimal runs are dropped, which catches commit shas and content
///   hashes;
/// - tokens shorter than three characters, and stopwords, are dropped.
#[must_use]
pub fn normalize_task_text(text: &str) -> Vec<String> {
    let mut tokens = BTreeSet::new();

    for raw in text.split_whitespace() {
        let raw = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '\\');
        if raw.is_empty() {
            continue;
        }
        // Collapse a path to its basename before anything else looks at it. An
        // absolute path is the most common volatile identifier in a task
        // description and its final segment is the only stable part.
        let candidate = basename_of(raw);
        if is_volatile_token(&candidate.to_ascii_lowercase()) {
            continue;
        }

        for part in candidate.split(|c: char| !c.is_alphanumeric()) {
            let part = part.to_ascii_lowercase();
            if part.len() < 3 {
                continue;
            }
            if STOPWORDS.contains(&part.as_str()) {
                continue;
            }
            if is_volatile_token(&part) {
                continue;
            }
            tokens.insert(part);
        }
    }

    tokens.into_iter().collect()
}

/// Final path segment, for tokens that look like paths. Returns the input
/// unchanged otherwise.
fn basename_of(token: &str) -> &str {
    if !token.contains('/') && !token.contains('\\') {
        return token;
    }
    token
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(token)
}

/// True for tokens that identify an *instance* of work rather than its kind.
fn is_volatile_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if is_uuid_shaped(token) {
        return true;
    }
    if is_identifier_prefixed(token) {
        return true;
    }
    if is_date_shaped(token) {
        return true;
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if has_long_digit_run(token, 6) {
        return true;
    }
    // A long all-hex token is a sha, a content hash, or a truncated uuid.
    // Eight is the shortest abbreviated git sha in common use.
    if token.len() >= 8 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    false
}

/// `8-4-4-4-12` hexadecimal, the canonical uuid rendering.
fn is_uuid_shaped(token: &str) -> bool {
    let groups: Vec<&str> = token.split('-').collect();
    if groups.len() != 5 {
        return false;
    }
    const WIDTHS: [usize; 5] = [8, 4, 4, 4, 12];
    groups
        .iter()
        .zip(WIDTHS)
        .all(|(group, width)| group.len() == width && group.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `wf-…`, `run-…`, `session-…` and friends: a known-volatile prefix followed
/// by anything. The prefix alone is not enough — `run-tests` must survive — so
/// the suffix must also look like an identifier rather than a word.
fn is_identifier_prefixed(token: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "wf-", "run-", "sess-", "session-", "graph-", "trace-", "job-", "attempt-", "req-",
        "uuid-", "id-",
    ];
    let Some(suffix) = PREFIXES
        .iter()
        .find_map(|prefix| token.strip_prefix(prefix))
    else {
        return false;
    };
    // An identifier suffix is hex-ish or numeric; a word suffix is not.
    !suffix.is_empty()
        && suffix
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_')
        && suffix.chars().any(|c| c.is_ascii_digit())
}

/// `YYYY-MM-DD` or an RFC3339-ish timestamp beginning with one.
fn is_date_shaped(token: &str) -> bool {
    let bytes = token.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    let digits_at =
        |range: std::ops::Range<usize>| range.into_iter().all(|i| bytes[i].is_ascii_digit());
    digits_at(0..4) && bytes[4] == b'-' && digits_at(5..7) && bytes[7] == b'-' && digits_at(8..10)
}

/// True when `token` contains `len` or more consecutive ASCII digits.
fn has_long_digit_run(token: &str, len: usize) -> bool {
    let mut run = 0usize;
    for c in token.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= len {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// FNV-1a, 64-bit.
///
/// Hand-rolled rather than pulled from a crate for the reason the whole crate
/// is dependency-thin, and chosen over `std::collections::hash_map::
/// DefaultHasher` because that one's output is explicitly unspecified across
/// Rust releases. A corpus key that silently changes on a toolchain upgrade is
/// worse than no corpus key.
struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests;
