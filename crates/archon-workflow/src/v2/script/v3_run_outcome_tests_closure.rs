//! Batch O test support: the per-finding facts a live host would record for
//! the legacy-shaped call lists these tests build. Every remediation call of
//! a key names the ids of the findings that key covers (and the finding
//! standing for a blocked task); an accepted verifier AGENT closes each with
//! evidence, any other verifier leaves each open. So a test about unit
//! backing still decides by its calls, and closure has tests of its own.

use std::collections::{BTreeMap, BTreeSet};

use super::super::remediation_dispositions::DispositionFact;
use super::*;
use crate::v2::review_finding_ids::finding_id_of;
use crate::v2::review_findings::task_ids_of;

pub(super) fn stamped(calls: &[AuthoredCallFact], accounting: &str) -> Vec<AuthoredCallFact> {
    let Ok(accounting) = serde_json::from_str::<serde_json::Value>(accounting) else {
        return calls.to_vec();
    };
    let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for field in ["adversarial_findings", "uncovered_requirements"] {
        for finding in array(accounting.get(field)) {
            let tasks = task_ids_of(finding);
            let id = finding_id_of(finding);
            if finding.get("attributable_to_task") == Some(&serde_json::Value::Bool(false)) {
                by_key.entry(cross_key(tasks)).or_default().push(id);
            } else {
                for task in tasks {
                    by_key.entry(task).or_default().push(id.clone());
                }
            }
        }
    }
    let mut plan = AuthoredCallFact {
        id: "remediation-plan-1".into(),
        role: AuthoredCallRole::Other,
        status: Some(WorkflowV2Status::Accepted),
        transport: false,
        tasks: BTreeMap::new(),
        record_path: None,
        agent_attributed: false,
        landed_nothing: false,
        host_reverify: false,
        remediation: Default::default(),
    };
    for entry in array(accounting.get("blocked")) {
        let task = task_id(entry).unwrap_or_default().to_string();
        let id = format!("blocked:{task}");
        by_key.entry(task.clone()).or_default().push(id.clone());
        plan.remediation
            .planned_blocked
            .insert(task, BTreeSet::from([id]));
    }
    let mut out = vec![plan];
    for call in calls {
        let mut call = call.clone();
        // A call a test stamped itself is left as it is.
        if !call.remediation.finding_ids.is_empty() {
            out.push(call);
            continue;
        }
        let (task, verify) = match &call.role {
            AuthoredCallRole::RemediationFix { task, .. } => (task.clone(), None),
            AuthoredCallRole::RemediationVerify { task, agent, .. } => (task.clone(), Some(*agent)),
            _ => {
                out.push(call);
                continue;
            }
        };
        let ids: Vec<String> = by_key
            .iter()
            .filter(|(key, _)| parts(key) == parts(&task))
            .flat_map(|(_, ids)| ids.clone())
            .collect();
        call.remediation.finding_ids = ids.clone();
        if verify == Some(true) {
            let closes = call.status.is_some_and(is_reusable_status)
                && call
                    .tasks
                    .values()
                    .all(|outcome| is_reusable_status(outcome.status));
            call.remediation.dispositions = ids
                .into_iter()
                .map(|id| {
                    let said = DispositionFact {
                        disposition: if closes { "resolved" } else { "open" }.into(),
                        evidence: true,
                        said: "scripted".into(),
                        ..Default::default()
                    };
                    (id, said)
                })
                .collect();
        }
        out.push(call);
    }
    out
}

/// A key's tasks, case-folded and sorted: `cross:A+B` and `cross:b+a` are one.
fn parts(key: &str) -> Vec<String> {
    let bare = key
        .trim()
        .strip_prefix(CROSS_TASK_KEY_PREFIX)
        .unwrap_or(key.trim());
    let mut parts: Vec<String> = bare
        .split('+')
        .map(|p| p.trim().to_ascii_uppercase())
        .collect();
    parts.sort();
    parts
}
