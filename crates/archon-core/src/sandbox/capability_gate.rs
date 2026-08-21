//! The one gating decision every isolation backend shares (#201 Phase 3).
//!
//! Docker, ssh and openshell used to carry three near-identical name
//! allowlists, each ending in a catch-all that denied everything unlisted. The
//! decision does not actually differ between them — what differs is which
//! seams they implement — so it lives here once and is keyed on the class the
//! tool declares.

use archon_permissions::{ToolCapability, WorldReach};

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
) -> Result<(), String> {
    match capability {
        // Archon's own state is in the same place whatever the world is.
        ToolCapability::HostLocal => Ok(()),

        // `execute_bash` is the seam every backend implements, so this work
        // genuinely lands in the world.
        ToolCapability::WorldBound(WorldReach::Execution) => Ok(()),

        // Both served by `ToolContext::fs`, which #201 Phase 2 made the
        // backend's own: docker translates against its bind mount, an ssh or
        // openshell remote workspace is reached over the same transport
        // `execute_bash` uses, and the mirror modes are the host tree by
        // definition. A write therefore lands in the world the shell sees
        // rather than on the host behind its back, which is the whole reason
        // this arm was closed.
        ToolCapability::WorldBound(WorldReach::FileRead | WorldReach::FileWrite) => Ok(()),

        // `SandboxBackend::terminal` is a seam too, so the class passes here
        // and the backend gives the specific answer: docker attaches a TTY to a
        // container, ssh puts one on the connection it already has, and a
        // backend with no session to attach to refuses by name there. Deciding
        // it here instead would have to guess which, and would refuse the two
        // that work.
        ToolCapability::WorldBound(WorldReach::Terminal) => Ok(()),

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

        // Spawned work stays inside the world rather than escaping it. A child
        // is built from its parent's context (`build_child_tool_context`): the
        // same backend `Arc`, and the parent's filesystem rerooted at the
        // child's own working directory. Every tool the child then calls goes
        // back through this gate on that backend before it runs, so a subagent
        // is not a hole in the boundary — it is another caller of it. That was
        // the thing this arm was waiting on, and #201 Phase 4 proves it end to
        // end under docker.
        ToolCapability::ControlPlane => Ok(()),
    }
}

#[cfg(test)]
#[path = "capability_gate_tests.rs"]
mod tests;
