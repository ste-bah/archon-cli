//! Does a task declare a deliverable the runtime will accept, on a task that
//! can produce it?
//!
//! The decomposition's own §11 gate runs this lint over what it wrote, and
//! until now the lint never looked at a deliverable contract at all — it
//! rendered diamonds, edge support and fusion, all graph analyses. So a
//! contract could pass the gate and be refused by the runtime seventeen hours
//! later, in the same codebase, for a reason the codebase already knew.
//!
//! Observed live: a task declared
//! `${PROJECT_ROOT}/…/${DATASET_ID}/${VERSION}/validation.json` with
//! `min_instances: 1` — every binding the gate asks for, in the one syntax it
//! refuses to read — on a task that could not have produced an instance
//! anyway, because the datasets that path indexes are created by a task it
//! `blocks`. Four remediation cycles were spent on a defect no code change
//! could fix.
//!
//! The findings come from [`archon_workflow::task_universe_contract_audit`],
//! which asks the runtime's own predicate rather than reimplementing it. That
//! is the point: lint and gate are the same function, so a contract cannot pass
//! one and fail the other.
//!
//! **Advisory, like every other section here.** Certain findings are marked as
//! such so a caller that wants to gate has something to gate on, but nothing in
//! this file blocks anything. A heuristic that blocks correct work gets the
//! whole lint switched off, and a lint nobody runs catches nothing.

use std::path::Path;

use archon_workflow::task_universe::parsing::parse_task_file;
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, task_files_under};
use archon_workflow::task_universe_contract_audit::{ContractFindingKind, audit_contracts};

/// Contracts the runtime is certain to refuse, for a caller that wants to
/// block rather than report.
///
/// Only the certain half. The ownership heuristic is deliberately excluded: a
/// guess that blocks a decomposition is a guess the author cannot argue with,
/// and the first time it is wrong the whole gate gets switched off.
#[cfg(test)]
pub(crate) fn blocking_findings(tasks_root: Option<&Path>) -> Vec<String> {
    blocking_defects(tasks_root)
        .into_iter()
        .map(|defect| defect.message)
        .collect()
}

/// Every certain contract defect and every unparseable spec under `root`,
/// each with its host identity, from one read of the task directory.
pub(super) fn blocking_defects(
    tasks_root: Option<&Path>,
) -> Vec<archon_workflow::defect::ValidationDefect> {
    use archon_workflow::defect::ValidationDefect;
    let Some(root) = tasks_root else {
        return Vec::new();
    };
    let Ok(paths) = task_files_under(root) else {
        return vec![ValidationDefect::new(
            "unreadable_task_directory",
            "tasks",
            "directory",
            "the task directory could not be read",
        )];
    };
    // An unreadable spec is one defect, never permission to hide defects in
    // every other readable spec. Structural slots exclude rejected filenames.
    let mut universe = WorkflowV2TaskUniverse {
        schema_version: String::new(),
        source_roots: vec![root.display().to_string()],
        tasks: Vec::new(),
    };
    let mut findings = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unnamed>");
        let problem = match std::fs::read_to_string(path) {
            Ok(raw) => match parse_task_file(path, &raw) {
                Ok(task) => {
                    universe.tasks.push(task);
                    None
                }
                Err(error) => Some(("invalid_task_spec", format!("{name}: {error}"))),
            },
            Err(error) => Some((
                "unreadable_task_spec",
                format!("{name}: unreadable: {error}"),
            )),
        };
        if let Some((code, message)) = problem {
            findings.push(ValidationDefect::new(
                code,
                &format!("tasks/{index}"),
                "spec",
                message,
            ));
        }
    }
    findings.extend(
        audit_contracts(&universe)
            .into_iter()
            .filter(|finding| finding.kind.is_certain())
            .filter_map(|finding| {
                finding.identity.map(|identity| ValidationDefect {
                    identity,
                    message: format!("{}: {}", finding.task_id, finding.message),
                })
            }),
    );
    findings
}

/// Gate findings for `defects`, each carrying its own identity.
pub(super) fn gate_findings(
    defects: Vec<archon_workflow::defect::ValidationDefect>,
    (gate_id, subject, source_path, scope): (
        crate::command::workflow_gate::GateId,
        &str,
        Option<&Path>,
        archon_workflow::RemediationScope,
    ),
) -> Vec<crate::command::workflow_gate::GateFinding> {
    use crate::command::workflow_gate::{GateFinding, finding_subject};
    let path = source_path.map(Path::to_path_buf);
    defects
        .into_iter()
        .map(|defect| {
            let named = finding_subject(&defect.message, subject);
            GateFinding::new(gate_id, defect.message, named, path.clone(), scope)
                .with_defect(defect.identity)
        })
        .collect()
}

/// Parse every task file under `root`, skipping the ones that will not parse.
fn load_universe(root: &Path) -> Option<WorkflowV2TaskUniverse> {
    let paths = task_files_under(root).ok()?;
    let mut universe = WorkflowV2TaskUniverse {
        schema_version: String::new(),
        source_roots: vec![root.display().to_string()],
        tasks: Vec::new(),
    };
    for path in &paths {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Ok(task) = parse_task_file(path, &raw) {
            universe.tasks.push(task);
        }
    }
    Some(universe)
}

pub(super) fn section(tasks_root: Option<&Path>) -> String {
    let mut out = String::from("\n## deliverable contracts\n");
    let Some(root) = tasks_root else {
        out.push_str("  not analysed: this lint needs a task directory\n");
        return out;
    };
    let Some(universe) = load_universe(root) else {
        out.push_str(&format!(
            "  could not read task files under {}\n",
            root.display()
        ));
        return out;
    };
    // Nothing parsed is NOT a clean bill of health. Reported as a pass, it is
    // indistinguishable from a task set whose contracts are all fine — and a
    // whole decomposition once printed "every declared contract is satisfiable"
    // while all fifteen of its specs were unreadable.
    if universe.tasks.is_empty() {
        out.push_str("  no task file could be read; NOTHING was checked, and this is not a pass\n");
        return out;
    }
    let findings = audit_contracts(&universe);
    if findings.is_empty() {
        out.push_str(&format!(
            "  {} task(s) checked; every declared contract is satisfiable as written\n",
            universe.tasks.len()
        ));
        return out;
    }
    for finding in &findings {
        let label = match finding.kind {
            ContractFindingKind::Unsatisfiable => "REFUSED BY THE RUNTIME",
            ContractFindingKind::RepairedAtLoad => "repaired at load",
            ContractFindingKind::Misallocated => "likely on the wrong task",
        };
        out.push_str(&format!(
            "  [{}] {label}\n    {}\n    {}\n",
            finding.task_id, finding.artifact_path, finding.message
        ));
    }
    let certain = findings
        .iter()
        .filter(|finding| finding.kind.is_certain())
        .count();
    out.push_str(&format!(
        "  {} finding(s), {certain} of which the runtime will refuse outright\n",
        findings.len()
    ));
    out
}
