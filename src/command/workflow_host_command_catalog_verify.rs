//! The two frozen-chain verification capabilities (Issue-46).
//!
//! A launch on a task root that already holds a frozen acceptance contract
//! or skeleton skips re-authoring it. The script still needs a committed host
//! outcome for each skipped stage — the evidence `reconcile` and the final
//! report carry — so it asks the host to verify the frozen artifact in place.
//! The child publishes nothing but its gate envelope; the parent's
//! postcondition is the freeze command's own (bundle, lock, pin, chain), and
//! `verify-frozen-skeleton` reports the frozen subjects exactly as
//! `freeze-skeleton` does.
use archon_workflow::{CommandCapability, EnvironmentProfileId, RemediationScope, StdinDelivery};

use super::{MIB, capability};

pub(super) fn verify_frozen_chain_capabilities() -> [CommandCapability; 2] {
    [
        verify_capability("verify-frozen-acceptance", "acceptance"),
        verify_capability("verify-frozen-skeleton", "skeleton"),
    ]
}

fn verify_capability(id: &str, stage: &str) -> CommandCapability {
    capability(
        id,
        &[
            "workflow",
            "verify-frozen-chain",
            "--stage",
            stage,
            "--tasks",
            "{TASK_ROOT}",
            "--prd",
            "{PRD_PATH}",
            "--gate-envelope",
            "{GATE_ENVELOPE}",
            "--call-id",
            "{CALL_ID}",
        ],
        StdinDelivery::None,
        EnvironmentProfileId::None,
        300,
        0,
        MIB,
        MIB,
        &["{GATE_ENVELOPE}"],
        // Verification either holds or fails operationally; there is no
        // candidate to send a finding back to.
        &[RemediationScope::Operational],
    )
}
