//! Claim extraction for repository path assertions.

use super::{Claim, PathClaim};
mod predicate;
mod reporting;
use predicate::claim_before;
use reporting::reported_claim_clause;

const ABSENT_AFTER: &[&str] = &[
    "does not exist yet",
    "does not yet exist",
    "does not exist",
    "do not exist yet",
    "do not yet exist",
    "do not exist",
    "doesn't exist yet",
    "doesn't exist",
    "don't exist",
    "is not present",
    "are not present",
    "is absent",
    "are absent",
    "absent",
    "is missing",
    "are missing",
    "missing",
    "will be created",
    "to be created",
    "must be created",
    "needs to be created",
    "is new",
    "is a new file",
    "is a new module",
    "(new)",
    "new",
];
/// Phrases that, directly after the path, assert it is present.
const EXISTS_AFTER: &[&str] = &[
    "already exists",
    "exists",
    "exist",
    "is present",
    "are present",
    "present",
    "will be modified",
    "will be extended",
    "will be edited",
    "will be updated",
    "to be modified",
    "to be extended",
    "is modified",
    "is extended",
];
/// Words allowed between the path and its phrase without changing the subject.
const FILLER: &[&str] = &[
    "file",
    "module",
    "directory",
    "dir",
    "crate",
    "path",
    "currently",
    "already",
    "still",
    "now",
    "which",
    "that",
    "is",
    "are",
    "was",
    "(",
    ":",
    "—",
    "-",
    "–",
    ",",
];
/// What may follow a phrase for it to be the whole predicate.
const TERMINATORS: &[&str] = &[
    "",
    ".",
    ",",
    ";",
    ":",
    ")",
    "]",
    "(",
    "yet",
    "today",
    "currently",
    "in the repository",
    "in the repo",
    "at",
    "in",
    "within",
    "and",
    "but",
    "—",
    "-",
    "–",
    "so",
    "under",
];

/// Every unambiguous claim in `text`, in order. Fenced code blocks are not
/// prose and are skipped; inline backticks are read. What a specified test
/// asserts at run time is no claim (`assertion`, #443).
pub(crate) fn extract_claims(text: &str) -> Vec<PathClaim> {
    super::assertion::sentences(text)
        .into_iter()
        .flat_map(|(sentence, carried)| claims_in_sentence(sentence, carried))
        .collect()
}

fn claims_in_sentence(sentence: &str, carried: bool) -> Vec<PathClaim> {
    let (mut claims, leading_paths) = leading_absence_claims(sentence).unwrap_or_default();
    let mut rest = sentence;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('`') else {
            break;
        };
        let token = &after_open[..close];
        let before = &rest[..open];
        let after = &after_open[close + 1..];
        let at = sentence.len() - rest.len() + open;
        if !leading_paths.contains(&at)
            && !is_incomplete_path_fragment(before, after)
            && !reported_claim_clause(&sentence[..at])
            && let Some(path) = repository_or_scoped_token(token, after)
        {
            // The predicate after the path may be what a test asserts; a
            // word before it is the body's own statement.
            let claim = claim_after(after)
                .or_else(|| explicit_scope(after).map(|(_, claim)| claim))
                .filter(|_| !super::assertion::exempt(sentence, carried, at, after))
                .or_else(|| claim_before(before));
            if let Some(claim) = claim {
                let scope = explicit_scope(after).map(|(scope, _)| scope);
                claims.push(PathClaim {
                    path,
                    claim,
                    scope,
                    scoped_join: false,
                    sentence: sentence.trim().to_string(),
                });
            }
        }
        rest = after;
    }
    claims
}

