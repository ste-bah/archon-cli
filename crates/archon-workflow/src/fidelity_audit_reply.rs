//! Reading a critic's reply before it is a verdict: what an omitted key
//! means, and how far a refused reply parsed. Split from `fidelity_audit.rs`
//! so that file stays under its size budget.

use serde::Deserialize;
use serde_path_to_error::Segment;

use super::{DOCUMENT_KEYS, FidelityVerdict, VERDICT_KEYS};

/// One verdict exactly as the critic wrote it. `weakest_task_id` and
/// `quoted_task_text` are optional only so that their absence is seen, never
/// so that it is filled in: [`checked_verdict`] refuses a false verdict
/// without them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyVerdict {
    pub(super) obligation_id: String,
    pub(super) necessarily_true: bool,
    pub(super) weakest_task_id: Option<String>,
    pub(super) reason: String,
    pub(super) quoted_task_text: Option<String>,
}

/// The kind of a refused reply's first error: a closed set. A serde error
/// or one of this module's own refusals maps to one kind; anything unmapped
/// is [`RefusalKind::Other`], one kind. A key is always one of
/// [`VERDICT_KEYS`] or [`DOCUMENT_KEYS`], so with six keys there are
/// 1 + 6 + 6 + 6 + 1 + 1 + 1 + 6 + 1 = 29 kinds.
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

/// Decide what an omitted `weakest_task_id` or `quoted_task_text` means, by
/// the verdict's meaning — never by a silent default.
///
/// A false verdict says a task's allowance lets the obligation stay false;
/// it is checkable only with the task it names and the words it quotes, so
/// either key missing or empty is refused, and the error names the key: the
/// critic is shown this error and can supply it. A true verdict has no
/// loophole to name or quote, so the empty string is its correct value and
/// leaving the key out says the same thing; it is accepted, and the omission
/// is logged at debug level so it stays visible.
pub(super) fn checked_verdict(
    reply: ReplyVerdict,
) -> Result<FidelityVerdict, (RefusalKind, String)> {
    let id = &reply.obligation_id;
    let given = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    if reply.necessarily_true {
        if reply.weakest_task_id.is_none() || reply.quoted_task_text.is_none() {
            tracing::debug!(
                obligation_id = %id,
                "true fidelity verdict omits weakest_task_id or quoted_task_text; a true verdict has none, so it is empty"
            );
        }
    } else if !given(&reply.weakest_task_id) {
        return Err((
            RefusalKind::BlankRequired("weakest_task_id"),
            format!(
                "false verdict for {id} names no weakest task: `weakest_task_id` is missing or empty; a false verdict names the listed task whose allowance grants the loophole"
            ),
        ));
    } else if !given(&reply.quoted_task_text) {
        return Err((
            RefusalKind::BlankRequired("quoted_task_text"),
            format!(
                "false verdict for {id} quotes nothing from the task: `quoted_task_text` is missing or empty; a false verdict copies the loophole verbatim from its weakest task"
            ),
        ));
    }
    Ok(FidelityVerdict {
        obligation_id: reply.obligation_id,
        necessarily_true: reply.necessarily_true,
        weakest_task_id: reply.weakest_task_id.unwrap_or_default(),
        reason: reply.reason,
        quoted_task_text: reply.quoted_task_text.unwrap_or_default(),
    })
}
