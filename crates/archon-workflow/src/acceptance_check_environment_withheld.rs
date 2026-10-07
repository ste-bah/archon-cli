//! Output is never evidence for changing a verifier verdict (Issue 349).
//! A failed child may mention host variables it was actually denied. Report
//! their names separately so the operator can consider forwarding them. Even
//! expectations, quoted JSON and framework prefixes use the same literal,
//! case-sensitive substring rule. Never parse prose or disclose host values.

use super::lookup;
use std::collections::{BTreeMap, BTreeSet};

/// The host variables a check given `environment` does not get.
pub fn withheld(
    host: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    (host.keys())
        .filter(|name| lookup(environment, name).is_none())
        .cloned()
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
                .any(|output| String::from_utf8_lossy(output).contains(name.as_str()))
        })
        .map(String::as_str)
        .collect();
    (!named.is_empty()).then(|| format!(
        "Note: output mentions withheld variable(s) {}. This note does not establish the cause of failure; the verifier's real result is retained. {remedy}.", named.join(", ")
    ))
}
