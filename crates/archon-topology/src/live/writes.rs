//! Invariant 2 — single writer per artifact.
//!
//! A write to a path already claimed by a **concurrently live** node with **no
//! dependency path** between them is blocked. Both qualifiers matter: a node
//! that has finished holds no claim, and two nodes joined by a dependency path
//! are ordered relative to one another and so cannot race.
//!
//! # This extends the write coordinator; it does not reimplement it
//!
//! `archon-write-plan` already answers "do these two write sets overlap?"
//! properly, with a resource-key overlap table covering file/dir/glob
//! combinations (`archon_write_plan::write_plan::keys_conflict`). It is the same
//! table the write coordinator plans by; it lives in its own leaf crate
//! precisely so both callers share it without a dependency edge between them.
//! Admission calls it rather than carrying a second opinion.
//! `archon-topology`'s own `TaskGraph::write_conflicts` uses exact-string
//! overlap and says so — that is the conservative floor available under
//! milestone 1's dependency budget, not a rival algorithm.
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
use archon_write_plan::shared_append::shared_append_key_for_raw_target;
use archon_write_plan::write_plan::{ResourceKey, resource_key_for_raw_target};

/// Admit a write, claiming its paths when admitted.
pub(super) fn admit_write(
    state: &mut SessionState,
    config: LiveTopologyConfig,
    intent: &WriteIntent,
) -> Verdict {
    let claims = declared_claims(intent);
    if claims.is_empty() {
        return Verdict::Allowed;
    }

    if config.single_writer {
        for (path, key) in &claims {
            if let Some(claim) = state.conflicting_claims(&intent.node_id, key) {
                return Verdict::blocked(Invariant::SingleWriter, reason(intent, path, claim));
            }
        }
    }

    // Claim only after every path cleared, so a partially-admitted write leaves
    // no claim behind for the caller to trip over on its next attempt.
    for (path, key) in claims {
        state.claim(&intent.node_id, path, key);
    }
    Verdict::Allowed
}

/// One resource key per declared path, exclusive unless declared shared.
///
/// The same three rules the write coordinator plans by, because they are the
/// same table: `keys_conflict` decides, and this only chooses which key to hand
/// it. A path in both lists resolves to `SharedAppend` — the shared declaration
/// is the more specific statement, and resolving it the other way would make the
/// declaration unusable for any node that also lists its full target set.
fn declared_claims(intent: &WriteIntent) -> Vec<(&str, ResourceKey)> {
    let shared: Vec<&str> = intent
        .shared_append
        .iter()
        .map(String::as_str)
        .filter(|path| !path.trim().is_empty())
        .collect();
    let mut claims: Vec<(&str, ResourceKey)> = shared
        .iter()
        .map(|path| (*path, shared_append_key_for_raw_target(path)))
        .collect();
    for path in &intent.paths {
        let path = path.as_str();
        if path.trim().is_empty() || shared.contains(&path) {
            continue;
        }
        claims.push((path, resource_key_for_raw_target(path)));
    }
    claims
}

fn reason(intent: &WriteIntent, path: &str, claim: &super::state::WriteClaim) -> String {
    format!(
        "single_writer: '{node}' cannot write '{path}' — node '{holder}' is live and already \
         claims '{held}', and no dependency path connects the two, so the writes would race. \
         Depend on '{holder}', wait for it to finish, or write a different path. If both writes \
         are genuinely coordinated and atomic, both sides must declare the path as a shared \
         append; one side declaring it is a claim to exclusive access.",
        node = intent.node_id,
        holder = claim.node_id,
        held = claim.declared,
    )
}
