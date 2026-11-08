//! Origin-aware baseline no-verdict and timeout settlement.
use super::*;

/// [`settle`], recording an unproven check with the cause of `why`.
pub(in super::super) fn settle_as(
    probe: &HostProbe,
    origin: BaselineOrigin,
    commit: &str,
    contract: &AcceptanceContract,
    result: &CheckResult,
    (why, cause): (&str, UnprovenCause),
    at: &Context,
) -> Option<String> {
    let tree = origin.tree_name();
    let id = result.acceptance_id.as_str();
    let short: String = commit.chars().take(12).collect();
    let (environment, forwarded) = super::super::sites::site_environment(probe);
    let redactor = Redactor::for_environment(environment, &forwarded);
    let evidence = program_output(&result.stderr, &redactor, EVIDENCE_BYTES);
    let signature = setup_path_signature(contract, id, result, why, at);
    let retried = match cause {
        UnprovenCause::TransientHost => "the host retries its inconclusive listing probe",
        // Issue 548: nothing in the host adds a program to the site it
        // built (its search path is the configured toolchain path, its
        // cargo home holds only the seed's caches), so a site that lacks
        // what the check runs never comes to provide it, and no retry can
        // prove it. It goes to its author at once, in the same budget, with
        // no strike saved: nothing is held against the author (F6), and no
        // attempt is spent on it.
        UnprovenCause::SiteLacks => {
            return Some(format!(
                "check '{id}': it cannot be proven on the {tree} at {short}: {why}. The host has no repair that makes the check's site provide it, so no retry can prove it; this is not a finding against the criterion. Its stderr there:\n{evidence}\nCheck the criterion with what the site provides, without depending on that program or subcommand"
            ));
        }
        // Issue 557: a program neither the site nor the host has is the
        // author's at once for the same reason (the re-author path already
        // sends it there): no host action makes it available.
        UnprovenCause::ProgramAbsent => {
            return Some(format!(
                "check '{id}': it cannot be proven on the {tree} at {short}: {why}. Neither the check's site nor the host has that program, and no host action makes it available, so no retry can prove it; this is not a finding against the criterion. Its stderr there:\n{evidence}\nCheck the criterion with what the site provides: {PROGRAM_ABSENT_REMEDY}."
            ));
        }
        UnprovenCause::Host | UnprovenCause::PermanentHost => {
            "it is probed again on retry, and goes to its author if it fails so again"
        }
    };
    strike_or_unproven(
        probe,
        commit,
        contract,
        id,
        signature.as_deref(),
        (why, cause),
        || {
            (
                format!(
                    "check '{id}': it cannot be proven on the {tree} at {short}: {why}. It failed so again when the host ran it on that same commit, and the commit never changes, so no retry can prove it. Its stderr there:\n{evidence}\nMake it fail by its own assertion on the pre-implementation tree -- exercise what the implementation must add -- or prove it a regression guard, not through a tool, build or environment it cannot run there"
                ),
                format!(
                    "it failed on the {tree} at {short}, but {why}, so it gave no verdict there and is not proven able to fail; {retried}. Its stderr there:\n{evidence}"
                ),
            )
        },
    )
}

/// Settle a check timeout using a durable per-check stall strike.
#[cfg(all(test, unix))]
pub(in super::super) fn settle_timed_out(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    settle_timed_out_with_diagnostics(
        probe,
        BaselineOrigin::RecordedBase,
        commit,
        contract,
        id,
        "",
    )
}

/// Add run-specific diagnostics without changing the timeout verdict.
pub(in super::super) fn settle_timed_out_with_diagnostics(
    probe: &HostProbe,
    origin: BaselineOrigin,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    diagnostics: &str,
) -> Option<String> {
    let short: String = commit.chars().take(12).collect();
    let tree = origin.tree_name();
    let window = probe.check_bound_secs();
    let why = format!("no output or process-tree activity for {window}s");
    let suffix = if diagnostics.is_empty() {
        String::new()
    } else {
        format!("; {diagnostics}")
    };
    let path = stall_strike(probe, commit, contract, id);
    struck(
        probe,
        &path,
        id,
        (&why, UnprovenCause::Host),
        Credit::None,
        || {
            (
                format!(
                    "check '{id}': it made no progress on the {tree} at {short} twice: {why}, and so again after a resume. Repair or replace it so that it ends by its own assertion{suffix}"
                ),
                format!(
                    "unproven (timed out): {why} on the {tree} at {short}; the run pauses resumably since the host may be at fault. If it stalls again after a resume, it goes to its author{suffix}"
                ),
            )
        },
    )
}
