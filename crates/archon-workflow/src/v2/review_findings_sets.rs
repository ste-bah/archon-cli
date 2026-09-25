//! Reading the host's attached finding set back, and comparing finding sets
//! as multisets; split out of `review_findings.rs` to hold the 500-line
//! ceiling.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{HOST_REVIEW_FINDINGS_KEY, finding_key};

/// Count of each finding by key.
pub fn multiset(findings: &[Value]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for finding in findings {
        *counts.entry(finding_key(finding)).or_default() += 1;
    }
    counts
}

/// Findings in `have` that `want` lacks (by key, honouring multiplicity).
pub fn multiset_difference(have: &[Value], want: &[Value]) -> Vec<Value> {
    let mut remaining = multiset(want);
    have.iter()
        .filter(|finding| {
            let key = finding_key(finding);
            match remaining.get_mut(&key) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            }
        })
        .cloned()
        .collect()
}

/// The host-attached finding set on a result's `data`, if any.
pub fn attached(data: &Value) -> Option<Vec<Value>> {
    data.get(HOST_REVIEW_FINDINGS_KEY)?
        .get("findings")?
        .as_array()
        .cloned()
}

/// The source maps a reduce named whose results were missing when it ran.
pub fn attached_missing_sources(data: &Value) -> Vec<String> {
    data.get(HOST_REVIEW_FINDINGS_KEY)
        .and_then(|value| value.get("missing_source_map_call_ids"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
