use super::*;

/// A remediation as the acceptance stage routes it: its units name the
/// observation they fix (`observedBy`). The first round's failing check,
/// already observed.
pub(super) fn script(rounds: u32, with_b: bool, b_targets: &[&str]) -> String {
    script_routed(rounds, with_b, b_targets, true)
}

pub(super) fn script_routed(rounds: u32, with_b: bool, b_targets: &[&str], routed: bool) -> String {
    let observed = if routed {
        "observedBy: ['acceptance-contract-run-1'], "
    } else {
        ""
    };
    let mut findings = vec![json!({"id": "gate", "canonical_task_ids": ["TASK-A"],
        "severity": "high", "claim": "register only real data"})];
    if with_b {
        findings.push(json!({"id": "doc", "canonical_task_ids": ["TASK-B"],
            "severity": "medium", "claim": "document it"}));
    }
    format!(
        r#"export const meta = {{ name: 'refused', description: 'd', phases: [] }}
const tasks = [
  {{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['{A_FILE}'] }},
  {{ id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: {b} }},
]
const byId = (id) => tasks.find((t) => t.id === id) || {{}}
return await remediateFindings({findings}, {{ {observed}maxRounds: {rounds}, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }})
"#,
        b = json!(b_targets),
        findings = json!(findings),
    )
}
