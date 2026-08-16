//! What was true about an agent when it was spawned (#184 M9).
//!
//! The question worth learning to predict is whether parallel agents will
//! conflict when their work is merged. Answering it needs both ends: what was
//! known at spawn — declared writes, whether they overlapped a running agent,
//! which isolation tier was used — and what happened at merge, an hour later
//! and after the agent is gone.
//!
//! Write claims cannot carry it. They are liveness-derived by design (M2): the
//! claim disappears the moment its agent does, which is exactly what makes them
//! self-healing and exactly what makes them useless as a record. So the facts
//! are copied here at spawn, and read back at merge.
//!
//! Bounded by the agents one session spawns, and entries are consumed when the
//! merge reads them.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// What was known about an agent at the moment it was spawned.
#[derive(Debug, Clone, Default)]
pub struct SpawnFacts {
    /// Agent type, for a human reading the row back.
    pub label: Option<String>,
    /// What it said it would write.
    pub declared: Vec<String>,
    /// Whether that overlapped a running agent's claim.
    pub claim_overlap: bool,
    /// Whether it got its own worktree.
    pub isolated: bool,
    /// The team, or the lead, these agents were coordinating under.
    pub coordination_run_id: Option<String>,
}

static RECORDS: OnceLock<Mutex<HashMap<String, SpawnFacts>>> = OnceLock::new();

fn records() -> MutexGuard<'static, HashMap<String, SpawnFacts>> {
    RECORDS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Remember what was true when `agent_id` started.
pub fn record_spawn(agent_id: &str, facts: SpawnFacts) {
    records().insert(agent_id.to_string(), facts);
}

/// Read an agent's spawn facts without consuming them.
pub fn peek(agent_id: &str) -> Option<SpawnFacts> {
    records().get(agent_id).cloned()
}

/// Read an agent's spawn facts and forget them.
///
/// Consumed rather than left behind: the merge is the one event that closes the
/// loop, and a record nothing will ever read again is just a leak.
pub fn take(agent_id: &str) -> Option<SpawnFacts> {
    records().remove(agent_id)
}

/// Drop an agent's record without reading it.
///
/// For agents that end without a merge — nothing will close their loop, so
/// nothing should keep their facts.
pub fn forget(agent_id: &str) {
    records().remove(agent_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(overlap: bool) -> SpawnFacts {
        SpawnFacts {
            label: Some("coder".into()),
            declared: vec!["src/lib.rs".into()],
            claim_overlap: overlap,
            isolated: true,
            coordination_run_id: Some("team-1".into()),
        }
    }

    #[test]
    fn spawn_facts_survive_the_agent_that_declared_them() {
        record_spawn("m9-agent-survive", facts(true));
        let read = peek("m9-agent-survive").expect("recorded");
        assert!(read.claim_overlap);
        assert_eq!(read.declared, vec!["src/lib.rs".to_string()]);
        forget("m9-agent-survive");
    }

    /// The merge closes the loop, so the record goes with it. Otherwise every
    /// session accumulates facts nothing will read.
    #[test]
    fn taking_the_facts_consumes_them() {
        record_spawn("m9-agent-take", facts(false));
        assert!(take("m9-agent-take").is_some());
        assert!(take("m9-agent-take").is_none());
    }

    #[test]
    fn an_agent_that_never_registered_has_no_facts() {
        assert!(peek("m9-agent-never").is_none());
    }
}
