//! Directory-wide checks for optional portable freezes, structured edges, and TASK equality.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use archon_core::config::GateMode;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptancePin,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest, validate_acceptance_bundle,
};
use archon_workflow::task_set_edges::analyze_task_set_edges;
use archon_workflow::task_skeleton::{
    FrozenTask, TaskSkeleton, compare_task_set, validate_full_chain, validate_skeleton,
    validate_skeleton_set,
};
use archon_workflow::task_universe::parsing::parse_task_file;
use archon_workflow::task_universe::{
    WorkflowV2TaskUniverseTask, resolve_task_references, short_task_alias, task_files_under,
};

pub(super) struct TaskSetFreezeLint {
    pub(super) report: String,
    pub(super) blockers: Vec<String>,
    pub(super) inherited_blockers: std::collections::BTreeSet<String>,
}

pub(super) fn freeze_chain_is_absent(cwd: &Path, tasks_root: &Path) -> bool {
    let pin = crate::command::workflow_task_set::acceptance_pin_path(cwd, tasks_root);
    [
        tasks_root.join(ACCEPTANCE_CONTRACT_FILE),
        tasks_root.join(ACCEPTANCE_LOCK_FILE),
        pin,
        tasks_root.join(TASK_SKELETON_FILE),
        tasks_root.join(TASK_SKELETON_LOCK_FILE),
    ]
    .iter()
    .all(|path| !path.exists())
}

pub(super) fn inspect(
    project_root: &Path,
    tasks_root: &Path,
    mode: GateMode,
) -> Result<TaskSetFreezeLint> {
    let mut report = String::from("\n## task-set freeze\n");
    let mut blockers = Vec::new();
    let mut inherited_blockers = std::collections::BTreeSet::new();
    let tasks = load_tasks(tasks_root)?;
    let runtime_skeleton = skeleton_from_tasks(&tasks, String::new());

    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, tasks_root);
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let acceptance_lock_path = tasks_root.join(ACCEPTANCE_LOCK_FILE);
    let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
    let skeleton_lock_path = tasks_root.join(TASK_SKELETON_LOCK_FILE);
    let acceptance_state = [
        contract_path.exists(),
        acceptance_lock_path.exists(),
        pin_path.exists(),
    ];
    let skeleton = if freeze_chain_is_absent(project_root, tasks_root) {
        report.push_str(
            "  legacy compatibility: no freeze artifacts exist; freeze integrity is NOT ANALYSED and this is not a freeze pass\n",
        );
        runtime_skeleton
    } else {
        if !acceptance_state.iter().all(|exists| *exists) {
            return Err(anyhow::anyhow!(
                "partial acceptance freeze beside {}: contract={}, lock={}, pin={}; restore all three matching artifacts or re-run `workflow freeze-acceptance`",
                tasks_root.display(),
                acceptance_state[0],
                acceptance_state[1],
                acceptance_state[2],
            ));
        }
        let pin = read_pin(&pin_path)?;
        let (_contract, expected_obligations) =
            validate_acceptance_phase(project_root, tasks_root, &contract_path, &pin)?;
        append_predecessor_finding(
            mode,
            "acceptance",
            &pin.acceptance_gate,
            &mut blockers,
            &mut inherited_blockers,
        );

        let skeleton_state = [
            skeleton_path.exists(),
            skeleton_lock_path.exists(),
            pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some(),
        ];
        match skeleton_state {
            [false, false, false] => {
                report.push_str(
                    "  acceptance freeze is complete; task skeleton has not been authored, so skeleton equality is NOT ANALYSED\n",
                );
                skeleton_from_tasks(&tasks, pin.acceptance_digest)
            }
            [true, false, false] => {
                let draft = read_draft_skeleton(&skeleton_path)?;
                validate_skeleton(&draft, &pin.acceptance_digest)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                blockers.extend(
                    validate_skeleton_set(&draft, &expected_obligations)
                        .into_iter()
                        .map(|finding| format!("{}: {}", finding.field, finding.message)),
                );
                report.push_str(
                    "  draft skeleton was linted against the acceptance freeze; it is NOT frozen until `workflow freeze-skeleton` publishes its lock and pin\n",
                );
                draft
            }
            [true, true, true] => {
                let frozen = validate_full_chain(tasks_root, &pin)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                if let Some(stamp) = &pin.skeleton_gate {
                    append_predecessor_finding(
                        mode,
                        "task skeleton",
                        stamp,
                        &mut blockers,
                        &mut inherited_blockers,
                    );
                }
                blockers.extend(
                    compare_task_set(&tasks, &frozen)
                        .into_iter()
                        .map(|finding| format!("{}: {}", finding.field, finding.message)),
                );
                if blockers.is_empty() {
                    report.push_str(&format!(
                        "  {} TASK file(s) match the frozen skeleton and full digest chain\n",
                        tasks.len()
                    ));
                }
                frozen
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "partial skeleton freeze beside {}: file={}, lock={}, pin={}; restore a matching frozen triple, keep only an unlocked draft file, or re-run `workflow freeze-skeleton`",
                    tasks_root.display(),
                    skeleton_state[0],
                    skeleton_state[1],
                    skeleton_state[2],
                ));
            }
        }
    };

    append_edge_analysis(&skeleton, &mut report, &mut blockers);
    Ok(finish(report, blockers, inherited_blockers))
}

