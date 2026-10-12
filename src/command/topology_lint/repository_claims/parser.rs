//! Claim extraction for repository path assertions.

use super::{Claim, PathClaim};

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
/// Verbs directly before the path that assert it is present (the body will
/// change a file it has), and phrases that assert it is absent (a new file).
const EXISTS_BEFORE: &[&str] = &[
    "modify",
    "modifies",
    "modifying",
    "extend",
    "extends",
    "extending",
    "edit",
    "edits",
    "editing",
    "update",
    "updates",
    "updating",
    "existing file",
    "existing module",
    "existing",
];
const ABSENT_BEFORE: &[&str] = &[
    "new file",
    "new module",
    "new crate",
    "new directory",
    "create new",
    "creates new",
    "add new",
    "adds new",
    "a new file",
    "a new module",
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
/// prose and are skipped; inline backticks are read.
pub(crate) fn extract_claims(text: &str) -> Vec<PathClaim> {
    let mut claims = Vec::new();
    for line in super::super::fences::prose_lines(text) {
        for sentence in split_sentences(line) {
            claims.extend(claims_in_sentence(sentence));
        }
    }
    claims
}

fn split_sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if matches!(bytes[index], b'.' | b';' | b'!' | b'?') && bytes[index + 1] == b' ' {
            out.push(&line[start..=index]);
            start = index + 1;
        }
        index += 1;
    }
    out.push(&line[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn claims_in_sentence(sentence: &str) -> Vec<PathClaim> {
    if let Some(claims) = leading_absence_claims(sentence) {
        return claims;
    }
    let mut claims = Vec::new();
    let mut rest = sentence;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('`') else {
            break;
        };
        let token = &after_open[..close];
        let before = &rest[..open];
        let after = &after_open[close + 1..];
        if !is_incomplete_path_fragment(before, after)
            && let Some(path) = repository_or_scoped_token(token, after)
        {
            let claim = claim_after(after)
                .or_else(|| explicit_scope(after).map(|(_, claim)| claim))
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

/// A sentence-leading `No` or `Neither` negates each path in its coordinated
/// list. The list is accepted only when its final path has an existence
/// predicate, so incomplete or ambiguous prose remains unclaimed. A bare file
/// name is a claim only under the explicit scope that follows the predicate:
/// "No `Cargo.toml` is present in the new crate" names no repository path.
fn leading_absence_claims(sentence: &str) -> Option<Vec<PathClaim>> {
    let trimmed = sentence.trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    let (lead, conjunction) = if lowered.starts_with("neither ") {
        ("neither ", " nor ")
    } else if lowered.starts_with("no ") {
        ("no ", " or ")
    } else {
        return None;
    };
    let mut rest = &trimmed[lead.len()..];
    let mut paths = Vec::new();
    loop {
        rest = rest.trim_start();
        let token = rest.strip_prefix('`')?;
        let close = token.find('`')?;
        let path = match repository_relative_token(&token[..close]) {
            Some(path) => (path, false),
            None => (bare_repository_file_token(&token[..close])?, true),
        };
        paths.push(path);
        rest = &token[close + 1..];
        let lowered_rest = rest.to_ascii_lowercase();
        if lowered_rest.starts_with(conjunction) {
            rest = &rest[conjunction.len()..];
            continue;
        }
        if matches!(claim_after(rest), Some(Claim::Exists)) {
            let scope = explicit_scope(rest).map(|(scope, _)| scope);
            return Some(
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
            );
        }
        return None;
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
/// rather than its spelling, determines whether it names a file or directory.
fn repository_directory_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty()
        || token.chars().any(char::is_whitespace)
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
    Some(normalized)
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

fn claim_before(before: &str) -> Option<Claim> {
    let lowered = before.trim_end().to_ascii_lowercase();
    let mut rest = lowered.as_str();
    for _ in 0..3 {
        for (phrases, claim) in [
            (ABSENT_BEFORE, Claim::Absent),
            (EXISTS_BEFORE, Claim::Exists),
        ] {
            for phrase in phrases {
                if let Some(head) = rest.strip_suffix(phrase)
                    && head.chars().last().is_none_or(|c| !c.is_alphanumeric())
                {
                    return Some(claim);
                }
            }
        }
        // A filler word is stripped only as a whole word: "schema" must not
        // lose its "a" and become something else.
        let trimmed = [
            "the",
            "file",
            "module",
            "directory",
            "crate",
            "a",
            "an",
            ":",
            "-",
            "—",
        ]
        .iter()
        .find_map(|filler| {
            let head = rest.strip_suffix(filler)?;
            let whole = !filler.chars().last().is_some_and(char::is_alphanumeric)
                || head.chars().last().is_none_or(|c| !c.is_alphanumeric());
            whole.then(|| head.trim_end())
        });
        match trimmed {
            Some(head) if head.len() < rest.len() => rest = head,
            _ => return None,
        }
    }
    None
}
