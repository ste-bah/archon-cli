//! A check that names a live root by its absolute path (Issue 366).
//!
//! Every check runs from its working directory in a hermetic copy of the
//! trees (the probe's copy or the scratch observation), never in the live
//! repository or project: a command that names a live root by its absolute
//! path would still read or change the live tree there. The freeze probe
//! refuses such a check unrun, and the acceptance author step refuses it in
//! the same words, so the author repairs it in the same call. Both use this
//! one rule.
//!
//! A root is named when its text occurs in the command and the next
//! character does not continue a path component: the root itself, a path
//! under it, or the root followed by shell syntax (a quote, a glob, a
//! variable). A longer sibling name that only starts with the root's text
//! (`<root>-other`, `<root>.bak`) is a different path and is not the root.

use std::path::{Path, PathBuf};

/// The forms of `roots` check text could name: each as given and its
/// canonical form, without trailing separators. The filesystem root and
/// empty text are never a live root to match.
pub(crate) fn root_forms<'a>(roots: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    let mut forms = Vec::new();
    for root in roots {
        forms.push(root.to_path_buf());
        if let Ok(canonical) = root.canonicalize().map(archon_shell::paths::plain) {
            forms.push(canonical);
        }
    }
    let mut forms: Vec<PathBuf> = (forms.into_iter())
        .map(|form| {
            let text = form.to_string_lossy();
            PathBuf::from(text.trim_end_matches(['/', '\\']))
        })
        .filter(|form| !form.as_os_str().is_empty())
        .collect();
    forms.sort();
    forms.dedup();
    forms
}

/// Whether `c` continues a path component (so the root's text is only the
/// start of a longer name).
fn continues_name(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '~' | '@' | '%')
}

/// The first of `forms` (from [`root_forms`]) that `text` names.
pub(crate) fn named_root<'a>(text: &str, forms: &'a [PathBuf]) -> Option<&'a PathBuf> {
    forms.iter().find(|form| {
        let root = form.to_string_lossy();
        !root.is_empty()
            && text.match_indices(root.as_ref()).any(|(at, _)| {
                (text[at + root.len()..].chars().next()).is_none_or(|next| !continues_name(next))
            })
    })
}

/// The finding for check `id` that names live root `root`: the one text
/// the freeze and the author step both give.
pub(crate) fn live_root_finding(id: &str, root: &Path) -> String {
    format!(
        "check '{id}': it names the live root {} by its absolute path, so no hermetic copy can keep it off the live tree and the host never runs it; name every path relative to the check's working directory",
        root.display()
    )
}

/// The text the host executes for an authored entry's `check` value: a
/// command's text, or a floor's non-blank verifier (as `executed_text`).
pub(crate) fn executed_check_text(check: &serde_json::Value) -> Option<&str> {
    match check.get("kind")?.as_str()? {
        "command" => check.get("command")?.as_str(),
        "floor" => (check
            .get("contract")?
            .get("typed_verifier_command")?
            .as_str())
        .filter(|command| !command.trim().is_empty()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_live_root_tests.rs"]
mod tests;
