use super::*;

fn path(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) -> PathBuf {
    stall_strike(probe, commit, contract, id).with_extension("mutation.strike")
}

pub(crate) fn settle_mutation_timeout(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    moved: &[String],
    diagnostics: &str,
) -> Option<String> {
    let why = format!(
        "no output or process-tree activity for {}s",
        probe.check_bound_secs()
    );
    let moved = if moved.is_empty() {
        "no recorded inputs".into()
    } else {
        moved.join(", ")
    };
    let suffix = if diagnostics.is_empty() {
        String::new()
    } else {
        format!("; {diagnostics}")
    };
    struck(
        probe,
        &path(probe, commit, contract, id),
        id,
        (&why, UnprovenCause::Host),
        Credit::None,
        || {
            (
                format!(
                    "check '{id}': with {moved} moved aside it made no progress twice: {why}, and so again after a resume; it must fail, not wait, when an input is missing{suffix}"
                ),
                format!(
                    "unproven (timed out): with {moved} moved aside, {why}; the host may be at fault{suffix}"
                ),
            )
        },
    )
}

pub(crate) fn ran_mutation(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
) {
    for phase in ["move", "fault"] {
        let strike = mutation_strike(probe, commit, contract, id, phase);
        if let Err(error) = archon_workflow::stage_write::remove_file(&strike) {
            probe.unproven_error(
                id,
                error.into(),
                format!("mutation strike {} could not be cleared", strike.display()),
            );
        }
    }
    let path = path(probe, commit, contract, id);
    if let Err(error) = archon_workflow::stage_write::remove_file(&path) {
        probe.unproven_error(
            id,
            error.into(),
            format!("stall strike {} could not be cleared", path.display()),
        );
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_silent_mutation_tests.rs"]
mod tests;

fn mutation_strike(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    phase: &str,
) -> PathBuf {
    let path = strike(probe, commit, contract, id);
    if phase == "baseline" {
        path
    } else {
        path.with_extension(format!("mutation-{phase}.strike"))
    }
}

pub(crate) fn strike_mutation(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    phase: &str,
    why: (&str, UnprovenCause),
    texts: impl FnOnce() -> (String, String),
) -> Option<String> {
    struck(
        probe,
        &mutation_strike(probe, commit, contract, id, phase),
        id,
        why,
        if phase == "baseline" {
            Credit::Saved
        } else {
            Credit::None
        },
        texts,
    )
}
