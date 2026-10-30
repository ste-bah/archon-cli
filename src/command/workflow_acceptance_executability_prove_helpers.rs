use super::*;
use crate::command::workflow_task_set::passability::evidence::Redactor;

impl HostProbe {
    /// The environment this probe's site gives every check, and the host
    /// variable names it forwards (Issue 544: the reverify judge's evidence
    /// is redacted with them).
    pub(crate) fn site_environment(&self) -> (BTreeMap<String, String>, Vec<String>) {
        super::super::sites::site_environment(self)
    }

    /// Issue 544: one redactor for every value the host binds into a
    /// check's environment: the site policy's own values, the host values
    /// it forwards, the host's credentials, and every URL password among
    /// them. All mutation evidence passes through it before any line is
    /// left out or any excerpt is cut.
    pub(in crate::command::workflow_task_set::executability) fn output_redactor(&self) -> Redactor {
        let (environment, forwarded) = self.site_environment();
        Redactor::for_environment(environment.into_iter().chain(self.host.clone()), &forwarded)
    }
}

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
        // Issue 495: state only what the run showed. The caller names any
        // environment repair the host really made in `reason`.
        return Some(MutationFinding::AuthorFinding(format!(
            "{passed_there}; with the data it names moved aside, the check stopped at a setup failure: {reason}; choose a path available in the selected root and make the check fail on its own assertion"
        )));
    }
    // Restoration can establish that a moved path was present again, but
    // cannot attribute the writer. A check's own assertion failure is still
    // valid mutation evidence even if another path reappeared in the copy.
    None
}

pub(in crate::command::workflow_task_set::executability) fn unattributed_recreation_finding(
    baseline: &str,
) -> String {
    format!(
        "{baseline}; the check passed, but a moved input was present again after mutation, or its parent directory metadata changed while it was moved aside; the observed change does not identify who wrote the input (its writer is unknown), so this run does not prove that the check depends on the moved input"
    )
}

pub(in crate::command::workflow_task_set::executability) fn recreated_inputs(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> Vec<String> {
    markers.recreated(result, inputs).into_iter().collect()
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
        "{passed_there}; it recreated {} while running, so that data is not evidence; use a movable input that exists under the selected root before the run and make the check fail whenever the deciding test fails",
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

/// Issue 495: a setup-failure reason that names the environment repairs the
/// host really made before the run, and none when it made none.
pub(in crate::command::workflow_task_set::executability) fn setup_reason_with_repairs(
    reason: String,
    repairs: &[String],
) -> String {
    if repairs.is_empty() {
        return reason;
    }
    format!(
        "{reason} (the host had repaired its environment before this run: {})",
        repairs.join("; then ")
    )
}

/// Issue 492: what the run with inputs moved aside showed, for every
/// verdict reached from it: the state of each input, the inputs moved, the
/// exit status, and the check's own output. Mutation protocol lines are
/// left out of the output (their facts are the states); the output is
/// redacted before its excerpt is cut.
pub(in crate::command::workflow_task_set::executability) fn mutation_observation(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
    redactor: &Redactor,
) -> String {
    // Credentials are masked in the whole stream before any line is left
    // out, so a value spanning lines is masked whole.
    let own = |bytes: &[u8]| -> Vec<u8> {
        redactor
            .redact(&String::from_utf8_lossy(bytes))
            .lines()
            .filter(|line| {
                !(line.starts_with("archon-mutation-") && line.contains(markers.nonce()))
            })
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    };
    let mut check_only = result.clone();
    check_only.stdout = own(&result.stdout);
    check_only.stderr = own(&result.stderr);
    format!(
        "inputs={:?}; moved={:?}; mutated_exit_code={:?}; status/signal: {}; output_excerpt={:?}",
        markers.move_states(result, inputs),
        markers.moved(result, inputs),
        result.exit_code,
        describe_status(markers.status(result)),
        super::output_evidence_with(&check_only, redactor)
    )
}

/// `finding` with the observation it was reached from.
pub(in crate::command::workflow_task_set::executability) fn observed(
    finding: String,
    observation: &str,
) -> String {
    format!("{finding}; mutation observation: {observation}")
}

/// The last non-empty line of the check's own stderr (mutation protocol
/// lines left out), quoted verbatim; `(none)` when empty. The whole stderr
/// is redacted first, then the line is picked: a credential that spans
/// lines is masked whole, never cut so that its tail survives.
pub(in crate::command::workflow_task_set::executability) fn last_stderr_line(
    result: &CheckResult,
    markers: &mutation::Markers,
    redactor: &Redactor,
) -> String {
    let stderr = archon_observability::redaction::redact_secret_values(
        &redactor.redact(&String::from_utf8_lossy(&result.stderr)),
    );
    stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !(line.starts_with("archon-mutation-") && line.contains(markers.nonce())))
        .last()
        .map_or_else(|| "(none)".to_string(), |line| format!("{line:?}"))
}

/// Issue 328: whether a mutated run crashed in the check's own code, judged
/// on `view`'s streams (the raw capture, or a stored copy) against the
/// check's original command: the executor classified the generated mutation
/// wrapper, whose shell text is not the author's inline program, so the
/// classification is dropped and the stderr, with the host's mutation
/// protocol lines left out, is classified again.
pub(in crate::command::workflow_task_set::executability) fn mutation_crashed(
    contract: &AcceptanceContract,
    view: &CheckResult,
    markers: &mutation::Markers,
) -> bool {
    let mut view = view.clone();
    view.classification = None;
    let protocol = [
        markers.moved_prefix(),
        markers.move_state_prefix(),
        markers.generated_prefix(),
        markers.recreated_prefix(),
        format!("archon-mutation-check-recreated-{}: ", markers.nonce()),
        format!("archon-mutation-guard-{}:", markers.nonce()),
    ];
    view.stderr = String::from_utf8_lossy(&view.stderr)
        .lines()
        .filter(|line| !protocol.iter().any(|prefix| line.starts_with(prefix)))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    !super::super::crash_findings(contract, [&view]).is_empty()
}