fn load_tasks(tasks_root: &Path) -> Result<Vec<WorkflowV2TaskUniverseTask>> {
    let paths = task_files_under(tasks_root).map_err(|error| {
        anyhow::anyhow!(
            "task directory {} could not be enumerated: {error}; restore it and re-run workflow lint --tasks",
            tasks_root.display()
        )
    })?;
    let mut tasks = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("{} is unreadable; restore the TASK file", path.display()))?;
        tasks.push(parse_task_file(&path, &raw).map_err(|error| {
            anyhow::anyhow!(
                "{}: {error}; make the parser-required edit and re-run workflow lint --tasks",
                path.display()
            )
        })?);
    }
    let mut aliases = BTreeMap::new();
    for task in &tasks {
        aliases.insert(
            task.canonical_task_id.clone(),
            task.canonical_task_id.clone(),
        );
        if let Some(short) = short_task_alias(&task.canonical_task_id) {
            aliases.insert(short, task.canonical_task_id.clone());
        }
    }
    for task in &mut tasks {
        task.dependency_ids = resolve_task_references(
            &task.dependency_ids,
            &aliases,
            &task.source_path,
            "dependency",
        )?;
        for dependency in &mut task.dependencies {
            dependency.task_id = resolve_task_references(
                std::slice::from_ref(&dependency.task_id),
                &aliases,
                &task.source_path,
                "dependency",
            )?
            .remove(0);
        }
        task.dependencies.sort();
        task.dependencies.dedup();
        task.blocks_ids =
            resolve_task_references(&task.blocks_ids, &aliases, &task.source_path, "blocks")?;
    }
    tasks.sort_by(|left, right| left.canonical_task_id.cmp(&right.canonical_task_id));
    Ok(tasks)
}

fn skeleton_from_tasks(
    tasks: &[WorkflowV2TaskUniverseTask],
    acceptance_digest: String,
) -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest,
        tasks: tasks
            .iter()
            .map(|task| FrozenTask {
                task_id: task.canonical_task_id.clone(),
                file_name: PathBuf::from(&task.source_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| task.source_path.clone()),
                depends_on: task.dependencies.clone(),
                blocks: task.blocks_ids.clone(),
                implements: task.implements.clone(),
                deliverable_contracts: task.deliverable_contracts.clone(),
            })
            .collect(),
    }
}

