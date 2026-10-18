use super::Claim;

/// Verbs directly before the path that assert it is present, and phrases that
/// assert it is absent.
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

pub(super) fn claim_before(before: &str) -> Option<Claim> {
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
