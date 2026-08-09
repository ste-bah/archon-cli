//! Tests for [`TaskManager::resolve_task`](super::TaskManager::resolve_task) —
//! the two-namespace lookup and its refusal to guess.

use super::*;

/// The live failure: an agent held its SUBAGENT id, shortened it, and asked
/// `TaskGet` about it. Task ids and subagent ids are separate namespaces, so
/// exact lookup answered "task not found" for work that existed.
#[test]
fn resolves_by_agent_id_and_by_prefix_of_it() {
    let mgr = TaskManager::new();
    let task_id = mgr.create_task("decompose one spec");
    mgr.set_agent_id(&task_id, "8949f93a-90bf-4d35-aa9a-2a7156e43c16");

    assert!(mgr.resolve_task(&task_id).is_some(), "exact task id");
    assert!(
        mgr.resolve_task("8949f93a-90bf-4d35-aa9a-2a7156e43c16")
            .is_some(),
        "full subagent id"
    );
    assert!(
        mgr.resolve_task("8949f93a").is_some(),
        "shortened subagent id — the case that failed live"
    );
}

/// An ambiguous prefix must return nothing. Reporting the wrong task's
/// status is worse than reporting none, and a caller can always pass more
/// characters.
#[test]
fn an_ambiguous_prefix_resolves_to_nothing() {
    let mgr = TaskManager::new();
    let a = mgr.create_task("first");
    let b = mgr.create_task("second");
    mgr.set_agent_id(&a, "dead0000-0000-0000-0000-000000000001");
    mgr.set_agent_id(&b, "dead0000-0000-0000-0000-000000000002");

    assert!(
        mgr.resolve_task("dead0000").is_none(),
        "a prefix matching two tasks must not guess"
    );
    assert!(
        mgr.resolve_task("dead0000-0000-0000-0000-000000000002")
            .is_some(),
        "the full id still resolves"
    );
}

#[test]
fn unknown_and_empty_ids_still_resolve_to_nothing() {
    let mgr = TaskManager::new();
    mgr.create_task("only task");
    assert!(mgr.resolve_task("nosuchid").is_none());
    assert!(mgr.resolve_task("   ").is_none());
}