/// Sentence-leading negation applies to a short subject noun phrase and each
/// path in its coordinated list. Ambiguous prose remains unclaimed. A bare
/// file name is a claim only under an explicit scope:
/// "No `Cargo.toml` is present in the new crate" names no repository path.
fn leading_absence_claims(sentence: &str) -> Option<(Vec<PathClaim>, Vec<usize>)> {
    let trimmed = sentence.trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    let (lead, conjunction) = if lowered.starts_with("neither ") {
        ("neither ", " nor ")
    } else if lowered.starts_with("none of ") {
        ("none of ", " or ")
    } else if lowered.starts_with("there is no ") {
        ("there is no ", " or ")
    } else if lowered.starts_with("no ") {
        ("no ", " or ")
    } else {
        return None;
    };
    let after_lead = &trimmed[lead.len()..];
    let Some(path_offset) = after_lead.find('`') else {
        return None;
    };
    let subject = after_lead[..path_offset].trim();
    // Clause punctuation ends the scope of the sentence-leading negator.
    // The path after it belongs to the normal per-path reader.
    if subject.contains([';', ',']) {
        return None;
    }
    // `none of the files` has the same noun phrase as `No files`; the
    // article is part of that negation form rather than a subject modifier.
    let subject = subject
        .strip_prefix("the ")
        .filter(|_| lead == "none of ")
        .unwrap_or(subject);
    let ambiguous_subject = !subject.is_empty() && !is_unambiguous_absence_subject(subject);
    let mut rest = &after_lead[path_offset..];
    let mut paths = Vec::new();
    let mut offsets = Vec::new();
    loop {
        rest = rest.trim_start();
        let offset = sentence.len() - rest.len();
        let token = rest.strip_prefix('`')?;
        let close = token.find('`')?;
        let path = match repository_relative_token(&token[..close]) {
            Some(path) => (path, false),
            None => (bare_repository_file_token(&token[..close])?, true),
        };
        paths.push(path);
        offsets.push(offset);
        rest = &token[close + 1..];
        let lowered_rest = rest.to_ascii_lowercase();
        if lowered_rest.starts_with(conjunction) {
            rest = &rest[conjunction.len()..];
            continue;
        }
        if matches!(claim_after(rest), Some(Claim::Exists)) {
            if ambiguous_subject {
                return Some((Vec::new(), offsets));
            }
            let scope = explicit_scope(rest).map(|(scope, _)| scope);
            return Some((
                paths
                    .into_iter()
                    .filter(|(_, bare)| !bare || scope.is_some())
                    .map(|(path, _)| PathClaim {
                        path,
                        claim: Claim::Absent,
                        scope: scope.clone(),
                        scoped_join: false,
                        sentence: sentence.trim().to_string(),
                    })
                    .collect(),
                offsets,
            ));
        }
        return None;
    }
}

/// Whether the complete negated subject has a path noun as its head, with
/// only the supported modifiers and directly attached path qualifier.
fn is_unambiguous_absence_subject(subject: &str) -> bool {
    let words = subject.to_ascii_lowercase();
    let words = words.split_whitespace().collect::<Vec<_>>();
    let words = if words
        .first()
        .is_some_and(|word| matches!(*word, "such" | "other" | "separate" | "additional"))
    {
        &words[1..]
    } else {
        &words[..]
    };

    let Some(noun) = words.first() else {
        return false;
    };
    if !matches!(
        *noun,
        "path"
            | "paths"
            | "file"
            | "files"
            | "module"
            | "modules"
            | "directory"
            | "directories"
            | "dir"
            | "dirs"
            | "crate"
            | "crates"
            | "entry"
            | "entries"
            | "artifact"
            | "artifacts"
    ) {
        return false;
    }

    match &words[1..] {
        [] => true,
        ["named"] | ["called"] | ["like"] | ["at"] => true,
        ["of", "the", "form"] => true,
        _ => false,
    }
}

/// Return a directory scope only when it directly follows an existence
/// predicate, after the same filler words `claim_after` skips ("`foo.rs`
/// currently exists in `src/`"). Trailing prose elsewhere in the sentence
/// cannot scope a path. The scope keeps its original spelling.
fn explicit_scope(after: &str) -> Option<(String, Claim)> {
    let original = after.trim_start();
    let lowered = original.to_ascii_lowercase();
    let mut rest = lowered.as_str();
    for _ in 0..4 {
        if let Some((tail, claim)) = scoped_predicate_at_start(rest) {
            // ASCII lowering keeps byte offsets, so the token is cut from the
            // original text and its case survives.
            let token = original[lowered.len() - tail.len()..].strip_prefix('`')?;
            let end = token.find('`')?;
            return repository_directory_token(&token[..end]).map(|scope| (scope, claim));
        }
        rest = skip_filler(rest)?;
    }
    None
}

/// The text after `<predicate> under ` or `<predicate> in ` at the start of
/// `rest`, with the predicate's claim.
fn scoped_predicate_at_start(rest: &str) -> Option<(&str, Claim)> {
    [(ABSENT_AFTER, Claim::Absent), (EXISTS_AFTER, Claim::Exists)]
        .iter()
        .find_map(|(phrases, claim)| {
            phrases.iter().find_map(|phrase| {
                let tail = rest.strip_prefix(phrase)?.trim_start();
                ["under ", "in "]
                    .iter()
                    .find_map(|scope| tail.strip_prefix(scope))
                    .map(|tail| (tail, *claim))
            })
        })
}

/// `rest` (lowercase) without one leading filler word, or `None` when it does
/// not start with one.
fn skip_filler(rest: &str) -> Option<&str> {
    FILLER.iter().find_map(|filler| {
        let stripped = rest.strip_prefix(filler)?;
        let boundary = filler.chars().last().is_some_and(|c| !c.is_alphanumeric())
            || stripped.starts_with([' ', '(', ':', ',', '—', '-']);
        boundary.then(|| stripped.trim_start())
    })
}

