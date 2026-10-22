use super::*;

pub(in crate::command::workflow_task_set::executability) enum MutationFinding {
    HostUnproven(String),
    AuthorFinding(String),
}

pub(in crate::command::workflow_task_set::executability) fn mutation_host_or_author_finding(
    result: &CheckEvidence,
    markers: &mutation::Markers,
    inputs: &[String],
    setup_failure: Option<String>,
    passed_there: &str,
) -> Option<MutationFinding> {
    // The caller supplies `setup_failure` only after the setup-path
    // classifier matched a path in the check text to output reporting that
    // exact path missing from the disposable copy. Still check host-side
    // execution and mutation integrity first: an operational error, guard,
    // signal, or failed restoration must never be attributed to the author.
    if let Some(reason) = mutation_host_failure(result, markers, inputs) {
        return Some(MutationFinding::HostUnproven(reason));
    }
    if let Some(reason) = setup_failure {
        return Some(MutationFinding::AuthorFinding(format!(
            "{passed_there}; the check stopped at a setup failure after the host repaired its environment: {reason}; choose a path available in the selected root and make the check fail on its own assertion"
        )));
    }
    let recreated = recreated_inputs(&result.result, markers, inputs);
    (!recreated.is_empty())
        .then(|| MutationFinding::AuthorFinding(author_recreated_finding(passed_there, &recreated)))
}

pub(in crate::command::workflow_task_set::executability) fn recreated_inputs(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> Vec<String> {
    recreated_paths(&input_states(result, markers, inputs))
        .into_iter()
        .chain(markers.recreated(result, inputs))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn mutation_host_failure(
    result: &CheckEvidence,
    markers: &mutation::Markers,
    inputs: &[String],
) -> Option<String> {
    let states = input_states(&result.result, markers, inputs);
    let status = describe_status(markers.status(&result.result));
    let details = format!(
        "{}; check status/signal: {status}",
        describe_states(&states)
    );
    if result.result.operational_error.is_some() {
        return Some(operational_error_diagnostics(
            &result.result,
            markers,
            inputs,
        ));
    }
    let cause = if markers.guarded(&result.result) {
        Some("the mutation's live-root guard stopped it".to_string())
    } else if let Some(signal) = markers.observed_signal(&result.result) {
        Some(format!("the check wrapper observed signal {signal}"))
    } else {
        let moved = markers.moved(&result.result, inputs);
        let states = markers.states(&result.result, inputs);
        let incomplete_states = moved.iter().any(|path| !states.contains_key(path));
        if incomplete_states {
            Some("the mutation run did not record a state for every moved input".into())
        } else if markers.status(&result.result).is_none() {
            Some("the mutation run did not record its exit status".into())
        } else {
            let unrestored = states
                .iter()
                .any(|(_, state)| state != "restored" && state != "original-recreated");
            unrestored.then(|| "the mutation run left input state unproven".into())
        }
    };
    cause.map(|cause| format!("{cause}; {details}"))
}

pub(in crate::command::workflow_task_set::executability) fn describe_states(
    states: &std::collections::BTreeMap<String, String>,
) -> String {
    format!(
        "input states: {}",
        states
            .iter()
            .map(|(path, state)| format!("{path}={state}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(in crate::command::workflow_task_set::executability) fn input_states(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> std::collections::BTreeMap<String, String> {
    let mut states = markers.states(result, inputs);
    for path in markers.moved(result, inputs) {
        states
            .entry(path)
            .or_insert_with(|| "aside-missing: no restoration state was recorded".into());
    }
    for input in inputs {
        states
            .entry(input.clone())
            .or_insert_with(|| "unreported".into());
    }
    states
}

pub(in crate::command::workflow_task_set::executability) fn all_moved_inputs_restored(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> bool {
    let states = markers.states(result, inputs);
    markers
        .moved(result, inputs)
        .iter()
        .all(|path| states.get(path).is_some_and(|state| state == "restored"))
}

pub(in crate::command::workflow_task_set::executability) fn operational_error_diagnostics(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> String {
    format!(
        "{}; {}; check status/signal: {}",
        result.operational_error.as_deref().unwrap_or_default(),
        describe_states(&input_states(result, markers, inputs)),
        describe_status(markers.status(result))
    )
}

pub(in crate::command::workflow_task_set::executability) fn recreated_paths(
    states: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    states
        .iter()
        .filter_map(|(path, state)| (state == "original-recreated").then_some(path.clone()))
        .collect()
}

pub(in crate::command::workflow_task_set::executability) fn author_recreated_finding(
    passed_there: &str,
    paths: &[String],
) -> String {
    format!(
        "{passed_there}; it recreated {} while running, so that data is not evidence; use a movable input that exists under the selected root before the run and make the check fail on its own assertion when it is moved aside",
        paths.join(", ")
    )
}

pub(in crate::command::workflow_task_set::executability) fn describe_status(
    status: Option<String>,
) -> String {
    match status {
        Some(status) => {
            let code = status
                .split(':')
                .nth(1)
                .and_then(|code| code.parse::<i32>().ok());
            let signal = status.split(':').nth(3).unwrap_or("none");
            if signal != "none" {
                format!(
                    "exit status {}, wrapper-observed signal {signal}",
                    code.unwrap_or_default()
                )
            } else {
                format!(
                    "exit status {}",
                    code.map_or(status, |code| code.to_string())
                )
            }
        }
        None => "signal/exit status unavailable (wrapper interrupted before recording it)".into(),
    }
}
