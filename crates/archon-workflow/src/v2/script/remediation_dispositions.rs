//! Batch O: a remediation verifier's verdict on EACH finding it judged.
//!
//! One accepted fix and one accepted check used to resolve a whole group of
//! findings. Now a verifier reports, under [`FINDING_DISPOSITIONS_KEY`] (the
//! agent adapter lifts a top-level field of that name into `data`), one
//! entry per finding id its contract names (`findingIds`):
//!
//! `{finding_id, disposition: "resolved" | "invalid" | "open", evidence,
//!   mutation: {command, failed}}`
//!
//! A finding is CLOSED by that verifier only when its entry says `resolved`
//! or `invalid` with non-empty evidence and, for a finding about a test or
//! check ([`is_check_finding`], or an entry declaring `kind: "check"`), a
//! `mutation` whose `command` is named and which `failed` -- the check was
//! shown to fail on a mutated temporary copy. A verifier judging a fix that
//! landed nothing (`refutation` in its contract) judges whether each finding
//! still holds on the tree as it is, under the same rule. Everything else -- no entry,
//! `open`, missing evidence, an unreadable status -- leaves it open, whatever
//! the verifier's overall status says.
//!
//! The same reading is shown to the script on the verifier's view
//! ([`REMEDIATION_DISPOSITIONS_KEY`]) so it re-sends exactly the open ids;
//! the terminal rule reads it again from the records.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{WorkflowV2CallRecord, WorkflowV2Result, remediation_contract};

/// Key of the dispositions in a verifier result's `data`.
pub const FINDING_DISPOSITIONS_KEY: &str = "finding_dispositions";
/// Key of the host's reading on the verifier's view.
pub const REMEDIATION_DISPOSITIONS_KEY: &str = "remediation_dispositions";
/// Contract field: the finding ids a remediation call acts on.
pub const FINDING_IDS_KEY: &str = "findingIds";
/// Contract field: of those, the ones about a test or check.
pub const CHECK_FINDING_IDS_KEY: &str = "checkFindingIds";
/// Contract field on a verifier judging a fix that landed nothing.
pub const REFUTATION_KEY: &str = "refutation";

/// What one verifier said of one finding, as the host reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispositionFact {
    /// `resolved`, `invalid`, `open`, or empty when no entry named it.
    pub disposition: String,
    pub evidence: bool,
    /// The entry declared the finding a check (`kind: "check"`).
    pub declared_check: bool,
    /// A mutation proof: a named command that failed on the mutated copy.
    pub mutation_failed: bool,
    /// The entry's evidence text, whole, for the next round's prompt.
    pub said: String,
}

impl DispositionFact {
    /// Whether this verdict closes the finding (see the module doc).
    /// `refutation` marks a verifier judging a fix that landed nothing: it
    /// judges whether each finding still holds on the tree as it is (the
    /// run's other landings may have fixed it; the reviewer may have been
    /// wrong), under the same evidence rule.
    pub fn closes(&self, check: bool, _refutation: bool) -> bool {
        let allowed = matches!(self.disposition.as_str(), "resolved" | "invalid");
        let proven = !(check || self.declared_check) || self.mutation_failed;
        allowed && self.evidence && proven
    }

    /// Why it does not close, for the accounting and the next prompt.
    pub fn open_reason(&self, check: bool, _refutation: bool) -> String {
        if self.disposition.is_empty() {
            "the verifier gave no disposition for it".into()
        } else if !matches!(self.disposition.as_str(), "resolved" | "invalid") {
            format!("the verifier judged it `{}`", self.disposition)
        } else if !self.evidence {
            "the verifier gave no evidence for its disposition".into()
        } else if (check || self.declared_check) && !self.mutation_failed {
            "it concerns a check, and no mutation of a temporary copy was shown to make the check fail".into()
        } else {
            "open".into()
        }
    }
}

/// Whether a finding is about a test or check: its text names a test or
/// check together with how it is weak (it only checks existence, cannot
/// fail, passes trivially, pins nothing, asserts nothing).
pub fn is_check_finding(finding: &Value) -> bool {
    let text = super::remediation_plan::finding_text(finding).to_ascii_lowercase();
    // Whole words: "latest" and "attest" are no tests.
    let words: BTreeSet<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect();
    let subject = [
        "test",
        "tests",
        "assert",
        "asserts",
        "assertion",
        "check",
        "checks",
        "pin",
        "pins",
    ]
    .iter()
    .any(|word| words.contains(word));
    let weakness = [
        "only checks",
        "only asserts",
        "never asserts",
        "does not assert",
        "doesn't assert",
        "does not check",
        "never checks",
        "cannot fail",
        "can never fail",
        "always passes",
        "passes trivially",
        "trivially",
        "vacuous",
        "no test pins",
        "not pinned",
        "unpinned",
        "exists, never",
        "only that",
        "tautolog",
    ]
    .iter()
    .any(|word| text.contains(word));
    subject && weakness
}

