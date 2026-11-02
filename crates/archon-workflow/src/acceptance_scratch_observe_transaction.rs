use super::*;

/// The caller supplies an integrity-validated pinned contract and chain digest.
/// Authorization is repeated here before creating scratch or spawning children.
pub async fn observe_commands(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> WorkflowResult<ObservationResult> {
    observe_commands_cancellable(
        policy,
        commit,
        contract,
        chain_digest,
        refs,
        evidence,
        ObserveControl::for_resource_limits(Arc::new(AtomicBool::new(false)), resource_limits),
    )
    .await
}
pub async fn observe_commands_cancellable(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    control: ObserveControl,
) -> WorkflowResult<ObservationResult> {
    observe_commands_hooked(
        policy,
        commit,
        contract,
        chain_digest,
        refs,
        evidence,
        control,
    )
    .await
}
