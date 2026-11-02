use super::*;
/// Settle check `id`, which failed on `commit` without a verdict (`why`):
/// unproven the first time (the strike is saved, which a staged freeze
/// counts as progress, so it is retried); the author's finding when an
/// earlier attempt already saw it fail so.
pub(in super::super) fn settle(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    result: &CheckResult,
    why: &str,
    at: &Context,
) -> Option<String> {
    settle_as(
        probe,
        commit,
        contract,
        result,
        (why, UnprovenCause::Host),
        at,
    )
}
