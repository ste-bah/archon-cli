//! Exact-one TASK-file linting against portable freezes and the host pin.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

use archon_workflow::defect::{DefectStage, DeterministicDefect, ValidationDefect};

/// Identities for the blockers, keyed by blocker text in report order.
type Identities = BTreeMap<String, VecDeque<DeterministicDefect>>;

/// Records a deterministic blocker with its host identity (Issue 261: no
/// body-lint finding may reach the author loop looking like a judge's).
fn block(
    blockers: &mut Vec<String>,
    identities: &mut Identities,
    text: String,
    defect: DeterministicDefect,
) {
    identities
        .entry(text.clone())
        .or_default()
        .push_back(defect);
    blockers.push(text);
}

/// A predecessor validator's own defects, every one of them, seen from this
/// lint: each keeps its identity and is reported at the freeze stage.
fn block_predecessor(
    blockers: &mut Vec<String>,
    identities: &mut Identities,
    defects: &[ValidationDefect],
    fallback: (&str, String),
) {
    if defects.is_empty() {
        block(
            blockers,
            identities,
            fallback.1,
            DeterministicDefect::new(fallback.0, "task_file", "predecessor"),
        );
    }
    for defect in defects {
        let identity = defect.identity.clone().at_stage(DefectStage::Freeze);
        block(blockers, identities, defect.message.clone(), identity);
    }
}

pub(super) struct TaskFileLint {
    pub(super) report: String,
    pub(super) blockers: Vec<String>,
    pub(super) inherited_blockers: BTreeSet<String>,
    pub(super) deterministic:
        BTreeMap<String, VecDeque<archon_workflow::defect::DeterministicDefect>>,
}

pub(super) fn inspect(
    cwd: &Path,
    path: &Path,
    mode: archon_core::config::GateMode,
) -> TaskFileLint {
    let path = absolute(cwd, path);
    match std::fs::read_to_string(&path) {
        Ok(raw) => inspect_raw(cwd, &path, &raw, mode),
        Err(error) => {
            let report = format!("# topology lint — task file {}\n", path.display());
            let (mut blockers, mut identities) = (Vec::new(), Identities::new());
            block(
                &mut blockers,
                &mut identities,
                format!(
                    "{}: unreadable: {error}; restore the TASK file and re-run `workflow lint --task-file {}`",
                    path.display(),
                    path.display()
                ),
                DeterministicDefect::new("unreadable_task_file", "task_file", "file"),
            );
            finish(report, blockers, BTreeSet::new(), identities)
        }
    }
}

