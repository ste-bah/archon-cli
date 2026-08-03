use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use archon_topology::ir::{GraphBudget, GraphOrigin, NodeRole, TaskGraph, TaskNode, WriteTarget};
use archon_workflow::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[path = "workflow_live_task_status.rs"]
pub(crate) mod task_status;

#[path = "workflow_live_task_universe_a.rs"]
mod workflow_live_task_universe_a;
pub(crate) use workflow_live_task_universe_a::*;
#[path = "workflow_live_task_universe_b.rs"]
mod workflow_live_task_universe_b;
pub(crate) use workflow_live_task_universe_b::*;

#[path = "workflow_live_task_universe_parsing.rs"]
pub(super) mod parsing;
use parsing::{merge_project_capabilities, parse_task_file};

/// Lower a directory of decomposed-PRD `TASK-*.md` files into the topology IR
/// so the milestone 4 lints can run against it.
///
/// # Why this exists rather than reusing the run path
///
/// [`extract_task_universe_for_generated_run`] is gated on the run's *task
/// text* mentioning a decomposed PRD, because it is part of planning a run.
/// Linting is not planning: the caller has already named the directory, and
/// nothing about linting should require a workflow to be started. So this walks
/// the directory directly and reuses the same parser, alias resolution,
/// `blocks` reconciliation, and dependency validation — an unparseable task
/// file, an unknown dependency, or a cycle is an error here exactly as it is
/// there.
///
/// # The dataflow contract, and what it is read from
///
/// - **Production** is `deliverable_contracts[].artifact_path` — what the task
///   is contracted to produce — plus every concrete path named under
///   `## Files Expected to Change`.
/// - **Consumption** is every artifact path *some other task produces* that
///   appears verbatim anywhere in this task's file, plus the contract's own
///   declared input paths (`registry_path`, `instance_source_path`).
///
/// `files_expected_to_change` is deliberately **not** treated as consumption.
/// In practice it is near-identical boilerplate across the tasks of one PRD —
/// the same "likely anchors" list repeated — so counting it as consumption
/// would make every task appear to consume every other task's files and the
/// fake-edge lint would report nothing, ever. It stays on the production side,
/// where the overlap it creates is a genuine write-conflict signal.
pub(crate) fn task_graph_from_root(root: &Path) -> WorkflowResult<TaskGraph> {
    let mut parsed = Vec::new();
    for path in task_files_under(root)? {
        let raw = fs::read_to_string(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        let mut task = parse_task_file(&path, &raw)?;
        merge_project_capabilities(&mut task, &path)?;
        parsed.push((task, raw));
    }
    if parsed.is_empty() {
        return Err(WorkflowError::SpecInvalid(format!(
            "no TASK-*.md files found under {}",
            root.display()
        )));
    }

    let mut aliases = BTreeMap::new();
    for (task, _) in &parsed {
        aliases.insert(
            task.canonical_task_id.clone(),
            task.canonical_task_id.clone(),
        );
        if let Some(short) = short_task_alias(&task.canonical_task_id) {
            aliases.insert(short, task.canonical_task_id.clone());
        }
    }
    for (task, _) in &mut parsed {
        task.dependency_ids = resolve_task_references(
            &task.dependency_ids,
            &aliases,
            &task.source_path,
            "dependency",
        )?;
        task.blocks_ids =
            resolve_task_references(&task.blocks_ids, &aliases, &task.source_path, "blocks")?;
    }
    parsed.sort_by(|left, right| left.0.canonical_task_id.cmp(&right.0.canonical_task_id));

    let mut tasks: Vec<WorkflowV2TaskUniverseTask> =
        parsed.iter().map(|(task, _)| task.clone()).collect();
    reconcile_blocks_into_dependencies(&mut tasks)?;
    validate_task_dependency_graph(&tasks)?;
    validate_declared_statuses(&tasks)?;

    // Artifact path → the tasks contracted to produce it. Several tasks
    // legitimately produce the same templated path (`…/<dataset-id>/…`), which
    // is why this is a set rather than a single owner.
    let mut producers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for task in &tasks {
        for contract in &task.deliverable_contracts {
            let path = normalized_target(&contract.artifact_path);
            if path.is_empty() {
                continue;
            }
            producers
                .entry(path)
                .or_default()
                .insert(task.canonical_task_id.clone());
        }
    }

    let raw_by_id: BTreeMap<&str, &str> = parsed
        .iter()
        .map(|(task, raw)| (task.canonical_task_id.as_str(), raw.as_str()))
        .collect();

    let nodes = tasks
        .iter()
        .map(|task| {
            let raw = raw_by_id
                .get(task.canonical_task_id.as_str())
                .copied()
                .unwrap_or_default();
            TaskNode {
                depends_on: task.dependency_ids.clone(),
                writes: task_production(task),
                reads: task_consumption(task, raw, &producers),
                ..TaskNode::new(&task.canonical_task_id, NodeRole::Work)
            }
        })
        .collect();

    Ok(TaskGraph {
        id: root.display().to_string(),
        // A decomposed-PRD task set is the declared input to a workflow run, so
        // it lints as one. There is no run id yet, so the directory names it.
        origin: GraphOrigin::Workflow {
            run_id: root.display().to_string(),
        },
        nodes,
        budget: GraphBudget::default(),
    })
}

/// What one task file claims about the PRD it came from.
///
/// The topology IR carries no requirement claims — it is a dependency and
/// dataflow graph — so the coverage lint reads them from here rather than
/// re-deriving a second, drifting notion of "what this task file said".
pub(crate) struct TaskRequirementClaims {
    pub(crate) task_id: String,
    pub(crate) source_path: String,
    pub(crate) implements: Vec<String>,
}

/// Every task file's `implements:` claims, parsed by the same parser a run uses.
///
/// Rejects exactly what a run rejects: a file with no YAML block, an
/// unparseable one, or one missing a required key — `implements` among them —
/// is an error naming the file. The lint that calls this has already loaded the
/// same directory as a graph, so this cannot be the first thing to fail.
pub(crate) fn task_requirement_claims_from_root(
    root: &Path,
) -> WorkflowResult<Vec<TaskRequirementClaims>> {
    let mut claims = Vec::new();
    for path in task_files_under(root)? {
        let raw = fs::read_to_string(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        let task = parse_task_file(&path, &raw)?;
        claims.push(TaskRequirementClaims {
            task_id: task.canonical_task_id,
            source_path: task.source_path,
            implements: task.implements,
        });
    }
    Ok(claims)
}

/// Artifacts the task is contracted to produce, plus the concrete files it
/// declares it will change.
fn task_production(task: &WorkflowV2TaskUniverseTask) -> Vec<WriteTarget> {
    let mut targets: BTreeSet<WriteTarget> = task
        .deliverable_contracts
        .iter()
        .map(|contract| normalized_target(&contract.artifact_path))
        .filter(|path| !path.is_empty())
        .map(WriteTarget::Artifact)
        .collect();
    for item in &task.files_expected_to_change {
        for path in declared_paths_in(item) {
            targets.insert(WriteTarget::Path(path));
        }
    }
    targets.into_iter().collect()
}

/// Artifacts the task declares it consumes.
///
/// Two sources, both declarations rather than inferences: the contract's own
/// input path fields, and any *other* task's contracted artifact path named
/// verbatim in this task's file.
fn task_consumption(
    task: &WorkflowV2TaskUniverseTask,
    raw: &str,
    producers: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<WriteTarget> {
    let own: BTreeSet<String> = task
        .deliverable_contracts
        .iter()
        .map(|contract| normalized_target(&contract.artifact_path))
        .collect();

    let mut targets: BTreeSet<WriteTarget> = BTreeSet::new();
    for contract in &task.deliverable_contracts {
        for declared in [
            contract.registry_path.as_deref(),
            contract.instance_source_path.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let path = normalized_target(declared);
            if !path.is_empty() && !own.contains(&path) {
                targets.insert(WriteTarget::Artifact(path));
            }
        }
    }

    let normalized_raw = declaration_text(raw).replace('\\', "/");
    for (path, owners) in producers {
        if own.contains(path) {
            continue;
        }
        if owners.len() == 1 && owners.contains(&task.canonical_task_id) {
            continue;
        }
        if normalized_raw.contains(path.as_str()) {
            targets.insert(WriteTarget::Artifact(path.clone()));
        }
    }
    targets.into_iter().collect()
}

/// Marker delimiting machine-appended prior-run output inside a task file.
const PRIOR_RUN_BEGIN: &str = "<!-- PRIOR-RUN-FINDINGS:BEGIN -->";
const PRIOR_RUN_END: &str = "<!-- PRIOR-RUN-FINDINGS:END -->";

/// The part of a task file that is the author's declaration.
///
/// A previous run's findings are appended into the file between explicit
/// markers. That block quotes evidence verbatim — including absolute paths to
/// artifacts the task never claimed to read — so scanning it for artifact
/// references attributes a *reviewer's* citation to the *task author* and
/// invents a dataflow declaration nobody made. This is not hypothetical: it
/// turned a correctly-silent edge into a reported one the first time the lint
/// ran over a real task set, and the regression test for it lives beside the
/// lint command.
fn declaration_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(begin) = rest.find(PRIOR_RUN_BEGIN) {
        out.push_str(&rest[..begin]);
        rest = match rest[begin..].find(PRIOR_RUN_END) {
            Some(end) => &rest[begin + end + PRIOR_RUN_END.len()..],
            // An opened-but-unterminated block runs to end of file; dropping the
            // remainder is the conservative reading, and it under-reports
            // consumption rather than inventing it.
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Path-shaped tokens inside one prose bullet.
///
/// Task files write their file references inside backticks, embedded in
/// sentences ("Likely anchors: `a.rs`, `b.rs`, and the real TUI registry"), so
/// the backtick span is the only reliable delimiter. A bullet with no backticks
/// is taken whole only when the entire bullet is itself a path — otherwise a
/// sentence would be recorded as a filename.
fn declared_paths_in(item: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut spans: Vec<&str> = Vec::new();
    let mut rest = item;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        spans.push(&after[..close]);
        rest = &after[close + 1..];
    }
    if spans.is_empty() {
        spans.push(item);
    }
    for span in spans {
        for token in span.split(&[',', ';'][..]) {
            let path = normalized_target(token);
            if is_path_shaped(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// A token is path-shaped when it has no whitespace and either contains a
/// separator or carries a file extension. Prose fails both.
fn is_path_shaped(token: &str) -> bool {
    if token.is_empty() || token.contains(char::is_whitespace) {
        return false;
    }
    token.contains('/')
        || Path::new(token)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| !extension.is_empty())
}

/// Fold a declared path into the form the exact-string overlap check compares.
///
/// Separator normalization only — no canonicalization, no filesystem access.
/// Templated segments (`<dataset-id>`) are left verbatim: two contracts naming
/// the same template are naming the same artifact family, and rewriting them
/// would lose that.
fn normalized_target(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '`' | '*' | '"' | '\'' | '(' | ')' | '[' | ']'))
        .trim()
        .trim_end_matches(['.', ':'])
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}
