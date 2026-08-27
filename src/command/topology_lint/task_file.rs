//! Exact-one TASK-file linting against portable freezes and the host pin.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::obligation_ids::acceptance_ids;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptancePin, TASK_SKELETON_FILE,
    TASK_SKELETON_LOCK_FILE, content_digest, validate_acceptance_bundle,
};
use archon_workflow::task_skeleton::{compare_frozen_task, validate_full_chain};
use archon_workflow::task_universe::parsing::parse_task_file;
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::task_universe_contract_audit::{ContractFindingKind, audit_contracts};

pub(super) struct TaskFileLint {
    pub(super) report: String,
    pub(super) blockers: Vec<String>,
}

pub(super) fn inspect(
    cwd: &Path,
    path: &Path,
    mode: archon_core::config::GateMode,
) -> TaskFileLint {
    let path = absolute(cwd, path);
    let mut report = format!("# topology lint — task file {}\n", path.display());
    let mut blockers = Vec::new();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            blockers.push(format!(
                "{}: unreadable: {error}; restore the TASK file and re-run `workflow lint --task-file {}`",
                path.display(),
                path.display()
            ));
            return finish(report, blockers);
        }
    };
    let task = match parse_task_file(&path, &raw) {
        Ok(task) => task,
        Err(error) => {
            blockers.push(format!(
                "{}: {error}; make the exact parser-required edit and re-run `workflow lint --task-file {}`",
                path.display(),
                path.display()
            ));
            return finish(report, blockers);
        }
    };
    report.push_str(&format!(
        "\n## parser\n  {} parsed with runtime parse_task_file as {}\n",
        path.display(),
        task.canonical_task_id
    ));
    validate_declared_shape(&task, &raw, &mut report, &mut blockers);
    blockers.extend(
        archon_workflow::task_set_edges::validate_dependency_declarations(
            &task.canonical_task_id,
            &task.dependencies,
        )
        .into_iter()
        .map(|finding| format!("{}: {}", finding.field, finding.message)),
    );
    report.push_str(
        "\n## dependency contracts\n  local consumes/ordering_only shape checked; producer-path and multi-writer matching are NOT ANALYSED for --task-file and run under --tasks\n",
    );

    let Some(tasks_root) = path.parent() else {
        blockers.push(format!(
            "{} has no parent task directory; move it under a task directory and re-run --task-file",
            path.display()
        ));
        return finish(report, blockers);
    };
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, tasks_root);
    let pin: AcceptancePin = match read_json(&pin_path, "acceptance pin", "freeze-acceptance") {
        Ok(pin) => pin,
        Err(finding) => {
            blockers.push(finding);
            return finish(report, blockers);
        }
    };
    append_predecessor_finding(
        mode,
        "acceptance",
        pin.acceptance_gate.finding_count,
        &mut blockers,
    );
    let expected_ids = match validate_prd_identity(cwd, tasks_root) {
        Ok(ids) => ids,
        Err(finding) => {
            blockers.push(finding);
            return finish(report, blockers);
        }
    };
    if let Err(error) = validate_acceptance_bundle(tasks_root, Some(&pin), &expected_ids) {
        blockers.push(error.to_string());
        return finish(report, blockers);
    }
    report.push_str(
        "\n## acceptance freeze\n  acceptance contract, lock, PRD digest, and host pin match\n",
    );

    let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
    let lock_path = tasks_root.join(TASK_SKELETON_LOCK_FILE);
    let skeleton_present = skeleton_path.exists();
    let lock_present = lock_path.exists();
    match (skeleton_present, lock_present, pin.skeleton_digest.is_some()) {
        (false, false, false) => report.push_str(
            "\n## frozen skeleton\n  Step-1 compatibility mode: no skeleton file, lock, or pin exists; frozen-field equality is NOT ANALYSED\n",
        ),
        (true, true, true) => match validate_full_chain(tasks_root, &pin) {
            Ok(skeleton) => {
                if let Some(stamp) = &pin.skeleton_gate {
                    append_predecessor_finding(
                        mode,
                        "task skeleton",
                        stamp.finding_count,
                        &mut blockers,
                    );
                }
                let Some(frozen) = skeleton
                    .tasks
                    .iter()
                    .find(|frozen| frozen.task_id == task.canonical_task_id)
                else {
                    blockers.push(format!(
                        "{} is absent from {}; add its entry and re-run `workflow freeze-skeleton` before body writing",
                        task.canonical_task_id,
                        skeleton_path.display()
                    ));
                    return finish(report, blockers);
                };
                let findings = compare_frozen_task(&task, frozen);
                if findings.is_empty() {
                    report.push_str("\n## frozen skeleton\n  frozen fields match structurally\n");
                } else {
                    blockers.extend(findings.into_iter().map(|finding| {
                        format!("{}: {}", task.canonical_task_id, finding.message)
                    }));
                }
            }
            Err(error) => blockers.push(error.to_string()),
        },
        _ => blockers.push(format!(
            "partial skeleton freeze beside {}: file={}, lock={}, pin={}; restore all three matching artifacts or re-run `workflow freeze-skeleton`",
            path.display(), skeleton_present, lock_present, pin.skeleton_digest.is_some()
        )),
    }

    let universe = WorkflowV2TaskUniverse {
        schema_version: "v1".into(),
        source_roots: vec![tasks_root.display().to_string()],
        tasks: vec![task],
    };
    let contract_findings = audit_contracts(&universe);
    for finding in &contract_findings {
        if finding.kind.is_certain() {
            blockers.push(format!("{}: {}", finding.task_id, finding.message));
        }
    }
    if contract_findings.is_empty() {
        report.push_str(
            "\n## deliverable contracts\n  every declared contract is satisfiable as written\n",
        );
    } else {
        report.push_str("\n## deliverable contracts\n");
        for finding in contract_findings {
            let label = if finding.kind == ContractFindingKind::Misallocated {
                "REPORT ONLY"
            } else if finding.kind.is_certain() {
                "REFUSED BY THE RUNTIME"
            } else {
                "REPAIRED AT LOAD"
            };
            report.push_str(&format!(
                "  [{}] {label}: {}\n    {}\n",
                finding.task_id, finding.artifact_path, finding.message
            ));
        }
    }
    finish(report, blockers)
}