pub(super) fn inspect_raw(
    cwd: &Path,
    path: &Path,
    raw: &str,
    mode: archon_core::config::GateMode,
) -> TaskFileLint {
    let path = absolute(cwd, path);
    let mut report = format!("# topology lint — task file {}\n", path.display());
    let mut blockers = Vec::new();
    let mut deterministic: BTreeMap<_, VecDeque<_>> = BTreeMap::new();
    let mut inherited_blockers = BTreeSet::new();
    let task = match parse_task_file(&path, raw) {
        Ok(task) => task,
        Err(error) => {
            // The parser stops at its first error: one parse-stage defect.
            block(
                &mut blockers,
                &mut deterministic,
                format!(
                    "{}: {error}; make the exact parser-required edit and re-run `workflow lint --task-file {}`",
                    path.display(),
                    path.display()
                ),
                DeterministicDefect::new("unparseable_task_file", "task_file", "parse"),
            );
            return finish(report, blockers, inherited_blockers, deterministic);
        }
    };
    let id = task.canonical_task_id.clone();
    report.push_str(&format!(
        "\n## parser\n  {} parsed with runtime parse_task_file as {}\n",
        path.display(),
        task.canonical_task_id
    ));
    validate_declared_shape(&task, raw, &mut report, &mut blockers, &mut deterministic);
    for (index, text) in super::tool_obligations::inspect(cwd, &task, raw)
        .into_iter()
        .enumerate()
    {
        let defect = DeterministicDefect::new("tool_obligation", &id, format!("tools/{index}"));
        block(&mut blockers, &mut deterministic, text, defect);
    }
    blockers.extend(
        archon_workflow::task_set_edges::validate_dependency_declarations(
            &task.canonical_task_id,
            &task.canonical_task_id,
            &task.dependencies,
        )
        .into_iter()
        .map(|finding| {
            let text = format!("{}: {}", finding.field, finding.message);
            deterministic
                .entry(text.clone())
                .or_default()
                .push_back(finding.identity);
            text
        }),
    );
    report.push_str(
        "\n## dependency contracts\n  local consumes/ordering_only shape checked; producer-path and multi-writer matching are NOT ANALYSED for --task-file and run under --tasks\n",
    );

    let Some(tasks_root) = path.parent() else {
        block(
            &mut blockers,
            &mut deterministic,
            format!(
                "{} has no parent task directory; move it under a task directory and re-run --task-file",
                path.display()
            ),
            DeterministicDefect::new("task_file_without_directory", &id, "path"),
        );
        return finish(report, blockers, inherited_blockers, deterministic);
    };
    if super::task_set::freeze_chain_is_absent(cwd, tasks_root) {
        report.push_str(
            "\n## acceptance freeze\n  legacy compatibility: no freeze artifacts exist; predecessor integrity is NOT ANALYSED and this is not a freeze pass\n",
        );
    } else {
        let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, tasks_root);
        let pin: AcceptancePin = match read_json(&pin_path, "acceptance pin", "freeze-acceptance") {
            Ok(pin) => pin,
            Err(finding) => {
                let defect =
                    DeterministicDefect::new("unreadable_acceptance_pin", &id, "acceptance_pin");
                block(&mut blockers, &mut deterministic, finding, defect);
                return finish(report, blockers, inherited_blockers, deterministic);
            }
        };
        append_predecessor_finding(
            mode,
            "acceptance",
            pin.acceptance_gate.finding_count,
            &mut blockers,
            &mut inherited_blockers,
            &mut deterministic,
        );
        let expected_ids = match validate_prd_identity(cwd, tasks_root) {
            Ok(ids) => ids,
            Err(finding) => {
                let defect = DeterministicDefect::new("prd_identity_mismatch", &id, "prd");
                block(&mut blockers, &mut deterministic, finding, defect);
                return finish(report, blockers, inherited_blockers, deterministic);
            }
        };
        if let Err(error) = validate_acceptance_bundle(tasks_root, Some(&pin), &expected_ids) {
            let fallback = ("invalid_acceptance_bundle", error.to_string());
            block_predecessor(&mut blockers, &mut deterministic, &error.defects, fallback);
            return finish(report, blockers, inherited_blockers, deterministic);
        }
        report.push_str(
            "\n## acceptance freeze\n  acceptance contract, lock, PRD digest, and host pin match\n",
        );

        let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
        let lock_path = tasks_root.join(TASK_SKELETON_LOCK_FILE);
        let skeleton_present = skeleton_path.exists();
        let lock_present = lock_path.exists();
        let skeleton_pin_present = pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some();
        match (skeleton_present, lock_present, skeleton_pin_present) {
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
                            &mut inherited_blockers,
                            &mut deterministic,
                        );
                    }
                    let Some(frozen) = skeleton
                        .tasks
                        .iter()
                        .find(|frozen| frozen.task_id == task.canonical_task_id)
                    else {
                        block(&mut blockers, &mut deterministic, format!(
                            "{} is absent from {}; add its entry and re-run `workflow freeze-skeleton` before body writing",
                            task.canonical_task_id,
                            skeleton_path.display()
                        ), DeterministicDefect::new("task_absent_from_skeleton", &id, "skeleton"));
                        return finish(report, blockers, inherited_blockers, deterministic);
                    };
                    let findings = compare_frozen_task(&task, frozen);
                    if findings.is_empty() {
                        report.push_str("\n## frozen skeleton\n  frozen fields match structurally\n");
                    } else {
                        blockers.extend(findings.into_iter().map(|finding| {
                            let text = format!("{}: {}", task.canonical_task_id, finding.message);
                            deterministic.entry(text.clone()).or_default().push_back(finding.identity);
                            text
                        }));
                    }
                }
                Err(error) => {
                    let fallback = ("invalid_skeleton_chain", error.to_string());
                    block_predecessor(&mut blockers, &mut deterministic, &error.defects, fallback);
                }
            },
            _ => block(&mut blockers, &mut deterministic, format!(
                "partial skeleton freeze beside {}: file={}, lock={}, pin={}; restore all three matching artifacts or re-run `workflow freeze-skeleton`",
                path.display(), skeleton_present, lock_present, skeleton_pin_present
            ), DeterministicDefect::new("partial_skeleton_freeze", &id, "skeleton")),
        }
    }

    let universe = WorkflowV2TaskUniverse {
        schema_version: "v1".into(),
        source_roots: vec![tasks_root.display().to_string()],
        tasks: vec![task],
    };
    let contract_findings = audit_contracts(&universe);
    for finding in &contract_findings {
        if finding.kind.is_certain() {
            let text = format!("{}: {}", finding.task_id, finding.message);
            if let Some(identity) = &finding.identity {
                deterministic
                    .entry(text.clone())
                    .or_default()
                    .push_back(identity.clone());
            }
            blockers.push(text);
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
    finish(report, blockers, inherited_blockers, deterministic)
}

fn validate_declared_shape(
    task: &WorkflowV2TaskUniverseTask,
    raw: &str,
    report: &mut String,
    blockers: &mut Vec<String>,
    identities: &mut Identities,
) {
    // Every shape defect of the task, each with its own identity.
    let id = task.canonical_task_id.as_str();
    for (index, issue) in task.section_heading_issues.iter().enumerate() {
        let text = format!(
            "{id}: heading issue '{issue}'; rename the heading to the exact required TASK section name"
        );
        let defect = DeterministicDefect::new("section_heading", id, format!("headings/{index}"));
        block(blockers, identities, text, defect);
    }
    if let Err(error) =
        archon_workflow::task_universe::validate_declared_statuses(std::slice::from_ref(task))
    {
        let defect = DeterministicDefect::new("invalid_declared_status", id, "status");
        block(blockers, identities, error.to_string(), defect);
    }
    for (index, text) in super::scope_declarations::inspect(task)
        .into_iter()
        .enumerate()
    {
        let defect = DeterministicDefect::new("scope_declaration", id, format!("scope/{index}"));
        block(blockers, identities, text, defect);
    }
    if !super::declarations::task_has_runnable_test(raw) {
        let text = super::declarations::missing_runnable_test_finding(id);
        let defect = DeterministicDefect::new("missing_runnable_test", id, "focused_tests");
        block(blockers, identities, text, defect);
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
    inherited_blockers: &mut BTreeSet<String>,
    identities: &mut Identities,
) {
    if finding_count == 0 || mode == archon_core::config::GateMode::Off {
        return;
    }
    let text = format!(
        "predecessor {label} freeze carries {finding_count} policy finding(s); re-freeze under enforce and resolve every named finding before continuing"
    );
    inherited_blockers.insert(text.clone());
    let defect = DeterministicDefect::new("predecessor_findings", label, "freeze");
    block(blockers, identities, text, defect);
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

fn finish(
    mut report: String,
    blockers: Vec<String>,
    inherited_blockers: BTreeSet<String>,
    deterministic: Identities,
) -> TaskFileLint {
    report.push_str("\n## set-level checks\n  coverage: NOT ANALYSED for --task-file\n  edges: NOT ANALYSED for --task-file\n");
    if blockers.is_empty() {
        report.push_str("\nresult: PASS\n");
    } else {
        report.push_str("\n## blocking findings\n");
        for finding in &blockers {
            report.push_str(&format!("  {finding}\n"));
        }
    }
    TaskFileLint {
        report,
        blockers,
        inherited_blockers,
        deterministic,
    }
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
