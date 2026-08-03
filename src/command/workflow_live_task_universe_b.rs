use super::*;

impl WorkflowV2TaskUniverse {
    /// Declared prose across the universe: every task title, then every
    /// acceptance criterion.
    ///
    /// A narrow accessor rather than public fields: the learning-hook
    /// derivation lives outside this module and needs the *content* of the
    /// universe to classify the run, not its structure. Titles come first so a
    /// caller that truncates keeps the strongest signal.
    pub(crate) fn declared_prose(&self) -> Vec<&str> {
        let titles = self.tasks.iter().filter_map(|task| task.title.as_deref());
        let criteria = self
            .tasks
            .iter()
            .flat_map(|task| task.acceptance_criteria.iter().map(String::as_str));
        titles.chain(criteria).collect()
    }
}

pub(super) fn canonical_task_id_from_ref(value: &str) -> Option<String> {
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let first = parts[0];
    let second = parts[1];
    let third = parts[2];
    if first != "TASK"
        || second.is_empty()
        || !second
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        || third.len() != 3
        || !third.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{first}-{second}-{third}"))
}

pub(super) fn task_id_from_task_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    canonical_task_id_from_stem(stem)
}

fn canonical_task_id_from_stem(stem: &str) -> Option<String> {
    let mut parts = stem.split('-');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next()?;
    if first != "TASK"
        || second.is_empty()
        || !second
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        || third.len() != 3
        || !third.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{first}-{second}-{third}"))
}

pub(super) fn short_task_alias(canonical: &str) -> Option<String> {
    let digits = canonical.rsplit('-').next()?;
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("T{digits}"))
}

pub(super) fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Resolve a declared list of task references to canonical ids, or fail naming
/// the file and the unresolved reference.
pub(super) fn resolve_task_references(
    declared: &[String],
    aliases: &BTreeMap<String, String>,
    source_path: &str,
    kind: &str,
) -> WorkflowResult<Vec<String>> {
    let mut resolved = Vec::new();
    for reference in declared {
        let Some(canonical) = aliases.get(reference).cloned() else {
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow has unresolved {kind} reference '{reference}' in {source_path}"
            )));
        };
        resolved.push(canonical);
    }
    Ok(sorted_unique(resolved))
}

/// Fold every `blocks:` declaration into the one dependency graph the runner
/// actually schedules on, and refuse a pair whose two directions disagree.
///
/// `A blocks B` and `B depends_on A` are the same edge written from opposite
/// ends. Only `depends_on` was ever read, so a file that expressed its ordering
/// through `blocks` alone produced no edge and its dependents became eligible
/// immediately. Reconciling is a union: an edge declared from either end is an
/// edge.
///
/// A *contradiction* is the one case a union cannot absorb — the same pair
/// claiming both orders at once (`A blocks B` while `A depends_on B`, or `A` and
/// `B` each claiming to block the other). Folding those would manufacture a
/// two-cycle and the cycle detector would report it as a graph shape rather than
/// as the authoring mistake it is, so they are named here with both files.
pub(super) fn reconcile_blocks_into_dependencies(
    tasks: &mut [WorkflowV2TaskUniverseTask],
) -> WorkflowResult<()> {
    let mut declared_blocks: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut declared_dependencies: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut source_paths: BTreeMap<String, String> = BTreeMap::new();
    for task in tasks.iter() {
        declared_blocks.insert(task.canonical_task_id.clone(), task.blocks_ids.clone());
        declared_dependencies.insert(task.canonical_task_id.clone(), task.dependency_ids.clone());
        source_paths.insert(task.canonical_task_id.clone(), task.source_path.clone());
    }

    for (blocker, blocked_ids) in &declared_blocks {
        for blocked in blocked_ids {
            if blocked == blocker {
                return Err(WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow task {blocker} declares that it blocks itself in {}",
                    source_paths[blocker]
                )));
            }
            // `A blocks B` means B waits for A. `A depends_on B` means A waits
            // for B. Both at once is unsatisfiable.
            if declared_dependencies[blocker].contains(blocked) {
                return Err(WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow task {blocker} both blocks and depends_on {blocked} in {}",
                    source_paths[blocker]
                )));
            }
            if declared_blocks[blocked].contains(blocker) {
                return Err(WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow tasks {blocker} and {blocked} each declare that they block the other ({} / {})",
                    source_paths[blocker], source_paths[blocked]
                )));
            }
        }
    }

    for task in tasks.iter_mut() {
        let inherited = declared_blocks
            .iter()
            .filter(|(_, blocked)| blocked.contains(&task.canonical_task_id))
            .map(|(blocker, _)| blocker.clone());
        task.dependency_ids = sorted_unique(
            task.dependency_ids
                .iter()
                .cloned()
                .chain(inherited)
                .collect(),
        );
    }
    Ok(())
}

