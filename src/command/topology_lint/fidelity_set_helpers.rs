use std::collections::BTreeMap;

/// The canonical ids of the other tasks in the set that `text` names — the
/// one hop of the ownership chain. An id counts only whole: a text naming
/// `<task-id>1` does not name `<task-id>`, and an id outside the set is
/// never pulled in.
pub(super) fn named_set_tasks(
    own_id: &str,
    text: &str,
    set: &BTreeMap<String, (String, std::path::PathBuf)>,
) -> Vec<String> {
    set.keys()
        .filter(|id| id.as_str() != own_id && mentions_whole(text, id))
        .cloned()
        .collect()
}

pub(super) fn mentions_whole(text: &str, id: &str) -> bool {
    let bounded = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
    text.match_indices(id).any(|(start, _)| {
        bounded(text[..start].chars().next_back())
            && bounded(text[start + id.len()..].chars().next())
    })
}
