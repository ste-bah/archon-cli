//! The fields the fixed script's author step owns on an acceptance entry
//! (`setHostOwnedFields`, Issue 357), set on what a phase seed reads so it is
//! compared and validated as the loop kept it.
use std::collections::BTreeMap;

use serde_json::Value;

use super::{ACCEPTANCE_GATE, Gate};

/// The criterion criteria ids map to, and the requirement text of every
/// supplementary check a gate said is owed (the script's `acceptanceRepairIds`).
pub(super) struct HostOwned<'a> {
    criteria: &'a BTreeMap<String, String>,
    owed: BTreeMap<String, String>,
}

/// `check 'SUP-<req>': PRD requirement <req> is covered by no acceptance
/// check;<anything but ':'>: <text>`, as the script's `supFinding` reads it.
fn owed_text(finding: &str) -> Option<(&str, &str)> {
    let rest = finding.strip_prefix("check '")?;
    let (id, rest) = rest.split_once('\'')?;
    let requirement = id.strip_prefix("SUP-")?;
    let valid = requirement.strip_prefix("REQ-").is_some_and(|tail| {
        !tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    let rest = rest.strip_prefix(&format!(
        ": PRD requirement {requirement} is covered by no acceptance check;"
    ))?;
    let (_, text) = rest.split_once(':')?;
    valid.then_some((id, text.trim()))
}

impl<'a> HostOwned<'a> {
    pub(super) fn new(criteria: &'a BTreeMap<String, String>, gates: &[Gate<'_>]) -> Self {
        let mut owed = BTreeMap::new();
        for (.., gate) in gates
            .iter()
            .filter(|(_, command, ..)| *command == ACCEPTANCE_GATE)
        {
            for finding in &gate.findings {
                if finding["remediation_scope"] != "candidate_artifact" {
                    continue;
                }
                if let Some((id, text)) = finding["text"].as_str().and_then(owed_text) {
                    owed.insert(id.to_string(), text.to_string());
                }
            }
        }
        Self { criteria, owed }
    }

    /// What the script's `setHostOwnedFields` sets before it keeps an entry
    /// (Issue 357): the criterion is the host's, and a supplementary check
    /// covers exactly the requirement it is owed for and permits no gap.
    pub(super) fn stamp(&self, mut entry: Value, id: &str) -> Value {
        if let Some(criterion) = self.criteria.get(id).or_else(|| self.owed.get(id)) {
            entry["criterion"] = Value::from(criterion.as_str());
        }
        let Some(requirement) = id
            .strip_prefix("SUP-")
            .filter(|_| !self.criteria.contains_key(id))
        else {
            return entry;
        };
        let mut covers = vec![Value::from(requirement)];
        if let Some(listed) = entry["covers"].as_array() {
            covers.extend(
                listed
                    .iter()
                    .filter(|c| c.is_string() && c.as_str() != Some(requirement))
                    .cloned(),
            );
        }
        entry["covers"] = Value::Array(covers);
        entry["gap_permitted"] = Value::Bool(false);
        entry
    }
}
