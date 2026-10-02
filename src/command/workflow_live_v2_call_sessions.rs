//! Which agent sessions a workflow call dispatched, and the per-call bounds
//! the host puts on them.
//!
//! #215: `WorkflowV2CallRecord.agent_session_id` existed and no production
//! caller set it, so nothing linked a call to the transcript that explains it.
//! The session id is minted where the agent is dispatched (the live client),
//! long after the record's call has been planned and long before its record
//! is written, and a fan-out mints one per branch. So the client notes each
//! session it dispatches under the call id it served, and the host takes them
//! back by call when it writes the call's record: the call's own id and every
//! id nested under it (`{call}-{item}` branches, transport retries, repairs).
//!
//! Issue-213 C2c: the same place bounds the call's turns. A call that declares
//! `maxTurns` runs its session under that bound instead of the effectively
//! unlimited default; a call that declares none is unchanged.

use std::sync::{LazyLock, Mutex};

/// Sessions awaiting their call's record; the oldest is dropped past this.
const MAX_PENDING: usize = 4_096;

/// `(run_id, call_id, session_id)`, oldest first.
static PENDING: LazyLock<Mutex<Vec<(String, String, String)>>> = LazyLock::new(Mutex::default);

/// The client dispatched `session_id` to serve `call_id` of `run_id`.
pub(crate) fn note_session(run_id: &str, call_id: &str, session_id: &str) {
    let Ok(mut pending) = PENDING.lock() else {
        return;
    };
    if pending
        .iter()
        .any(|(run, call, session)| run == run_id && call == call_id && session == session_id)
    {
        return;
    }
    if pending.len() >= MAX_PENDING {
        pending.remove(0);
    }
    pending.push((
        run_id.to_string(),
        call_id.to_string(),
        session_id.to_string(),
    ));
}

/// Take every session dispatched for `call_id` or a call nested under it, in
/// dispatch order, leaving none behind.
pub(crate) fn take_sessions(run_id: &str, call_id: &str) -> Vec<String> {
    let Ok(mut pending) = PENDING.lock() else {
        return Vec::new();
    };
    let nested = format!("{call_id}-");
    let mut taken = Vec::new();
    pending.retain(|(run, call, session)| {
        let ours = run == run_id && (call == call_id || call.starts_with(&nested));
        if ours && !taken.contains(session) {
            taken.push(session.clone());
        }
        !ours
    });
    taken
}

/// Every session noted for `call_id` or a call nested under it, left in
/// place: what a running call's in-flight marker names (Issue-213 C5), before
/// its record takes them.
pub(crate) fn peek_sessions(run_id: &str, call_id: &str) -> Vec<String> {
    let Ok(pending) = PENDING.lock() else {
        return Vec::new();
    };
    let nested = format!("{call_id}-");
    let mut found = Vec::new();
    for (run, call, session) in pending.iter() {
        if run == run_id
            && (call == call_id || call.starts_with(&nested))
            && !found.contains(session)
        {
            found.push(session.clone());
        }
    }
    found
}

/// The turn bound a call declared (`maxTurns`), if any.
pub(crate) fn declared_max_turns(call: &archon_workflow::WorkflowV2HostCall) -> Option<u32> {
    ["maxTurns", "max_turns"]
        .iter()
        .find_map(|key| call.options.extra.get(*key))
        .and_then(serde_json::Value::as_u64)
        .map(|turns| u32::try_from(turns).unwrap_or(u32::MAX))
}

/// Run `work` under the turn bound `call` declared, or unchanged.
pub(crate) async fn bounded<T>(
    call: &archon_workflow::WorkflowV2HostCall,
    work: impl std::future::Future<Output = T>,
) -> T {
    archon_tools::host_max_turns::inherit(declared_max_turns(call), work).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_takes_its_own_and_its_nested_sessions_only() {
        let run = "wf-call-sessions-test";
        note_session(run, "agents-3", "s-parent");
        note_session(run, "agents-3-item-a", "s-a");
        note_session(run, "agents-3-item-a", "s-a");
        note_session(run, "agents-3-item-b-transport-retry-1", "s-b");
        note_session(run, "agents-30", "s-other-call");
        note_session("wf-another-run", "agents-3", "s-other-run");

        assert_eq!(
            peek_sessions(run, "agents-3"),
            vec!["s-parent", "s-a", "s-b"]
        );
        assert_eq!(
            take_sessions(run, "agents-3"),
            vec!["s-parent", "s-a", "s-b"]
        );
        assert!(take_sessions(run, "agents-3").is_empty(), "taken once");
        assert_eq!(take_sessions(run, "agents-30"), vec!["s-other-call"]);
        assert_eq!(
            take_sessions("wf-another-run", "agents-3"),
            vec!["s-other-run"]
        );
    }

    #[test]
    fn a_declared_turn_bound_is_read_from_the_call() {
        let mut call = archon_workflow::WorkflowV2HostCall {
            id: "agent-1".to_string(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        };
        assert_eq!(declared_max_turns(&call), None);
        call.options
            .extra
            .insert("maxTurns".to_string(), serde_json::json!(40));
        assert_eq!(declared_max_turns(&call), Some(40));
    }

    #[tokio::test]
    async fn the_declared_bound_reaches_the_session_builder() {
        let mut call = archon_workflow::WorkflowV2HostCall {
            id: "agent-2".to_string(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        };
        call.options
            .extra
            .insert("max_turns".to_string(), serde_json::json!(25));
        let seen = bounded(&call, async {
            archon_tools::host_max_turns::resolve(100_000, 100_000)
        })
        .await;
        assert_eq!(seen, 25);
        call.options.extra.clear();
        let unbounded = bounded(&call, async {
            archon_tools::host_max_turns::resolve(100_000, 100_000)
        })
        .await;
        assert_eq!(unbounded, 100_000);
    }
}
