//! Issue-117: the structured disposition a residual round's own verifier
//! gives each gap the round targets.
//!
//! A gap entry carries no status, and its prose is never read as one. The
//! round's verifier is asked instead for [`GAP_DISPOSITIONS_KEY`] in its
//! result's data: one `{gap_id, status}` per targeted gap, `status` exactly
//! `resolved` or `open`. The gate reads it from ONE record only -- the
//! round's own judge, the verifier that judged the round's fix -- and only
//! to decide whether a later gap naming the same file is the targeted gap
//! again or a new one. Any entry naming the gap as `open` holds it open; an
//! entry naming it with any other status is unreadable and changes nothing.

use serde_json::Value;

use super::super::WorkflowV2CallRecord;
use super::Residual;
use crate::v2::verification::UNOWNED_PATH_GAP_PREFIX;

/// Key of the dispositions in a verifier result's `data` (the agent adapter
/// lifts a top-level field of that name there).
pub const GAP_DISPOSITIONS_KEY: &str = "gap_dispositions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    Resolved,
    Open,
    /// An entry names the gap with a status that is neither word.
    Unreadable,
}

/// A gap id as compared: trimmed, lower-case, without the host's
/// unowned-path flag. Empty never names a gap.
pub(super) fn bare_id(id: &str) -> String {
    id.trim()
        .trim_start_matches(UNOWNED_PATH_GAP_PREFIX)
        .trim()
        .to_ascii_lowercase()
}

/// What `record` says of the gap `id`, from its own data and each branch
/// view's: `Open` when any entry naming it says `open`, else `Unreadable`
/// when any says something else, else `Resolved`; `None` when no entry
/// names it.
pub(super) fn disposition_of(record: &WorkflowV2CallRecord, id: &str) -> Option<Disposition> {
    let wanted = bare_id(id);
    if wanted.is_empty() {
        return None;
    }
    let data = &record.result.data;
    let views = data
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|view| view.pointer("/result/data"));
    let mut said: Vec<Disposition> = Vec::new();
    for entries in std::iter::once(data)
        .chain(views)
        .filter_map(|data| data.get(GAP_DISPOSITIONS_KEY).and_then(Value::as_array))
    {
        for entry in entries {
            let named = entry
                .get("gap_id")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str)
                .map(bare_id);
            if named.as_deref() != Some(wanted.as_str()) {
                continue;
            }
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status.trim().to_ascii_lowercase());
            said.push(match status.as_deref() {
                Some("resolved") => Disposition::Resolved,
                Some("open") => Disposition::Open,
                _ => Disposition::Unreadable,
            });
        }
    }
    [
        Disposition::Open,
        Disposition::Unreadable,
        Disposition::Resolved,
    ]
    .into_iter()
    .find(|kind| said.contains(kind))
}

/// Whether the gap (`id`, `description`) is the disposed-of `original`: the
/// same id, the same whole text, or the same opening words once every
/// path-like token (a file, with or without its line numbers) is dropped
/// from both -- a deep path alone fills the opening every gap on that file
/// shares. `None` when `original`'s own words are too few to compare that
/// way: the caller then compares as for any other gap.
pub(super) fn same_disposed_gap(original: &Residual, id: &str, description: &str) -> Option<bool> {
    if !bare_id(&original.id).is_empty() && bare_id(&original.id) == bare_id(id) {
        return Some(true);
    }
    let whole = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    if !whole(&original.description).is_empty()
        && whole(&original.description) == whole(description)
    {
        return Some(true);
    }
    let (mine, theirs) = (words(&original.description), words(description));
    (mine.len() >= 24).then(|| mine == theirs)
}

/// The opening words of `text` without its path-like tokens.
fn words(text: &str) -> String {
    let kept: Vec<&str> = text
        .split_whitespace()
        .filter(|token| !path_like(token))
        .collect();
    let letters: String = kept
        .join(" ")
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    letters
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect()
}

/// A token naming a path: it holds a separator, or it is `stem.ext`
/// (optionally `:line` or `:line-line`) with a short alphanumeric extension.
fn path_like(token: &str) -> bool {
    let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/' && c != '\\');
    if token.contains('/') || token.contains('\\') {
        return true;
    }
    let file = token
        .split_once(':')
        .filter(|(_, lines)| {
            !lines.is_empty() && lines.chars().all(|c| c.is_ascii_digit() || c == '-')
        })
        .map_or(token, |(file, _)| file);
    file.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=6).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

/// Whether the gap (`id`, `description`) is `original` by id or by its
/// opening words, whatever it names.
pub(in crate::v2::script) fn same_gap(original: &Residual, id: &str, description: &str) -> bool {
    let opening = |text: &str| {
        let words: String = text
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
            .collect();
        words
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(48)
            .collect::<String>()
    };
    let (mine, theirs) = (opening(&original.description), opening(description));
    (!bare_id(&original.id).is_empty() && bare_id(&original.id) == bare_id(id))
        || (mine.len() >= 24 && mine == theirs)
}