/// Every entry in `record` naming `id`, from its own data and each branch
/// view's; the last one wins.
pub fn disposition_of(record: &WorkflowV2CallRecord, id: &str) -> DispositionFact {
    let data = &record.result.data;
    let views = data
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|view| view.pointer("/result/data"));
    let mut fact = DispositionFact::default();
    let commands = recorded_commands(record);
    for entries in std::iter::once(data)
        .chain(views)
        .filter_map(|data| data.get(FINDING_DISPOSITIONS_KEY).and_then(Value::as_array))
    {
        for entry in entries {
            let named = entry
                .get("finding_id")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str)
                .map(str::trim);
            if named != Some(id) {
                continue;
            }
            let text = |key: &str| entry.get(key).map(value_text).unwrap_or_default();
            let mutation = entry.get("mutation");
            // The mutation's command must be one the record shows run and
            // failing, not only the verifier's word for it.
            let ran_failing = |command: &str| {
                let command = command.trim();
                !command.is_empty()
                    && commands
                        .iter()
                        .any(|(ran, failed)| *failed && ran.contains(command))
            };
            fact = DispositionFact {
                disposition: text("disposition").trim().to_ascii_lowercase(),
                evidence: !text("evidence").trim().is_empty(),
                declared_check: text("kind").trim().eq_ignore_ascii_case("check"),
                mutation_failed: mutation.is_some_and(|m| {
                    m.get("failed") == Some(&Value::Bool(true))
                        && m.get("command")
                            .map(value_text)
                            .is_some_and(|c| ran_failing(&c))
                }),
                said: text("evidence"),
            };
            if fact.disposition.is_empty() {
                fact.disposition = text("status").trim().to_ascii_lowercase();
            }
        }
    }
    fact
}

/// Every command the record shows run, with whether it failed: the call's
/// own list and each branch view's.
fn recorded_commands(record: &WorkflowV2CallRecord) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = record
        .result
        .commands_run
        .iter()
        .map(|c| {
            let failed = c.status == crate::v2::WorkflowV2CommandStatus::Failed
                || c.exit_code.is_some_and(|code| code != 0);
            (c.command.clone(), failed)
        })
        .collect();
    let views = record
        .result
        .data
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|view| {
            view.pointer("/result/commands_run")
                .and_then(Value::as_array)
        });
    for list in views {
        for entry in list {
            let command = entry.get("command").map(value_text).unwrap_or_default();
            let failed = entry.get("status").and_then(Value::as_str) == Some("failed")
                || entry
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .is_some_and(|code| code != 0);
            out.push((command, failed));
        }
    }
    out
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Array(items) if items.is_empty() => String::new(),
        Value::Object(object) if object.is_empty() => String::new(),
        other => other.to_string(),
    }
}

/// The contract's list under `key`, trimmed, non-empty entries.
pub fn contract_ids(record_contract: Option<&Value>, key: &str) -> BTreeSet<String> {
    record_contract
        .and_then(|contract| contract.get(key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

/// The host's reading of a remediation verifier's record: each finding id
/// its contract names, closed or open with why.
pub fn reading(record: &WorkflowV2CallRecord) -> Option<BTreeMap<String, Result<String, String>>> {
    let contract = remediation_contract(&record.call)?;
    if contract.get("stage").and_then(Value::as_str) != Some("verify") {
        return None;
    }
    let ids = contract_ids(Some(contract), FINDING_IDS_KEY);
    if ids.is_empty() {
        return None;
    }
    let checks = contract_ids(Some(contract), CHECK_FINDING_IDS_KEY);
    let refutation = contract.get(REFUTATION_KEY) == Some(&Value::Bool(true));
    Some(
        ids.into_iter()
            .map(|id| {
                let fact = disposition_of(record, &id);
                let check = checks.contains(&id);
                let verdict = if fact.closes(check, refutation) {
                    Ok(fact.disposition.clone())
                } else {
                    Err(format!(
                        "{}{}",
                        fact.open_reason(check, refutation),
                        said(&fact)
                    ))
                };
                (id, verdict)
            })
            .collect(),
    )
}

fn said(fact: &DispositionFact) -> String {
    if fact.said.trim().is_empty() {
        String::new()
    } else {
        format!("; the verifier said: {}", fact.said)
    }
}

/// `result` with the host's reading, for a remediation verifier's view;
/// `None` for every other record. The key is the host's alone.
pub fn with_remediation_dispositions(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
) -> Option<WorkflowV2Result> {
    let carried = result.data.get(REMEDIATION_DISPOSITIONS_KEY).is_some();
    let read = reading(record);
    if read.is_none() && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(REMEDIATION_DISPOSITIONS_KEY);
    }
    if let Some(read) = read {
        let closed: Vec<&String> = read
            .iter()
            .filter(|(_, v)| v.is_ok())
            .map(|(id, _)| id)
            .collect();
        let open: BTreeMap<&String, &String> = read
            .iter()
            .filter_map(|(id, v)| v.as_ref().err().map(|why| (id, why)))
            .collect();
        viewed.data[REMEDIATION_DISPOSITIONS_KEY] =
            json!({"source": "host", "closed": closed, "open": open});
    }
    Some(viewed)
}

#[cfg(test)]
#[path = "remediation_dispositions_tests.rs"]
mod tests;
