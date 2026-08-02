//! Invariant 2 — single writer per artifact.
//!
//! A write to a path already claimed by a **concurrently live** node with **no
//! dependency path** between them is blocked. Both qualifiers matter: a node
//! that has finished holds no claim, and two nodes joined by a dependency path
//! are ordered relative to one another and so cannot race.
//!
//! # This extends the write coordinator; it does not reimplement it
//!
//! `archon-workflow`'s `write_coordinator` already answers "do these two write
//! sets overlap?" properly, with a resource-key overlap table covering
//! file/dir/glob combinations
//! (`write_coordinator::write_plan::keys_conflict`). Admission calls that table
//! rather than carrying a second opinion. `archon-topology`'s own
//! `TaskGraph::write_conflicts` uses exact-string overlap and says so — that is
//! the conservative floor available under milestone 1's dependency budget, not
//! a rival algorithm.
//!
//! What admission adds is a hot-path-safe way to *build* a key:
//! `write_plan::resource_key_for_raw_target`, which does the same folding and
//! literal/pattern classification with no filesystem access. `normalize_target`
//! canonicalises and stats every component, and fails outright on a path
//! outside the repository — neither is acceptable on the synchronous critical
//! path of every tool call, and a failure there would have to resolve as either
//! "allow" (unsafe) or "block" (a false positive on every out-of-tree write).
//!
//! # The one place this does not fail open
//!
//! `keys_conflict` treats a **malformed glob as conflicting**
//! (`write_plan.rs`, `glob_match`'s `Err(_) => true`). That is deliberate and
//! it is preserved here. It is not in tension with "never fail closed on a
//! bookkeeping bug": a malformed glob is not missing bookkeeping, it is a
//! present and unreadable claim, and the safe reading of an unreadable claim on
//! a shared resource is that it might cover the resource.
//!
//! # Empty means unknown
//!
//! A tool call that declares no write paths runs no check. Under the
//! unknown-dataflow rule an empty write list means *unknown*, not *nothing*, so
//! there is no claim to compare and nothing to conclude.

use super::LiveTopologyConfig;
use super::state::SessionState;
use super::verdict::{Invariant, Verdict, WriteIntent};
use archon_workflow::write_coordinator::write_plan::resource_key_for_raw_target;

/// Admit a write, claiming its paths when admitted.
pub(super) fn admit_write(
    state: &mut SessionState,
    config: LiveTopologyConfig,
    intent: &WriteIntent,
) -> Verdict {
    let paths: Vec<&String> = intent
        .paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect();
    if paths.is_empty() {
        return Verdict::Allowed;
    }

    if config.single_writer {
        for path in &paths {
            let key = resource_key_for_raw_target(path);
            if let Some(claim) = state.conflicting_claims(&intent.node_id, &key) {
                let reason = format!(
                    "single_writer: '{node}' cannot write '{path}' — node '{holder}' is live and \
                     already claims '{held}', and no dependency path connects the two, so the \
                     writes would race. Depend on '{holder}', wait for it to finish, or write a \
                     different path.",
                    node = intent.node_id,
                    holder = claim.node_id,
                    held = claim.declared,
                );
                return Verdict::blocked(Invariant::SingleWriter, reason);
            }
        }
    }

    // Claim only after every path cleared, so a partially-admitted write leaves
    // no claim behind for the caller to trip over on its next attempt.
    for path in paths {
        state.claim(&intent.node_id, path);
    }
    Verdict::Allowed
}