/// A scope candidate is repository-relative syntax. The repository tree,
/// rather than its spelling, determines whether it names a file or directory;
/// a trailing `/` is kept, because it spells a directory the tree may lack
/// (`docs/v1.3/`). A module path (`crate::widgets`) is no scope.
fn repository_directory_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty()
        || token.chars().any(char::is_whitespace)
        || token.contains("::")
        || token.contains(['(', ')'])
        || token.starts_with('/')
        || token.starts_with('~')
        || token.contains("://")
        || token.contains(['*', '{', '}', '$', '<', '>', '|', '=', '"', '\''])
        || token.contains("...")
        || token.contains('…')
        || token.starts_with("..")
        || token.contains("/..")
    {
        return None;
    }
    let normalized = archon_workflow::repository_record::normalize_relative(token);
    if normalized.is_empty() {
        return None;
    }
    Some(if token.ends_with('/') {
        format!("{normalized}/")
    } else {
        normalized
    })
}

/// Bare file names may be joined to an explicit scope. A token that already
/// has a path component remains root-relative; the tree-aware checker decides
/// whether such a scoped claim is unambiguous.
fn repository_or_scoped_token(token: &str, after: &str) -> Option<String> {
    repository_relative_token(token).or_else(|| {
        explicit_scope(after)?;
        bare_repository_file_token(token)
    })
}

fn bare_repository_file_token(token: &str) -> Option<String> {
    let file = token.trim();
    let (stem, extension) = file.rsplit_once('.')?;
    (!file.contains('/')
        && !file.chars().any(char::is_whitespace)
        && !file.contains(['(', ')'])
        && !stem.is_empty()
        && !extension.is_empty()
        && extension.len() <= 8
        && extension.chars().all(|c| c.is_ascii_alphanumeric()))
    .then(|| file.to_string())
}

/// Do not treat a backticked component as a repository path when it is one
/// segment of a larger path containing a placeholder or ellipsis, such as
/// `<root>/…/`backtests`/…`.
fn is_incomplete_path_fragment(before: &str, after: &str) -> bool {
    let left = before
        .rsplit_once(char::is_whitespace)
        .map_or(before, |(_, tail)| tail);
    let right = after.split_whitespace().next().unwrap_or(after);
    let left_is_path_prefix = left.ends_with('/') && (left.contains("...") || left.contains('…'));
    let right_is_path_suffix =
        right.starts_with('/') && (right.contains("...") || right.contains('…'));
    left_is_path_prefix || right_is_path_suffix
}

/// A backticked token that reads as a file or directory path: no whitespace,
/// a `/`, no glob or template characters, and a file extension on its last
/// component or a trailing `/`. Relative tokens are normalised; an absolute
/// token (the `<repo>/<relative path>` citation the body author is asked
/// for) is kept whole and resolved against the repository root later, where
/// one outside the root is dropped.
pub(crate) fn repository_relative_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty()
        || token.chars().any(char::is_whitespace)
        || !token.contains('/')
        || token.starts_with('~')
        || token.contains(['(', ')'])
        || token.contains("://")
        || token.contains(['*', '{', '}', '$', '<', '>', '|', '=', '"', '\''])
        || token.contains("...")
        || token.contains('…')
        || token.starts_with("..")
        || token.contains("/..")
    {
        return None;
    }
    let absolute = token.starts_with('/');
    let normalized = archon_workflow::repository_record::normalize_relative(token);
    if normalized.is_empty() {
        return None;
    }
    let kept = if absolute {
        format!("/{normalized}")
    } else {
        normalized.clone()
    };
    if token.ends_with('/') {
        return Some(kept);
    }
    let last = normalized.rsplit('/').next().unwrap_or(&normalized);
    has_file_extension(last).then_some(kept)
}

pub(super) fn has_file_extension(component: &str) -> bool {
    let Some((stem, extension)) = component.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !extension.is_empty()
        && extension.len() <= 8
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
}

fn claim_after(after: &str) -> Option<Claim> {
    let lowered = after.trim_start().to_ascii_lowercase();
    let mut rest = lowered.as_str();
    for _ in 0..4 {
        if let Some(claim) = phrase_at_start(rest) {
            return Some(claim);
        }
        rest = skip_filler(rest)?;
    }
    None
}

fn phrase_at_start(rest: &str) -> Option<Claim> {
    for (phrases, claim) in [(ABSENT_AFTER, Claim::Absent), (EXISTS_AFTER, Claim::Exists)] {
        for phrase in phrases {
            if let Some(tail) = rest.strip_prefix(phrase)
                && TERMINATORS
                    .iter()
                    .any(|terminator| terminates(tail, terminator))
            {
                return Some(claim);
            }
        }
    }
    None
}

/// Does `tail` (what follows a phrase) begin with `terminator` as a whole
/// word, or is it empty for the empty terminator.
fn terminates(tail: &str, terminator: &str) -> bool {
    let tail = tail.trim_start();
    if terminator.is_empty() {
        return tail.is_empty();
    }
    let Some(rest) = tail.strip_prefix(terminator) else {
        return false;
    };
    let word = terminator.chars().last().is_some_and(char::is_alphanumeric);
    !word || rest.chars().next().is_none_or(|c| !c.is_alphanumeric())
}
