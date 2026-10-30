//! Observation transaction: resolve, snapshot, execute, clean up, audit.
use super::hooks::{CHECK_DEFERRED, ObserveControl};
#[path = "acceptance_scratch_isolate.rs"]
mod isolate;
use super::process::{CHECK_TIMED_OUT, CheckResult};
use super::*;
use crate::acceptance_world::{FrozenCommandRef, resolve_command};
use crate::task_set_contract::{AcceptanceContract, content_digest};
use isolate::normalized;
use std::sync::{Arc, atomic::AtomicBool};
#[path = "acceptance_scratch_observe_rebuild.rs"]
mod rebuild;
#[path = "acceptance_scratch_observe_result.rs"]
mod result_helpers;
use rebuild::Rebuild;
pub use result_helpers::{AttributedTeardown, ObservationResult};
use result_helpers::{faulted, operational, redacted};
#[path = "acceptance_scratch_observe_execute.rs"]
mod execute;
pub use execute::execute_check_at;

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

#[cfg(test)]
#[path = "acceptance_scratch_setup_fault_tests.rs"]
pub(super) mod setup_fault_tests;

#[path = "acceptance_scratch_observe_transaction.rs"]
mod transaction;
pub use transaction::observe_commands_hooked;