fn validate_declared_shape(
    task: &WorkflowV2TaskUniverseTask,
    raw: &str,
    report: &mut String,
    blockers: &mut Vec<String>,
) {
    for issue in &task.section_heading_issues {
        blockers.push(format!(
            "{}: heading issue '{issue}'; rename the heading to the exact required TASK section name",
            task.canonical_task_id
        ));
    }
    if let Err(error) =
        archon_workflow::task_universe::validate_declared_statuses(std::slice::from_ref(task))
    {
        blockers.push(error.to_string());
    }
    if !super::declarations::task_has_runnable_test(raw) {
        blockers.push(super::declarations::missing_runnable_test_finding(
            &task.canonical_task_id,
        ));
    }
    if blockers.is_empty() {
        report.push_str(
            "\n## task shape\n  headings, status, implements, and focused test shape pass\n",
        );
    }
}

fn validate_prd_identity(cwd: &Path, tasks_root: &Path) -> Result<BTreeSet<String>, String> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract: AcceptanceContract =
        read_json(&contract_path, "acceptance contract", "freeze-acceptance")?;
    let prd_path = absolute(cwd, Path::new(&contract.prd.path));
    let bytes = std::fs::read(&prd_path).map_err(|error| {
        format!(
            "PRD {} could not be read: {error}; restore it or re-run `workflow freeze-acceptance`",
            prd_path.display()
        )
    })?;
    let actual = content_digest(&bytes);
    if actual != contract.prd.digest {
        return Err(format!(
            "PRD digest mismatch for {}: expected {}, actual {}; restore the frozen PRD or re-run `workflow freeze-acceptance`",
            prd_path.display(),
            contract.prd.digest,
            actual
        ));
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        format!(
            "PRD {} is not UTF-8: {error}; restore UTF-8 content and re-run `workflow freeze-acceptance`",
            prd_path.display()
        )
    })?;
    Ok(acceptance_ids(&text))
}

fn append_predecessor_finding(
    mode: archon_core::config::GateMode,
    label: &str,
    finding_count: usize,
    blockers: &mut Vec<String>,
) {
    if finding_count == 0 || mode == archon_core::config::GateMode::Off {
        return;
    }
    blockers.push(format!(
        "predecessor {label} freeze carries {finding_count} policy finding(s); re-freeze under enforce and resolve every named finding before continuing"
    ));
}

fn read_json<T: for<'de> serde::Deserialize<'de>>(
    path: &Path,
    label: &str,
    freeze: &str,
) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "required {label} {} could not be read: {error}; run workflow {freeze}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{label} {} is malformed: {error}; re-run workflow {freeze}",
            path.display()
        )
    })
}

fn finish(mut report: String, blockers: Vec<String>) -> TaskFileLint {
    report.push_str("\n## set-level checks\n  coverage: NOT ANALYSED for --task-file\n  edges: NOT ANALYSED for --task-file\n");
    if blockers.is_empty() {
        report.push_str("\nresult: PASS\n");
    } else {
        report.push_str("\n## blocking findings\n");
        for finding in &blockers {
            report.push_str(&format!("  {finding}\n"));
        }
    }
    TaskFileLint { report, blockers }
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
