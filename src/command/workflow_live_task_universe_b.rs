fn canonical_task_id_from_ref(value: &str) -> Option<String> {
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

fn task_id_from_task_path(path: &Path) -> Option<String> {
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

fn short_task_alias(canonical: &str) -> Option<String> {
    let digits = canonical.rsplit('-').next()?;
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("T{digits}"))
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn validate_task_dependency_graph(tasks: &[WorkflowV2TaskUniverseTask]) -> WorkflowResult<()> {
    let graph = tasks
        .iter()
        .map(|task| {
            (
                task.canonical_task_id.clone(),
                task.dependency_ids.clone().into_iter().collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut state = BTreeMap::<String, VisitState>::new();
    for task_id in graph.keys() {
        visit_dependency_node(task_id, &graph, &mut state, &mut Vec::new())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn visit_dependency_node(
    task_id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    state: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> WorkflowResult<()> {
    match state.get(task_id).copied() {
        Some(VisitState::Done) => return Ok(()),
        Some(VisitState::Visiting) => {
            stack.push(task_id.to_string());
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow task dependency cycle detected: {}",
                stack.join(" -> ")
            )));
        }
        None => {}
    }
    state.insert(task_id.to_string(), VisitState::Visiting);
    stack.push(task_id.to_string());
    for dependency in graph.get(task_id).into_iter().flatten() {
        visit_dependency_node(dependency, graph, state, stack)?;
    }
    stack.pop();
    state.insert(task_id.to_string(), VisitState::Done);
    Ok(())
}

#[cfg(test)]
#[path = "workflow_live_task_universe_tests.rs"]
mod tests;
