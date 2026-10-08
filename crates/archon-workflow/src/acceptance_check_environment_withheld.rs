//! Output is never evidence for changing a verifier verdict (Issue 349).
//! A failed child may mention host variables it was actually denied. Report
//! their names separately so the operator can consider forwarding them. Even
//! expectations, quoted JSON and framework prefixes use the same literal,
//! case-sensitive rule: a name counts only as a whole identifier (no
//! [A-Za-z0-9_] byte before or after it). Shell-maintained names are skipped.
//! Never parse prose or disclose host values.

use super::lookup;
use std::collections::{BTreeMap, BTreeSet};

/// The host variables a check given `environment` does not get.
pub fn withheld(
    host: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    withheld_names(host.keys().map(String::as_str), environment)
}

/// The one candidate filter every capture path uses: a host name the check
/// does not get, other than a shell-maintained name.
pub(super) fn withheld_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    environment: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    (names.into_iter())
        .filter(|name| lookup(environment, name).is_none())
        .filter(|name| !process_maintained_name(name))
        .map(str::to_owned)
        .collect()
}

/// Call only for a failed check. This note does not establish a cause and
/// must never replace an exit status, failure, refusal, or success verdict.
pub fn withheld_note(outputs: &[&[u8]], withheld: &BTreeSet<String>) -> Option<String> {
    withheld_note_with_remedy(
        outputs,
        withheld,
        "Ask the operator to allowlist needed names in [workflow.acceptance_execution] environment_allowlist",
    )
}

pub(super) fn withheld_note_with_remedy(
    outputs: &[&[u8]],
    withheld: &BTreeSet<String>,
    remedy: &str,
) -> Option<String> {
    let named: Vec<&str> = withheld
        .iter()
        .filter(|name| {
            outputs
                .iter()
                .any(|output| contains_identifier(output, name.as_bytes()))
        })
        .map(String::as_str)
        .collect();
    (!named.is_empty()).then(|| format!(
        "Note: output mentions withheld variable(s) {}. This note does not establish the cause of failure; the verifier's real result is retained. {remedy}.", named.join(", ")
    ))
}

fn process_maintained_name(name: &str) -> bool {
    name.len() <= 1 || matches!(name, "PWD" | "OLDPWD" | "SHLVL")
}

fn contains_identifier(output: &[u8], name: &[u8]) -> bool {
    if name.is_empty() || name.len() > output.len() {
        return false;
    }
    output
        .windows(name.len())
        .enumerate()
        .any(|(index, found)| {
            if found != name {
                return false;
            }
            let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
            let before = index.checked_sub(1).and_then(|i| output.get(i));
            let after = output.get(index + name.len());
            !before.is_some_and(|byte| identifier(*byte))
                && !after.is_some_and(|byte| identifier(*byte))
        })
}
