//! The fields the fixed script's author step owns on an acceptance entry
//! (`setHostOwnedFields`, Issue 357), set on what a phase seed reads so it is
//! compared and validated as the loop kept it.
use std::collections::BTreeMap;

use serde_json::Value;

/// The criteria IDs and requirement text the current script receives from the PRD.
pub(super) struct HostOwned<'a> {
    criteria: &'a BTreeMap<String, String>,
    requirement_texts: &'a BTreeMap<String, String>,
}

impl<'a> HostOwned<'a> {
    pub(super) fn new(
        criteria: &'a BTreeMap<String, String>,
        requirement_texts: &'a BTreeMap<String, String>,
    ) -> Self {
        Self {
            criteria,
            requirement_texts,
        }
    }

    /// What the script's `setHostOwnedFields` sets before it keeps an entry
    /// (Issue 357): the criterion is the host's. Supplementary IDs take their
    /// current requirement text and cover exactly that requirement with no gap.
    pub(super) fn stamp(&self, mut entry: Value, id: &str) -> Value {
        let supplementary = id.strip_prefix("SUP-").filter(|requirement| {
            !self.criteria.contains_key(id) && self.requirement_texts.contains_key(*requirement)
        });
        let criterion = self.criteria.get(id).or_else(|| {
            supplementary.and_then(|requirement| self.requirement_texts.get(requirement))
        });
        if let Some(criterion) = criterion {
            entry["criterion"] = Value::from(criterion.as_str());
        }
        let Some(requirement) = supplementary else {
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
