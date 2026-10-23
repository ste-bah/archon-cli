//! Which checks a launch repair left pending, and whose each one is: a
//! check pending its author (the judge refuted every candidate, or its own
//! code crashed) goes to the fixed script's acceptance author loop; a check
//! the host could not prove (no pre-implementation tree, a first stall, a
//! site fault) is the host's and is never routed to an author (R7).

use std::collections::BTreeMap;
#[cfg(test)]
use std::path::Path;

/// The pending checks of a repair event's `detail`, split by owner.
pub(super) struct Pending {
    /// Pending their author, with each one's latest finding.
    pub(super) authored: BTreeMap<String, String>,
    /// Pending the host, with why it could not prove each one.
    pub(super) host: BTreeMap<String, String>,
}

impl Pending {
    pub(super) fn of(detail: &serde_json::Value) -> Self {
        let ids = |field: &str| -> Vec<String> {
            (detail.get(field).and_then(serde_json::Value::as_array))
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        };
        let reason = |id: &String| {
            (detail.get("unproven").and_then(|unproven| unproven.get(id)))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let with_reason = |id: String| {
            let why = reason(&id);
            (id, why)
        };
        let authored: BTreeMap<_, _> = (ids("authored_pending_ids").into_iter())
            .map(with_reason)
            .collect();
        let host = (ids("pending_ids").into_iter())
            .filter(|id| !authored.contains_key(id))
            .map(with_reason)
            .collect();
        Self { authored, host }
    }

    /// `ids` as `id: reason | ...`, for a log line.
    pub(super) fn reasons(ids: &BTreeMap<String, String>) -> String {
        (ids.iter())
            .map(|(id, reason)| format!("{id}: {reason:?}"))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// A test hook run after the repair published, only for the task root it
/// was set for: tests run in parallel, and another launch must never take it.
#[cfg(test)]
static AFTER_REAUTHOR_PUBLISHED_HOOK: std::sync::Mutex<Option<(std::path::PathBuf, fn(&Path))>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_after_reauthor_published_hook(task_root: &Path, hook: fn(&Path)) {
    *AFTER_REAUTHOR_PUBLISHED_HOOK.lock().unwrap() = Some((task_root.to_path_buf(), hook));
}

#[cfg(test)]
pub(super) fn take_after_reauthor_published_hook(task_root: &Path) -> Option<fn(&Path)> {
    let same = |a: &Path, b: &Path| {
        a == b || (a.canonicalize().ok()).is_some_and(|a| b.canonicalize().ok() == Some(a))
    };
    let mut slot = AFTER_REAUTHOR_PUBLISHED_HOOK.lock().unwrap();
    match slot.as_ref() {
        Some((root, _)) if same(root, task_root) => slot.take().map(|(_, hook)| hook),
        _ => None,
    }
}
