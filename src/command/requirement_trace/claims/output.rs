//! Claim report and finding rendering.
use super::*;

fn short(index: &EvidenceIndex) -> String {
    index.base().chars().take(12).collect()
}

pub(super) fn findings(index: &EvidenceIndex, results: &[ClaimResult]) -> Vec<TracePolicyFinding> {
    let base = short(index);
    // (task, subject, reason) -> the claims that observable refutes.
    let mut refuted: BTreeMap<(String, PathBuf, String, String), Vec<String>> = BTreeMap::new();
    let mut out = Vec::new();
    for result in results {
        for checked in &result.checked {
            if let Verdict::Refuted(reason) = &checked.verdict {
                refuted
                    .entry((
                        result.task_id.clone(),
                        result.source_path.clone(),
                        checked.subject.clone(),
                        reason.clone(),
                    ))
                    .or_default()
                    .push(result.requirement.clone());
            }
        }
        if let Some(reason) = &result.untestable {
            out.push(TracePolicyFinding {
                text: format!(
                    "task {}: its claim of '{}' cannot be tested at base commit {base}: {reason}; re-author the task body so a sentence naming {} also names the file, symbol or verifier command that makes it observable",
                    result.task_id, result.requirement, result.requirement
                ),
                subject: result.task_id.clone(),
                source_path: result.source_path.clone(),
                remediation_scope: archon_workflow::RemediationScope::Body,
            });
        }
    }
    for ((task, source_path, subject, reason), claims) in refuted {
        let quoted: Vec<String> = claims.iter().map(|id| format!("'{id}'")).collect();
        out.push(TracePolicyFinding {
            text: format!(
                "task {task}: its claim of {} is refuted at base commit {base}: {subject}: {reason}; re-author the task body so every claim rests only on files, symbols and verifier commands that exist at the base commit or that a task in the set declares it will create",
                quoted.join(", ")
            ),
            subject: task,
            source_path,
            remediation_scope: archon_workflow::RemediationScope::Body,
        });
    }
    out
}

pub(super) fn render(index: &EvidenceIndex, results: &[ClaimResult]) -> String {
    let refuted = |r: &ClaimResult| {
        r.checked
            .iter()
            .any(|c| matches!(c.verdict, Verdict::Refuted(_)))
    };
    let count = |f: &dyn Fn(&Verdict) -> bool| {
        results
            .iter()
            .flat_map(|r| &r.checked)
            .filter(|c| f(&c.verdict))
            .count()
    };
    let refuted_claims = results.iter().filter(|r| refuted(r)).count();
    let untestable = results.iter().filter(|r| r.untestable.is_some()).count();
    let mut out = format!(
        "\nClaim falsification against {} at base commit {}:\n  {} claim(s): {} tested, {} refuted, {} untestable\n  observables: {} exist, {} derived from declarations, {} refuted, {} untested\n",
        index.root_text(),
        index.base(),
        results.len(),
        results.len() - untestable,
        refuted_claims,
        untestable,
        count(&|v| matches!(v, Verdict::Exists(_))),
        count(&|v| matches!(v, Verdict::Derived(_))),
        count(&|v| matches!(v, Verdict::Refuted(_))),
        count(&|v| matches!(v, Verdict::Untested(_))),
    );
    for result in results {
        let head = format!("{} {}", result.task_id, result.requirement);
        if let Some(reason) = &result.untestable {
            out.push_str(&format!("  UNTESTABLE {head}: {reason}\n"));
        } else if !refuted(result) {
            let exist = result
                .checked
                .iter()
                .filter(|c| matches!(c.verdict, Verdict::Exists(_)))
                .count();
            let derived = result
                .checked
                .iter()
                .filter(|c| matches!(c.verdict, Verdict::Derived(_)))
                .count();
            out.push_str(&format!(
                "  tested {head}: {exist} exist, {derived} derived\n"
            ));
        }
        for checked in &result.checked {
            match &checked.verdict {
                Verdict::Refuted(reason) => {
                    out.push_str(&format!(
                        "  REFUTED {head}: {}: {reason}\n",
                        checked.subject
                    ));
                }
                Verdict::Untested(reason) => {
                    out.push_str(&format!("    untested {}: {reason}\n", checked.subject));
                }
                _ => {}
            }
        }
    }
    out
}
