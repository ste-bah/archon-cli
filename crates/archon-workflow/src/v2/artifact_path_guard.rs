//! One place that answers two questions the runtime kept answering by accident:
//! *is this string a path at all?* and *is this file evidence?*
//!
//! # Why this exists (issue #168)
//!
//! Run `wf-67dd2599` left directories in the project root named after acceptance
//! criteria — `A gap-audit report ... from environment/readiness blockers/`. The
//! `/` inside the prose became a real separator, so one criterion produced a
//! nested tree of empty directories. The same run recorded `artifact_path`
//! values still carrying `${PROJECT_ROOT}` and `${DATASET_ID}`.
//!
//! Both are the same defect: a value that was never a path used as one. And the
//! reason it was not merely untidy is the second question — the declared
//! artifact check asked only whether the path *existed*. A directory exists. An
//! empty file exists. So the litter a criterion produced could satisfy the
//! contract that criterion described, and a run would report the contract met by
//! a directory containing nothing. That is the failure mode of issue #153
//! (fabricated success) reached by a different road.
//!
//! Nothing here creates or deletes anything. It refuses, and it reports.

use std::fmt;
use std::path::Path;

/// Longest single `/`-separated segment a declared artifact path may carry.
///
/// Generous on purpose: prose detection below is what catches a criterion, and a
/// length rule tight enough to catch prose on its own would refuse legitimate
/// long-but-real filenames. This is the backstop for prose that happens to be
/// terse and unpunctuated.
pub const MAX_ARTIFACT_SEGMENT_CHARS: usize = 96;

/// Longest whole declared artifact path, relative or absolute.
pub const MAX_ARTIFACT_PATH_CHARS: usize = 320;

/// Length at which a project-root entry name is reported as litter.
///
/// Deliberately tighter than [`MAX_ARTIFACT_SEGMENT_CHARS`], and deliberately
/// the number issue #168 verifies with (`ls -1 | awk 'length($0)>60'`). The two
/// thresholds answer different questions: the segment ceiling governs what the
/// runtime will *accept and hand to an agent*, where a false refusal costs a
/// real deliverable; this one governs what is *reported as already present*,
/// where a false report costs a line of text.
pub const LITTER_NAME_CHARS: usize = 60;

/// At this many whitespace-separated words, a path segment is a sentence.
const PROSE_WORD_COUNT: usize = 4;

const SENTENCE_PUNCTUATION: [char; 5] = [',', ';', ':', '!', '?'];

/// Why a value was refused as a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactPathRejection {
    /// Nothing, or only whitespace.
    Empty,
    /// A newline, tab, or other control character — never part of a real path.
    ControlCharacter,
    /// `${...}` or `<...>` survived into a value used as a path.
    UnexpandedTemplate { token: String },
    /// `${` with no closing `}`.
    MalformedTemplate,
    /// A `${NAME}` nothing binds. Never expanded to the empty string.
    UnboundTemplateVariable { name: String },
    /// A segment that reads as a sentence rather than a file name.
    Prose { segment: String },
    /// A single segment longer than [`MAX_ARTIFACT_SEGMENT_CHARS`].
    SegmentTooLong { segment: String, chars: usize },
    /// A whole path longer than [`MAX_ARTIFACT_PATH_CHARS`].
    TooLong { chars: usize },
}

impl fmt::Display for ArtifactPathRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "declared artifact path is empty"),
            Self::ControlCharacter => write!(
                formatter,
                "declared artifact path contains a control character"
            ),
            Self::UnexpandedTemplate { token } => write!(
                formatter,
                "declared artifact path still carries the unexpanded template token {token}; \
                 declare the concrete path that was written"
            ),
            Self::MalformedTemplate => write!(
                formatter,
                "declared artifact path opens '${{' without closing it"
            ),
            Self::UnboundTemplateVariable { name } => write!(
                formatter,
                "declared artifact path references ${{{name}}}, which nothing binds; an unset \
                 variable is an error here, never an empty expansion that silently makes the \
                 path relative"
            ),
            Self::Prose { segment } => write!(
                formatter,
                "declared artifact path segment reads as prose, not a file name: '{segment}'. \
                 Acceptance criteria and deliverable descriptions are prose fields; only \
                 artifact_path is a path"
            ),
            Self::SegmentTooLong { segment, chars } => write!(
                formatter,
                "declared artifact path segment is {chars} characters (limit \
                 {MAX_ARTIFACT_SEGMENT_CHARS}): '{segment}'"
            ),
            Self::TooLong { chars } => write!(
                formatter,
                "declared artifact path is {chars} characters (limit {MAX_ARTIFACT_PATH_CHARS})"
            ),
        }
    }
}

/// Accept `raw` as a declared artifact path, or say why not.
///
/// Applied before a path reaches a prompt, a schema, or an existence check — the
/// three positions where handing an agent a sentence is how a sentence becomes a
/// directory. Refusing loudly beats creating it.
pub fn validate_declared_artifact_path(raw: &str) -> Result<String, ArtifactPathRejection> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ArtifactPathRejection::Empty);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ArtifactPathRejection::ControlCharacter);
    }
    let chars = trimmed.chars().count();
    if chars > MAX_ARTIFACT_PATH_CHARS {
        return Err(ArtifactPathRejection::TooLong { chars });
    }
    for segment in trimmed.split(['/', '\\']).filter(|part| !part.is_empty()) {
        // Prose first: a sentence is usually also oversize, and "reads as
        // prose" tells the author what to change. "too long" does not.
        if segment_is_prose(segment) {
            return Err(ArtifactPathRejection::Prose {
                segment: segment.to_string(),
            });
        }
        let segment_chars = segment.chars().count();
        if segment_chars > MAX_ARTIFACT_SEGMENT_CHARS {
            return Err(ArtifactPathRejection::SegmentTooLong {
                segment: segment.to_string(),
                chars: segment_chars,
            });
        }
    }
    // Last, so that a sentence which happens to contain `<...>` is reported as
    // the sentence it is rather than as a template defect.
    if let Some(token) = first_template_token(trimmed) {
        return Err(ArtifactPathRejection::UnexpandedTemplate { token });
    }
    Ok(trimmed.to_string())
}