/// Refuse a task set whose declared `status:` values cannot be scheduled
/// honestly. See [`task_status`] for the full table of what each value causes.
///
/// Two things are refused here, both because the alternative is a run that
/// looks right:
///
/// - **A status nobody can classify.** Neither default is safe — runnable runs
///   work the author may have cancelled, complete skips work nobody proved —
///   and a typo reaches whichever default in silence.
/// - **`blocked` with no declared dependency.** `blocked` in these files means
///   "waiting on what I depend on", which is why fifteen of the seventeen real
///   tasks carry it and the corpus still runs. A `blocked` task that depends on
///   nothing is making a claim the task set cannot discharge: no other task
///   completing will ever unblock it, so either the dependency is missing or
///   the status is stale, and both are edits to the file named here.
pub(super) fn validate_declared_statuses(
    tasks: &[WorkflowV2TaskUniverseTask],
) -> WorkflowResult<()> {
    for task in tasks {
        let status = task_status::declared_status(task.status.as_deref()).map_err(|detail| {
            WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow task {} declares {detail}, in {}",
                task.canonical_task_id, task.source_path
            ))
        })?;
        if status == task_status::WorkflowV2DeclaredStatus::Blocked
            && task.dependency_ids.is_empty()
        {
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow task {} declares status: blocked but declares \
                 no dependency, in {}; nothing in this task set can ever unblock it, so either \
                 the dependency is missing or the status is stale",
                task.canonical_task_id, task.source_path
            )));
        }
    }
    Ok(())
}

impl WorkflowV2TaskUniverseTask {
    /// Whether the task file declares the work already finished.
    pub(crate) fn declared_status_is_complete(&self) -> bool {
        task_status::declared_status_is_complete(self.status.as_deref())
    }

    /// Whether the task file declares the task blocked by its author.
    pub(crate) fn declared_status_is_blocked(&self) -> bool {
        task_status::declared_status_is_blocked(self.status.as_deref())
    }
}

pub(super) fn validate_task_dependency_graph(
    tasks: &[WorkflowV2TaskUniverseTask],
) -> WorkflowResult<()> {
    let graph = tasks
        .iter()
        .map(|task| {
            (
                task.canonical_task_id.clone(),
                task.dependency_ids.clone().into_iter().collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let source_paths = tasks
        .iter()
        .map(|task| (task.canonical_task_id.clone(), task.source_path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut state = BTreeMap::<String, VisitState>::new();
    for task_id in graph.keys() {
        visit_dependency_node(task_id, &graph, &source_paths, &mut state, &mut Vec::new())?;
    }
    Ok(())
}

/// The cycle, from its first repeated task, with the file each task was
/// declared in.
///
/// Every other failure in this module names the offending file — the unresolved
/// reference, both `blocks` contradictions — because a task id is not something
/// a reader can open. A cycle was the one exception, and across seventeen files
/// it left the reader mapping ids back to filenames by hand.
///
/// The walk prefix is dropped rather than printed. A node the search merely
/// passed through on the way in is not on the cycle, and naming its file would
/// send the reader to a file that needs no edit.
fn cycle_description(stack: &[String], source_paths: &BTreeMap<String, String>) -> String {
    let Some(repeated) = stack.last() else {
        return String::new();
    };
    let start = stack
        .iter()
        .position(|entry| entry == repeated)
        .unwrap_or_default();
    stack[start..]
        .iter()
        .map(|task_id| match source_paths.get(task_id) {
            Some(path) => format!("{task_id} ({path})"),
            None => task_id.clone(),
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn visit_dependency_node(
    task_id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    source_paths: &BTreeMap<String, String>,
    state: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> WorkflowResult<()> {
    match state.get(task_id).copied() {
        Some(VisitState::Done) => return Ok(()),
        Some(VisitState::Visiting) => {
            stack.push(task_id.to_string());
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow task dependency cycle detected: {}",
                cycle_description(stack, source_paths)
            )));
        }
        None => {}
    }
    state.insert(task_id.to_string(), VisitState::Visiting);
    stack.push(task_id.to_string());
    for dependency in graph.get(task_id).into_iter().flatten() {
        visit_dependency_node(dependency, graph, source_paths, state, stack)?;
    }
    stack.pop();
    state.insert(task_id.to_string(), VisitState::Done);
    Ok(())
}

#[cfg(test)]
#[path = "workflow_live_task_universe_tests.rs"]
mod tests;

/// The cycle diagnostic's file paths, and everything `status:` now causes.
///
/// A separate file from `tests` only because each source file in this tree is
/// held under a 500-line ceiling.
#[cfg(test)]
#[path = "workflow_live_task_status_tests.rs"]
mod status_tests;

/// Declaration-driven capability merging, and the runtime-genericity gate.
///
/// A separate file from `tests` only because each source file in this tree is
/// held under a 500-line ceiling.
#[cfg(test)]
#[path = "workflow_live_task_universe_capability_tests.rs"]
mod capability_and_genericity_tests;
