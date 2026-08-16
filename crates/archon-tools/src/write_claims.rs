//! What each agent has said it intends to write (#184 M2).
//!
//! Parallel agents shared one working tree with last-write-wins, no locks and
//! no detection. Nothing recorded what anyone was working on, so two agents
//! editing the same file was not merely possible but invisible.
//!
//! An agent may declare its intended writes when it is spawned. Overlapping
//! declarations are reported at spawn time, before either agent has run —
//! "coordination at dispatch time beats reconciliation at merge time".
//!
//! ## Claims are advisory
//!
//! A declaration is a statement of intent, not a lock. An agent that declares
//! nothing is unconstrained, and an agent that declares badly is not stopped
//! from writing elsewhere. What this buys is a warning at the moment it is
//! still cheap to act on, plus the input M3 needs to decide whether two agents
//! should be isolated from each other at all.
//!
//! ## There is no release, on purpose
//!
//! Every terminal hook in this codebase skips the `AutoBackgrounded` arm, and
//! long-running agents are exactly the ones most likely to be holding a claim —
//! `board/leases.rs` documents that trap and refuses to hang its own sweep off
//! `SubagentStop` for the same reason. A claim released by a hook would leak
//! precisely when it matters.
//!
//! So liveness is applied when claims are *read*: a claim held by an agent that
//! is no longer running cannot conflict with anything, whatever became of the
//! agent. Entries are swept opportunistically to stop the map growing, but
//! correctness never depends on the sweep having run.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use archon_write_plan::write_plan::{ResourceKey, keys_conflict, resource_key_for_raw_target};

/// One agent's declared write intent.
#[derive(Debug, Clone)]
pub struct WriteClaim {
    /// The agent that declared it — the runtime's id, never a model argument.
    pub agent_id: String,
    /// Human-facing label for messages, usually the agent type.
    pub label: Option<String>,
    /// What it said it would write, as declared.
    pub declared: Vec<String>,
    keys: Vec<ResourceKey>,
}

impl WriteClaim {
    fn conflicts_with(&self, keys: &[ResourceKey]) -> Vec<String> {
        let mut overlapping = Vec::new();
        for (raw, key) in self.declared.iter().zip(&self.keys) {
            if keys.iter().any(|other| keys_conflict(key, other)) {
                overlapping.push(raw.clone());
            }
        }
        overlapping
    }

    /// How to describe this claim's holder in a warning.
    pub fn describe(&self) -> String {
        match &self.label {
            Some(label) if !label.is_empty() => format!("'{}' ({})", label, self.agent_id),
            _ => format!("'{}'", self.agent_id),
        }
    }
}

/// An overlap found at spawn time.
#[derive(Debug, Clone)]
pub struct ClaimOverlap {
    pub holder: WriteClaim,
    /// The paths or globs the two declarations have in common.
    pub paths: Vec<String>,
}

static CLAIMS: OnceLock<Mutex<HashMap<String, WriteClaim>>> = OnceLock::new();

fn claims() -> MutexGuard<'static, HashMap<String, WriteClaim>> {
    CLAIMS
        .get_or_init(|| Mutex::new(HashMap::new()))
        // A poisoned map means some other thread panicked while claiming, not
        // that the claims are wrong. Recovering keeps one panicking agent from
        // disabling conflict detection for the rest of the session.
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Whether an agent is still running, and therefore whether its claim counts.
///
/// One registry, one lookup — `holder_liveness` already handles the top-level
/// agent (never registered as a subagent, alive for the life of the process)
/// and the pipeline id shapes. Asking anything else here would be the second
/// opinion `board/leases.rs` was written to eliminate.
fn is_live(agent_id: &str) -> bool {
    matches!(
        crate::board::leases::holder_liveness(agent_id),
        crate::board::leases::HolderLiveness::Live
    )
}

/// Record `declared` as `agent_id`'s intent, returning any live overlaps.
///
/// The claim is recorded whether or not it overlaps: refusing to record would
/// mean the second agent's intent is invisible to a third.
pub fn claim(agent_id: &str, label: Option<&str>, declared: &[String]) -> Vec<ClaimOverlap> {
    let keys: Vec<ResourceKey> = declared
        .iter()
        .map(|raw| resource_key_for_raw_target(raw))
        .collect();

    let mut map = claims();

    // Opportunistic sweep. Correctness does not depend on it — the liveness
    // filter below is what decides — but without it the map keeps an entry per
    // agent for the life of the process.
    map.retain(|id, _| id == agent_id || is_live(id));

    let overlaps: Vec<ClaimOverlap> = map
        .values()
        .filter(|held| held.agent_id != agent_id && is_live(&held.agent_id))
        .filter_map(|held| {
            let paths = held.conflicts_with(&keys);
            (!paths.is_empty()).then(|| ClaimOverlap {
                holder: held.clone(),
                paths,
            })
        })
        .collect();

    map.insert(
        agent_id.to_string(),
        WriteClaim {
            agent_id: agent_id.to_string(),
            label: label.map(str::to_string),
            declared: declared.to_vec(),
            keys,
        },
    );

    overlaps
}

/// Live overlaps against a claim `agent_id` has already recorded.
///
/// The spawn path records the claim and reports overlaps in one step; the
/// executor needs to ask again slightly later, when deciding isolation, without
/// re-recording anything. Empty when the agent declared nothing — which is why
/// declaring is what buys the protection.
pub fn overlaps_for(agent_id: &str) -> Vec<ClaimOverlap> {
    let map = claims();
    let Some(mine) = map.get(agent_id) else {
        return Vec::new();
    };

    map.values()
        .filter(|held| held.agent_id != agent_id && is_live(&held.agent_id))
        .filter_map(|held| {
            let paths = held.conflicts_with(&mine.keys);
            (!paths.is_empty()).then(|| ClaimOverlap {
                holder: held.clone(),
                paths,
            })
        })
        .collect()
}

/// Drop `agent_id`'s claim.
///
/// Not required for correctness — a dead agent's claim is already ignored — but
/// worth calling where a terminal state is known, so the map stays small and
/// `live_claims` reads cleanly.
pub fn release(agent_id: &str) {
    claims().remove(agent_id);
}

/// Every claim whose holder is still running.
pub fn live_claims() -> Vec<WriteClaim> {
    claims()
        .values()
        .filter(|claim| is_live(&claim.agent_id))
        .cloned()
        .collect()
}

/// What one agent said it would write, as declared.
///
/// Empty when it declared nothing, or when its claim has already been swept —
/// callers that need the declaration to outlive the agent copy it while the
/// agent is starting (see `coordination_record`).
pub fn declared_by(agent_id: &str) -> Vec<String> {
    claims()
        .get(agent_id)
        .map(|claim| claim.declared.clone())
        .unwrap_or_default()
}

/// Render overlaps as the warning that goes back in the spawn result.
///
/// A warning, not an error: the declaration is advisory, and refusing the spawn
/// outright would make declaring intent worse than staying silent.
pub fn describe_overlaps(overlaps: &[ClaimOverlap]) -> String {
    let mut out = String::from(
        "Warning: another running agent has already declared writes that overlap yours.\n",
    );
    for overlap in overlaps {
        out.push_str(&format!(
            "  - {} also writes: {}\n",
            overlap.holder.describe(),
            overlap.paths.join(", ")
        ));
    }
    out.push_str(
        "Both agents share one working tree, so the later write wins. \
         Consider narrowing the split, sequencing them, or spawning with isolation.",
    );
    out
}

#[cfg(test)]
#[path = "write_claims_tests.rs"]
mod tests;