/// Does this segment read as a sentence rather than a name?
///
/// Two signals, either sufficient. Word count alone catches the observed litter
/// (`A gap-audit report or equivalent reducer evidence ...`). The punctuation
/// clause catches the short, emphatic criterion — `Fail closed, always.` — which
/// a word-count rule alone would let through.
fn segment_is_prose(segment: &str) -> bool {
    let words = segment.split_whitespace().count();
    if words >= PROSE_WORD_COUNT {
        return true;
    }
    if words < 2 {
        return false;
    }
    segment.contains(SENTENCE_PUNCTUATION) || segment.contains(". ") || segment.ends_with('.')
}

/// The first `${...}` or `<...>` span, if any.
pub fn first_template_token(value: &str) -> Option<String> {
    template_tokens(value).into_iter().next()
}

/// Every `${...}` and `<...>` span in `value`, in the order written.
///
/// Both shapes, because both were observed: `<dataset-id>` from deliverable
/// contracts and `${DATASET_ID}` from shell-templated ones. A path is not
/// checkable while either survives in it.
pub fn template_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let (open_len, close) = match bytes[index] {
            b'$' if bytes.get(index + 1) == Some(&b'{') => (2, b'}'),
            b'<' => (1, b'>'),
            _ => {
                index += 1;
                continue;
            }
        };
        let Some(offset) = value[index + open_len..].find(close as char) else {
            index += open_len;
            continue;
        };
        let end = index + open_len + offset + 1;
        tokens.push(value[index..end].to_string());
        index = end;
    }
    tokens
}

/// Expand every `${NAME}` in `raw` from `bind`, or refuse.
///
/// An unbound — or empty — variable is [`ArtifactPathRejection::
/// UnboundTemplateVariable`], never the empty string. Substituting empty is what
/// turns `${PROJECT_ROOT}/.archon/x.json` into `/.archon/x.json` or, worse,
/// silently into a relative path resolved against whatever the process happens
/// to be sitting in. Expand or refuse; there is no third option.
pub fn expand_artifact_path_template<F>(raw: &str, bind: F) -> Result<String, ArtifactPathRejection>
where
    F: Fn(&str) -> Option<String>,
{
    let mut expanded = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find("${") {
        expanded.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find('}') else {
            return Err(ArtifactPathRejection::MalformedTemplate);
        };
        let name = &after[..close];
        let bound = bind(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ArtifactPathRejection::UnboundTemplateVariable {
                name: name.to_string(),
            })?;
        expanded.push_str(&bound);
        rest = &after[close + 1..];
    }
    expanded.push_str(rest);
    Ok(expanded)
}

/// Expand `${PROJECT_ROOT}` and nothing else.
///
/// The only variable the host can honestly bind: it knows the project root and
/// it does not know `${DATASET_ID}`. Anything else is refused by name rather
/// than guessed at.
pub fn expand_project_root_template(
    raw: &str,
    project_root: Option<&str>,
) -> Result<String, ArtifactPathRejection> {
    expand_artifact_path_template(raw, |name| match name {
        "PROJECT_ROOT" => project_root.map(str::to_string),
        _ => None,
    })
}

/// Why `path` is not artifact evidence, or `None` when it is.
///
/// A declared artifact is a regular, non-empty file. Not a directory — the
/// litter this module exists to stop is directories, and `Path::exists` says
/// yes to every one of them. Not an empty file either: nothing was written, so
/// nothing was evidenced.
pub fn artifact_file_defect(path: &Path) -> Option<&'static str> {
    match std::fs::metadata(path) {
        Err(_) => Some("does not exist"),
        Ok(metadata) if metadata.is_dir() => Some("is a directory, not the declared file"),
        Ok(metadata) if !metadata.is_file() => Some("is not a regular file"),
        Ok(metadata) if metadata.len() == 0 => Some("is an empty file"),
        Ok(_) => None,
    }
}

/// A declared artifact is satisfied only by a regular, non-empty file.
pub fn artifact_file_is_evidence(path: &Path) -> bool {
    artifact_file_defect(path).is_none()
}

/// Project-root entries that look like a prose value used as a path.
///
/// The detection half of the fix. The host cannot stop an agent-issued
/// `mkdir -p` with a sentence in it — that command is composed inside the agent
/// and never reaches a host-side writer — so the honest position is to notice
/// the result and say so, rather than let a run finish over the top of it.
/// Immediate children only: the litter is created in the project root, and
/// walking further would report every deep path in the repository.
pub fn project_root_path_litter(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return Vec::new();
    };
    let mut litter: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| entry_name_is_litter(name))
        .collect();
    litter.sort();
    litter.dedup();
    litter
}

/// Is this directory-entry name a prose value that became a path?
pub fn entry_name_is_litter(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    name.chars().count() > LITTER_NAME_CHARS
        || segment_is_prose(name)
        || !template_tokens(name).is_empty()
}

#[cfg(test)]
#[path = "artifact_path_guard_tests.rs"]
mod tests;
