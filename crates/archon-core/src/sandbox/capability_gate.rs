//! The one gating decision every isolation backend shares (#201 Phase 3).
//!
//! Docker, ssh and openshell used to carry three near-identical name
//! allowlists, each ending in a catch-all that denied everything unlisted. The
//! decision does not actually differ between them — what differs is which
//! seams they implement — so it lives here once and is keyed on the class the
//! tool declares.

use archon_permissions::{ToolCapability, WorldReach};

/// Why the gate allowed a class, in the gate's own words.
///
/// The `Ok` was empty, which left the one caller that has to *explain* the
/// decision — `archon sandbox explain` — nothing to quote and no choice but to
/// compose a second account of these arms. Composing one gets three of the five
/// wrong: `HostLocal` is allowed because nothing is relocated, `ControlPlane`
/// because a child comes back through this gate rather than because a seam
/// carries it, and `Terminal` because the answer is not this function's to
/// give. Carrying the reason out with the verdict is what stops a caller
/// inventing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityAllowance(&'static str);

impl CapabilityAllowance {
    #[must_use]
    pub fn reason(self) -> &'static str {
        self.0
    }
}

/// Decide whether a backend that owns an execution world can host `tool`.
///
/// `backend` names the caller for the denial message; it never changes the
/// outcome. Every arm is a statement about a seam: allowed means the backend
/// has a way to put the work in its world, denied means it does not and would
/// otherwise silently do the work on the host.
///
/// Public because `archon sandbox explain` has to report this decision rather
/// than describe it. An explanation assembled from a second copy of these arms
/// is a claim about the gate that nothing keeps true.
pub fn check_capability(
    backend: &str,
    tool: &str,
    capability: ToolCapability,
) -> Result<CapabilityAllowance, String> {
    match capability {
        ToolCapability::HostLocal => Ok(CapabilityAllowance(
            "this tool touches only Archon's own state, which is in the same place whatever the \
             world is, so the sandbox has nothing to relocate",
        )),

        ToolCapability::WorldBound(WorldReach::Execution) => Ok(CapabilityAllowance(
            "`execute_bash` is the seam every backend implements, so the work lands in the \
             backend's world rather than on the host",
        )),

        // #201 Phase 2 made `ToolContext::fs` the backend's own: docker
        // translates against its bind mount, an ssh or openshell remote
        // workspace is reached over the same transport `execute_bash` uses, and
        // the mirror modes are the host tree by definition. Closing that is
        // what let this arm open.
        ToolCapability::WorldBound(WorldReach::FileRead | WorldReach::FileWrite) => {
            Ok(CapabilityAllowance(
                "`ToolContext::fs` is the backend's own filesystem, so the read or write lands in \
                 the world the shell sees rather than on the host behind its back",
            ))
        }

        // Deciding it here instead would have to guess which backend has a
        // session, and would refuse the two that do.
        ToolCapability::WorldBound(WorldReach::Terminal) => Ok(CapabilityAllowance(
            "`SandboxBackend::terminal` is a seam too, so this gate lets the class through and the \
             backend gives the specific answer; which backends actually have a session to attach a \
             TTY to is `terminal()`'s answer, not this one's",
        )),

        // A host PTY, a host language server, a directly spawned subprocess:
        // nothing routes these through the backend, so running one under an
        // active sandbox is a bypass, not an escape hatch.
        ToolCapability::WorldBound(WorldReach::HostHandle) => Err(format!(
            "{backend} sandbox: {tool} reaches the execution world through a host handle the \
             sandbox cannot redirect, so it would run outside the sandbox"
        )),

        ToolCapability::Egress => Err(format!(
            "{backend} sandbox: {tool} leaves the machine, and host-side network access is not \
             supported under isolation"
        )),

        // A child is built from its parent's context
        // (`build_child_tool_context`): the same backend `Arc`, and the
        // parent's filesystem rerooted at the child's own working directory.
        // That was the thing this arm was waiting on, and #201 Phase 4 proves
        // it end to end under docker.
        ToolCapability::ControlPlane => Ok(CapabilityAllowance(
            "a spawned child is built from this backend and its filesystem, and every tool the \
             child calls comes back through this gate, so spawning stays inside the boundary \
             instead of escaping it",
        )),
    }
}

#[cfg(test)]
#[path = "capability_gate_tests.rs"]
mod tests;
