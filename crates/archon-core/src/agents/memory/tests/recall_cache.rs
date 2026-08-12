//! Issue #171 Part 6 — the recall cache must collapse a fan-out's identical
//! queries without outliving the memories it caches.

use std::time::Duration;

use super::helpers::MockMemory;
use super::*;
use crate::agents::memory::AgentMemoryRecallCache;

fn queries() -> Vec<String> {
    vec!["past reviews".to_string(), "known pitfalls".to_string()]
}

#[test]
fn fanout_of_the_same_agent_type_queries_the_store_once() {
    let memory = MockMemory::new();
    memory.set_search_results(&["remember the lock ordering"]);
    let cache = AgentMemoryRecallCache::new();
    let queries = queries();

    let first = cache
        .block(
            "code-reviewer",
            &queries,
            &memory,
            Some(&AgentMemoryScope::Project),
        )
        .expect("memories present");

    for _ in 0..4 {
        let again = cache
            .block(
                "code-reviewer",
                &queries,
                &memory,
                Some(&AgentMemoryScope::Project),
            )
            .expect("memories present");
        assert_eq!(&*again, &*first);
    }

    assert_eq!(
        memory.search_count(),
        queries.len(),
        "5 spawns must run the recall queries exactly once"
    );
    let stats = cache.stats();
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 4);
    assert_eq!(stats.queries_run, queries.len());
}

#[test]
fn cached_block_is_the_rendered_agent_memory_element() {
    let memory = MockMemory::new();
    memory.set_search_results(&["alpha"]);
    let cache = AgentMemoryRecallCache::new();

    let block = cache
        .block(
            "explore",
            &["q".to_string()],
            &memory,
            Some(&AgentMemoryScope::User),
        )
        .unwrap();

    // Two queries' worth of rows join with the historical separator.
    assert!(block.starts_with("<agent-memory>\n"));
    assert!(block.ends_with("\n</agent-memory>"));
    assert!(block.contains("alpha"));
}

#[test]
fn rendered_block_matches_the_uncached_composition() {
    let memory = MockMemory::new();
    memory.set_search_results(&["row one", "row two"]);
    let cache = AgentMemoryRecallCache::new();
    let queries = queries();

    let direct = load_agent_memory(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    let expected = format!(
        "<agent-memory>\n{}\n</agent-memory>",
        direct.join("\n---\n")
    );

    let block = cache
        .block(
            "code-reviewer",
            &queries,
            &memory,
            Some(&AgentMemoryScope::Project),
        )
        .unwrap();
    assert_eq!(&*block, expected.as_str());
}

#[test]
fn empty_recall_is_cached_as_absent_without_requerying() {
    let memory = MockMemory::new(); // no rows configured
    let cache = AgentMemoryRecallCache::new();
    let queries = queries();

    assert!(
        cache
            .block("plan", &queries, &memory, Some(&AgentMemoryScope::Local))
            .is_none()
    );
    assert!(
        cache
            .block("plan", &queries, &memory, Some(&AgentMemoryScope::Local))
            .is_none()
    );

    assert_eq!(memory.search_count(), queries.len());
    assert_eq!(cache.stats().hits, 1);
}

#[test]
fn a_write_invalidates_the_matching_key_immediately() {
    let memory = MockMemory::new();
    memory.set_search_results(&["before the write"]);
    let cache = AgentMemoryRecallCache::new();
    let queries = queries();

    let before = cache
        .block(
            "code-reviewer",
            &queries,
            &memory,
            Some(&AgentMemoryScope::Project),
        )
        .unwrap();
    assert!(before.contains("before the write"));

    // Same shape as the executor's completion path: save, then invalidate.
    memory.set_search_results(&["after the write"]);
    cache.invalidate("code-reviewer", Some(&AgentMemoryScope::Project));

    let after = cache
        .block(
            "code-reviewer",
            &queries,
            &memory,
            Some(&AgentMemoryScope::Project),
        )
        .unwrap();
    assert!(
        after.contains("after the write"),
        "a spawn right after a write must not see the pre-write block"
    );
    assert_eq!(cache.stats().misses, 2);
}

#[test]
fn invalidation_is_scoped_to_the_agent_type_and_scope() {
    let memory = MockMemory::new();
    memory.set_search_results(&["shared row"]);
    let cache = AgentMemoryRecallCache::new();
    let queries = queries();

    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    cache.block(
        "explore",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::User),
    );
    assert_eq!(cache.stats().misses, 3);

    cache.invalidate("code-reviewer", Some(&AgentMemoryScope::Project));

    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    ); // miss
    cache.block(
        "explore",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    ); // hit
    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::User),
    ); // hit

    let stats = cache.stats();
    assert_eq!(stats.misses, 4);
    assert_eq!(stats.hits, 2);
}

#[test]
fn invalidation_drops_every_recall_query_variant_of_the_key() {
    let memory = MockMemory::new();
    memory.set_search_results(&["row"]);
    let cache = AgentMemoryRecallCache::new();

    cache.block(
        "code-reviewer",
        &["q1".to_string()],
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    cache.block(
        "code-reviewer",
        &["q2".to_string()],
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    assert_eq!(cache.stats().misses, 2);

    cache.invalidate("code-reviewer", Some(&AgentMemoryScope::Project));

    cache.block(
        "code-reviewer",
        &["q1".to_string()],
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    cache.block(
        "code-reviewer",
        &["q2".to_string()],
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    assert_eq!(cache.stats().misses, 4);
    assert_eq!(cache.stats().hits, 0);
}

#[test]
fn different_recall_queries_do_not_share_a_cache_entry() {
    let memory = MockMemory::new();
    memory.set_search_results(&["row"]);
    let cache = AgentMemoryRecallCache::new();

    cache.block(
        "code-reviewer",
        &["q1".to_string()],
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    cache.block(
        "code-reviewer",
        &["q2".to_string()],
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    assert_eq!(
        cache.stats().misses,
        2,
        "a registry reload that changes recall_queries must not hit"
    );
}

#[test]
fn an_expired_entry_is_requeried() {
    let memory = MockMemory::new();
    memory.set_search_results(&["stale"]);
    let cache = AgentMemoryRecallCache::with_ttl(Duration::from_millis(0));
    let queries = queries();

    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    memory.set_search_results(&["fresh"]);
    let after = cache
        .block(
            "code-reviewer",
            &queries,
            &memory,
            Some(&AgentMemoryScope::Project),
        )
        .unwrap();

    assert!(after.contains("fresh"));
    assert_eq!(cache.stats().hits, 0);
    assert_eq!(cache.stats().misses, 2);
}

#[test]
fn no_scope_and_no_queries_short_circuit_without_touching_the_store() {
    let memory = MockMemory::new();
    memory.set_search_results(&["row"]);
    let cache = AgentMemoryRecallCache::new();

    assert!(cache.block("plan", &queries(), &memory, None).is_none());
    assert!(
        cache
            .block("plan", &[], &memory, Some(&AgentMemoryScope::User))
            .is_none()
    );

    assert_eq!(memory.search_count(), 0);
    assert_eq!(cache.stats().misses, 0);
    assert_eq!(cache.stats().hits, 0);
}

#[test]
fn invalidating_a_none_scope_is_a_noop() {
    let memory = MockMemory::new();
    memory.set_search_results(&["row"]);
    let cache = AgentMemoryRecallCache::new();
    let queries = queries();

    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    );
    cache.invalidate("code-reviewer", None);
    cache.block(
        "code-reviewer",
        &queries,
        &memory,
        Some(&AgentMemoryScope::Project),
    );

    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 1);
}
