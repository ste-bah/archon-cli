//! The directories a workflow agent may write, named by the HOST.
//!
//! ## Why the host names them and the agent never does
//!
//! Write confinement can be built two ways. A path allowlist lets the agent
//! name any path and checks the result; a host-mediated channel lets the agent
//! name only a leaf and joins it to a directory the host owns. The second is
//! structurally stronger — a name cannot escape a directory it never mentions —
//! and it is the design this module takes as far as it goes here.
//!
//! It does not go all the way, and the reason is worth stating rather than
//! glossing. A workflow implementation agent edits source files it discovered
//! at runtime, whose names no host knows in advance; reducing its writes to
//! "one filename in one host directory" would not confine it, it would give it
//! a second, narrower way to write while `Write`, `Edit` and `ApplyPatch` kept
//! the first. A mechanism present and unused is precisely the defect this work
//! exists to remove, so the channel is not built for the general write path.
//!
//! What is taken from that design is the part that transfers: **the agent never
//! names a root**. It supplies paths, but every directory those paths are
//! measured against is computed here, from what the host already resolved
//! before the agent was spawned. There is no field on any agent-facing request
//! that widens this set, and no prompt text that reaches it. That closes the
//! traversal question — `../..` cannot reach outside a set it cannot influence
//! — and leaves the symlink question to the write guard, which refuses links
//! rather than resolving them (see `archon_tools::path_guard_symlink`).
//!
//! ## Why the set is the project root, not the working directory
//!
//! An earlier attempt confined a workflow agent to the tree it runs in. That is
//! wrong here and was caught before it shipped: the deliverable of a live
//! workflow is routinely a registry under a project directory that is not the
//! repository the agent has checked out, and is frequently not a git repository
//! at all. Confining to the workspace refuses the single write the task exists
//! to make, hours into an unattended run, with the refusal buried in a
//! transcript. Trading a silent escape for a silent blockage is the worse of
//! the two.
//!
//! So the set is what the host already computed for the artifact contract: the
//! project root that `.archon/…` deliverables hang off, and the repository root
//! where source lives when that is a different tree. Both are absolute and both
//! were resolved before any agent saw them.

use std::path::Path;

use super::WorkflowV2ProjectArtifactContext;

/// The absolute directories this call's agent may write, or empty for "the host
/// declared nothing, so there is nothing to enforce".
///
/// Empty is a real answer and not a failure. A run whose host resolved no
/// project root has told us nothing about where its deliverables live, and
/// inventing a root from the working directory is exactly the blockage
/// described above. The caller logs the gap rather than guessing at it.
///
/// `repository_root` is the call's own, which takes precedence over the copy on
/// the artifact context: the context's is documented as existence-checks-only
/// and may be absent on calls where the request's is set.
pub fn declared_write_roots(
    artifacts: &WorkflowV2ProjectArtifactContext,
    repository_root: Option<&str>,
) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();

    // The project root subsumes `artifact_roots`, `artifact_paths` and
    // `directory_artifacts`, every one of which is stored relative to it — see
    // `artifact_roots_for_run`, which produces `.archon/artifacts` and friends.
    // Listing them separately would add entries that resolve to the same tree
    // and one more place for the two lists to disagree.
    push_absolute(&mut roots, artifacts.project_root.as_deref());

    // Where source lives, when the workflow targets a tree other than the
    // project's own. An implementation agent's entire job is editing it.
    push_absolute(&mut roots, repository_root);
    push_absolute(&mut roots, artifacts.repository_root.as_deref());

    // Evidence the run writes about its own branches. Included only when it is
    // absolute; a relative value is already inside the project root.
    push_absolute(&mut roots, artifacts.branch_evidence_root.as_deref());

    roots
}

/// Add `candidate` if it is a usable absolute directory and not already present.
///
/// Relative entries are dropped rather than joined onto the project root. Every
/// relative value in this context is by construction *already* under the
/// project root, so joining would restate a root already in the list, while a
/// relative value that is not — a bug upstream — would silently widen the set
/// by being resolved against whatever this process's working directory happens
/// to be. Dropping can only narrow, and narrowing is visible: the agent is
/// refused and says so.
fn push_absolute(roots: &mut Vec<String>, candidate: Option<&str>) {
    let Some(candidate) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if !Path::new(candidate).is_absolute() {
        return;
    }
    if roots.iter().any(|existing| existing == candidate) {
        return;
    }
    roots.push(candidate.to_string());
}

#[cfg(test)]
#[path = "project_artifact_write_roots_tests.rs"]
mod tests;
