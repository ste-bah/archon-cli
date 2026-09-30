//! Batch O: every review finding closed by a verifier's own verdict on it.
//!
//! A finding is CLOSED only when the LATEST remediation verifier AGENT whose
//! contract names its id (`findingIds`) gave it a closing disposition
//! (`remediation_dispositions`: `resolved` or `invalid` with evidence, and a
//! failing mutation for a finding about a check), and that verifier judged a
//! fix of the same unit and round that named the id and ran before it. A fix
//! that landed nothing is judged only by the verifier the prelude asks about
//! it (`refutation` in its contract: does each finding still hold on the
//! tree as it is) or the host's own re-verification. Whether a
//! finding concerns a check is the host's reading of the finding itself,
//! whatever the script declared.
//!
//! Every other finding holds the run, at any severity and whoever it names:
//! one never judged, one left open, one a verifier closed without evidence.
//! A blocked task holds it too unless every finding the remediation plan
//! folded in for it is closed.

use std::collections::{BTreeMap, BTreeSet};

use super::keys::TaskKeys;
use super::*;
use crate::v2::review_finding_ids::finding_id_of;
use crate::v2::script::remediation_dispositions::is_check_finding;

/// The latest verdict on each finding id: closed, or open with why.
struct Verdicts(BTreeMap<String, (bool, String, String)>);

impl Verdicts {
    fn read(calls: &[AuthoredCallFact], keys: &TaskKeys<'_>, checks: &BTreeSet<String>) -> Self {
        let mut latest: BTreeMap<String, (bool, String, String)> = BTreeMap::new();
        for (at, call) in calls.iter().enumerate() {
            let AuthoredCallRole::RemediationVerify { task, round, agent } = &call.role else {
                continue;
            };
            let fact = &call.remediation;
            if !agent || fact.finding_ids.is_empty() {
                continue;
            }
            let key = keys.key(task);
            // The fix this verifier judged: the last of its unit and round
            // before it.
            let fix = calls[..at].iter().rev().find(|candidate| {
                matches!(&candidate.role, AuthoredCallRole::RemediationFix { task: t, round: r }
                    if keys.key(t) == key && r == round)
                    && candidate.remediation.unit == fact.unit
            });
            for id in &fact.finding_ids {
                let verdict = match fix {
                    None => (
                        false,
                        "its verifier judged no fix of its unit and round".to_string(),
                    ),
                    Some(fix) if !fix.remediation.finding_ids.contains(id) => (
                        false,
                        format!("the fix `{}` its verifier judged did not name it", fix.id),
                    ),
                    Some(fix) if fix.landed_nothing && !fact.refutation && !call.host_reverify => (
                        false,
                        format!(
                            "the fix `{}` landed nothing and `{}` judged no refutation",
                            fix.id, call.id
                        ),
                    ),
                    Some(_) => {
                        let said = fact.dispositions.get(id).cloned().unwrap_or_default();
                        let check = checks.contains(id) || fact.check_ids.contains(id);
                        let refutation = fact.refutation && !call.host_reverify;
                        if said.closes(check, refutation) {
                            (true, String::new())
                        } else {
                            (false, said.open_reason(check, refutation))
                        }
                    }
                };
                latest.insert(id.clone(), (verdict.0, verdict.1, call.id.clone()));
            }
        }
        Self(latest)
    }

    fn of(&self, id: &str) -> Option<&(bool, String, String)> {
        self.0.get(id)
    }
}

/// Hold the run on every finding (and blocked task) no verifier closed.
pub(super) fn check_finding_closure(
    accounting: &serde_json::Value,
    calls: &[AuthoredCallFact],
    keys: &TaskKeys<'_>,
    _discharged: &BTreeSet<String>,
    v: &mut Verdict,
) {
    // A residual round's "discharge" of a unit says nothing about which of
    // its findings it fixed: it never closes a finding (Batch O review).
    let findings: Vec<&serde_json::Value> = ["adversarial_findings", "uncovered_requirements"]
        .iter()
        .flat_map(|field| array(accounting.get(*field)))
        .filter(|finding| text(finding.get("review_outcome")) != UNREVIEWED_REVIEW_OUTCOME)
        .collect();
    let checks: BTreeSet<String> = findings
        .iter()
        .filter(|finding| is_check_finding(finding))
        .map(|finding| finding_id_of(finding))
        .collect();
    let verdicts = Verdicts::read(calls, keys, &checks);
    let mut seen = BTreeSet::new();
    for finding in findings {
        let id = finding_id_of(finding);
        if !seen.insert(id.clone()) {
            continue;
        }
        let label = super::findings::finding_label(finding);
        match verdicts.of(&id) {
            Some((true, _, _)) => {}
            Some((false, why, by)) => v.block(
                format!("finding {label} ({id}) is open after `{by}`: {}", clip(why)),
                false,
            ),
            None => v.block(
                format!("finding {label} ({id}) was judged by no remediation verifier"),
                false,
            ),
        }
    }
    // Every finding a remediation plan was handed is held to the same rule,
    // whether or not the accounting lists it.
    for call in calls {
        for id in &call.remediation.planned_ids {
            if !seen.insert(id.clone()) {
                continue;
            }
            match verdicts.of(id) {
                Some((true, _, _)) => {}
                Some((false, why, by)) => v.block(
                    format!("planned finding {id} is open after `{by}`: {}", clip(why)),
                    false,
                ),
                None => v.block(
                    format!("planned finding {id} was judged by no remediation verifier"),
                    false,
                ),
            }
        }
    }
    for entry in array(accounting.get("blocked")) {
        let task = keys.key(task_id(entry).unwrap_or("<unnamed>"));
        let planned: BTreeSet<&String> = calls
            .iter()
            .flat_map(|call| call.remediation.planned_blocked.iter())
            .filter(|(named, _)| keys.key(named) == task)
            .flat_map(|(_, ids)| ids)
            .collect();
        let open: Vec<&&String> = planned
            .iter()
            .filter(|id| !matches!(verdicts.of(id), Some((true, _, _))))
            .collect();
        if planned.is_empty() || !open.is_empty() {
            let reason = text(entry.get("reason"));
            v.block(
                format!("task {task} is blocked and review remediation closed no finding standing for it: {}", clip(reason)),
                is_transport_failure_text(reason),
            );
        } else {
            v.notes.push(format!(
                "blocked task {task} was finished by review remediation"
            ));
        }
    }
}
