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

pub(super) fn section(tasks_root: Option<&Path>) -> String {
    let mut out = String::from("\n## deliverable contracts\n");
    let Some(root) = tasks_root else {
        out.push_str("  not analysed: this lint needs a task directory\n");
        return out;
    };
    let paths = match task_files_under(root) {
        Ok(paths) => paths,
        Err(error) => {
            out.push_str(&format!("  could not read task files: {error}\n"));
            return out;
        }
    };
    if paths.is_empty() {
        out.push_str(&format!("  no task files under {}.\n", root.display()));
        return out;
    }
    // A file that will not parse is skipped rather than failing the section,
    // for the reason `run_lint` already gives: one malformed file must not cost
    // the reader everything the other fourteen would have told them.
    let mut universe = WorkflowV2TaskUniverse {
        schema_version: String::new(),
        source_roots: vec![root.display().to_string()],
        tasks: Vec::new(),
    };
    let mut unreadable = 0usize;
    for path in &paths {
        let Ok(raw) = std::fs::read_to_string(path) else {
            unreadable += 1;
            continue;
        };
        match parse_task_file(path, &raw) {
            Ok(task) => universe.tasks.push(task),
            Err(_) => unreadable += 1,
        }
    }
    if unreadable > 0 {
        out.push_str(&format!(
            "  {unreadable} task file(s) did not parse and were skipped\n"
        ));
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
