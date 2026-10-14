//! Reading a critic's reply before it is a verdict: what an omitted key
//! means, and how far a refused reply parsed. Split from `fidelity_audit.rs`
//! so that file stays under its size budget.

use serde::Deserialize;
use serde_path_to_error::Segment;

use super::{DOCUMENT_KEYS, FidelityVerdict, VERDICT_KEYS};

/// One verdict exactly as the critic wrote it. `weakest_task_id`,
/// `quoted_task_text`, `supporting_task_id` and `supporting_text` are
/// optional only so that their absence is seen, never so that it is filled
/// in: [`checked_verdict`] refuses a false verdict without the first two and
/// a true verdict without the last two.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyVerdict {
    pub(super) obligation_id: String,
    pub(super) necessarily_true: bool,
    pub(super) weakest_task_id: Option<String>,
    pub(super) reason: String,
    pub(super) quoted_task_text: Option<String>,
    #[serde(default)]
    pub(super) weakening_quotes: Vec<String>,
    pub(super) supporting_task_id: Option<String>,
    pub(super) supporting_text: Option<String>,
}

/// The kind of a refused reply's first error: a closed set. A serde error
/// or one of this module's own refusals maps to one kind; anything unmapped
/// is [`RefusalKind::Other`], one kind. A key is always one of
/// [`VERDICT_KEYS`] or [`DOCUMENT_KEYS`], so with eight keys there are
/// 1 + 8 + 8 + 8 + 1 + 1 + 1 + 8 + 8 + 1 + 1 = 46 kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefusalKind {
    /// A key that is not allowed; which name it was is ignored.
    UnknownField,
    MissingField(&'static str),
    InvalidType(&'static str),
    InvalidValue(&'static str),
    /// The document ended early.
    Truncated,
    Syntax,
    /// Verdict ids repeat, or do not match the cluster.
    WrongObligationIds,
    /// A key whose meaning requires a value is missing or blank.
    BlankRequired(&'static str),
    /// A quote under this key is not verbatim in the task it names.
    NotVerbatim(&'static str),
    /// A true verdict's `supporting_text` is verbatim but no anchor: too
    /// short, found more than once, or not in the task's prose (#434).
    WeakAnchor,
    Other,
}

/// A refused reply: the error the critic is shown, and its class — which
/// verdict (`None` for the document as a whole) and what kind of error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FidelityRefusal {
    pub message: String,
    pub verdict: Option<usize>,
    pub kind: RefusalKind,
}

impl FidelityRefusal {
    /// The (verdict, kind) pair the critic's progress is judged by.
    pub fn class(&self) -> (Option<usize>, RefusalKind) {
        (self.verdict, self.kind)
    }
}

impl std::fmt::Display for FidelityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `key` as an allowed key, if it is one.
pub(super) fn allowed(key: &str) -> Option<&'static str> {
    VERDICT_KEYS
        .iter()
        .chain(DOCUMENT_KEYS.iter())
        .copied()
        .find(|allowed| *allowed == key)
}

/// The class of a shape error. The verdict is the array index in the path
/// (`verdicts[i]`) when it is inside the cluster's size; any other place,
/// an index past the cluster included, is the document's, so a verdict
/// index is always below the cluster size.
pub(super) fn shape_class(
    error: &serde_path_to_error::Error<serde_json::Error>,
    cluster: usize,
) -> (Option<usize>, RefusalKind) {
    let segments: Vec<&Segment> = error.path().iter().collect();
    let verdict = match segments.as_slice() {
        [Segment::Map { key }, Segment::Seq { index }, ..]
            if key == "verdicts" && *index < cluster =>
        {
            Some(*index)
        }
        _ => None,
    };
    let last_key = segments.iter().rev().find_map(|segment| match segment {
        Segment::Map { key } => Some(key.as_str()),
        _ => None,
    });
    (verdict, json_kind(error.inner(), last_key))
}

/// The kind of a serde_json error; `key` is the last key on its path.
pub(super) fn json_kind(error: &serde_json::Error, key: Option<&str>) -> RefusalKind {
    use serde_json::error::Category;
    match error.classify() {
        Category::Eof => RefusalKind::Truncated,
        Category::Syntax => RefusalKind::Syntax,
        Category::Io => RefusalKind::Other,
        Category::Data => {
            let message = error.to_string();
            let keyed = |kind: fn(&'static str) -> RefusalKind| {
                key.and_then(allowed).map_or(RefusalKind::Other, kind)
            };
            if message.starts_with("unknown field `") {
                RefusalKind::UnknownField
            } else if let Some(rest) = message.strip_prefix("missing field `") {
                rest.split('`')
                    .next()
                    .and_then(allowed)
                    .map_or(RefusalKind::Other, RefusalKind::MissingField)
            } else if message.starts_with("invalid type:") {
                keyed(RefusalKind::InvalidType)
            } else if message.starts_with("invalid value:") || message.starts_with("invalid length")
            {
                keyed(RefusalKind::InvalidValue)
            } else {
                RefusalKind::Other
            }
        }
    }
}

/// Decide what an omitted quote key means, by the verdict's meaning —
/// never by a silent default.
///
/// A false verdict says a task's allowance lets the obligation stay false;
/// it is checkable only with the task it names and the words it quotes, so
/// `weakest_task_id` or `quoted_task_text` missing or empty is refused, and
/// the error names the key: the critic is shown this error and can supply
/// it. A true verdict says a task's text obliges the result (#434); it is
/// checkable, and reusable, only with the task it names and the words that
/// oblige it, so `supporting_task_id` or `supporting_text` missing or empty
/// is refused the same way. The keys of the other meaning have no value to
/// carry, so the empty string is their correct value and leaving them out
/// says the same thing; it is accepted, and logged at debug level.
pub(super) fn checked_verdict(
    reply: ReplyVerdict,
) -> Result<FidelityVerdict, (RefusalKind, String)> {
    let id = &reply.obligation_id;
    let given = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    let blank = |key: &'static str, what: &str| {
        Err((
            RefusalKind::BlankRequired(key),
            format!("{what}: `{key}` is missing or empty"),
        ))
    };
    if reply.necessarily_true {
        if !given(&reply.supporting_task_id) {
            return blank(
                "supporting_task_id",
                &format!(
                    "true verdict for {id} names no supporting task; a true verdict names the listed task whose text obliges the result"
                ),
            );
        }
        if !given(&reply.supporting_text) {
            return blank(
                "supporting_text",
                &format!(
                    "true verdict for {id} quotes nothing in support; a true verdict copies verbatim the words of its supporting task that oblige the result"
                ),
            );
        }
        if reply.weakest_task_id.is_none() || reply.quoted_task_text.is_none() {
            tracing::debug!(
                obligation_id = %id,
                "true fidelity verdict omits weakest_task_id or quoted_task_text; a true verdict has none, so it is empty"
            );
        }
    } else if !given(&reply.weakest_task_id) {
        return blank(
            "weakest_task_id",
            &format!(
                "false verdict for {id} names no weakest task; a false verdict names the listed task whose allowance grants the loophole"
            ),
        );
    } else if !given(&reply.quoted_task_text) {
        return blank(
            "quoted_task_text",
            &format!(
                "false verdict for {id} quotes nothing from the task; a false verdict copies the loophole verbatim from its weakest task"
            ),
        );
    } else if reply.supporting_task_id.is_none() || reply.supporting_text.is_none() {
        tracing::debug!(
            obligation_id = %id,
            "false fidelity verdict omits supporting_task_id or supporting_text; a false verdict has none, so it is empty"
        );
    }
    Ok(FidelityVerdict {
        obligation_id: reply.obligation_id,
        necessarily_true: reply.necessarily_true,
        weakest_task_id: reply.weakest_task_id.unwrap_or_default(),
        reason: reply.reason,
        quoted_task_text: reply.quoted_task_text.unwrap_or_default(),
        weakening_quotes: reply.weakening_quotes,
        supporting_task_id: reply.supporting_task_id.unwrap_or_default(),
        supporting_text: reply.supporting_text.unwrap_or_default(),
    })
}