fn validate_acceptance_phase(
    project_root: &Path,
    tasks_root: &Path,
    contract_path: &Path,
    pin: &AcceptancePin,
) -> Result<(AcceptanceContract, std::collections::BTreeSet<String>)> {
    let contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(contract_path)
            .with_context(|| format!("reading acceptance contract {}", contract_path.display()))?,
    )
    .with_context(|| {
        format!(
            "acceptance contract {} is malformed",
            contract_path.display()
        )
    })?;
    let prd_path = {
        let path = PathBuf::from(&contract.prd.path);
        if path.is_absolute() {
            path
        } else {
            project_root.join(path)
        }
    };
    let prd_bytes = std::fs::read(&prd_path)
        .with_context(|| format!("reading frozen PRD {}", prd_path.display()))?;
    let actual_prd_digest = content_digest(&prd_bytes);
    if actual_prd_digest != contract.prd.digest {
        return Err(anyhow::anyhow!(
            "PRD digest mismatch for {}: contract expected {}, actual {}; restore the frozen PRD or re-run workflow freeze-acceptance",
            prd_path.display(),
            contract.prd.digest,
            actual_prd_digest
        ));
    }
    let prd = String::from_utf8(prd_bytes)
        .with_context(|| format!("frozen PRD {} is not UTF-8", prd_path.display()))?;
    let expected = archon_workflow::obligation_ids::acceptance_ids(&prd);
    let validated = validate_acceptance_bundle(tasks_root, Some(pin), &expected)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let obligations = archon_workflow::obligation_ids::obligation_ids(&prd);
    Ok((validated, obligations))
}

fn read_draft_skeleton(path: &Path) -> Result<TaskSkeleton> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading draft skeleton {}", path.display()))?,
    )
    .with_context(|| {
        format!(
            "draft skeleton {} is malformed; repair its JSON or replace it before running workflow freeze-skeleton",
            path.display()
        )
    })
}

fn append_predecessor_finding(
    mode: GateMode,
    label: &str,
    stamp: &archon_workflow::task_set_contract::FreezeGateStamp,
    blockers: &mut Vec<String>,
    inherited_blockers: &mut std::collections::BTreeSet<String>,
) {
    if stamp.finding_count == 0 {
        return;
    }
    let mode_label = if mode == GateMode::Enforce {
        format!(" was minted in {:?} mode", stamp.mode)
    } else {
        String::new()
    };
    let text = format!(
        "predecessor {label} freeze{mode_label} carries {} policy finding(s); re-freeze under enforce and resolve every named finding before continuing",
        stamp.finding_count
    );
    inherited_blockers.insert(text.clone());
    blockers.push(text);
}

fn append_edge_analysis(skeleton: &TaskSkeleton, report: &mut String, blockers: &mut Vec<String>) {
    let edge_analysis = analyze_task_set_edges(skeleton);
    blockers.extend(
        edge_analysis
            .blockers
            .iter()
            .map(|finding| format!("{}: {}", finding.field, finding.message)),
    );
    if edge_analysis.information.is_empty() {
        report.push_str("  dependency contracts: no informational findings\n");
    } else {
        report.push_str("  dependency contract information (non-blocking):\n");
        for finding in &edge_analysis.information {
            report.push_str(&format!("    {}: {}\n", finding.field, finding.message));
        }
    }
}

fn read_pin(path: &Path) -> Result<AcceptancePin> {
    let bytes = std::fs::read(path).with_context(|| {
        format!(
            "required task-set pin {} could not be read; re-run workflow freeze-acceptance and workflow freeze-skeleton",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "task-set pin {} is malformed or unstamped; re-run workflow freeze-acceptance and workflow freeze-skeleton with the current binary",
            path.display()
        )
    })
}

fn finish(
    mut report: String,
    blockers: Vec<String>,
    inherited_blockers: std::collections::BTreeSet<String>,
) -> TaskSetFreezeLint {
    for finding in &blockers {
        report.push_str(&format!("  POLICY FINDING: {finding}\n"));
    }
    TaskSetFreezeLint {
        report,
        blockers,
        inherited_blockers,
    }
}
