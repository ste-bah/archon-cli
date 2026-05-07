use super::*;

// -----------------------------------------------------------------------
// TASK-T3 (G4): ProgressTracker accumulation tests
// -----------------------------------------------------------------------

#[test]
fn progress_tracker_default_has_sane_state() {
    let t = ProgressTracker::default();
    assert_eq!(t.tool_use_count, 0);
    assert_eq!(t.cumulative_input_tokens, 0);
    assert_eq!(t.cumulative_output_tokens, 0);
    assert_eq!(t.cumulative_cache_creation_tokens, 0);
    assert_eq!(t.cumulative_cache_read_tokens, 0);
    assert!(t.recent_activities.is_empty());
}

#[test]
fn progress_tracker_activities_bounded_at_five() {
    let mut t = ProgressTracker::default();
    for i in 0..7u32 {
        // Mirror the runner's bounding logic: pop oldest before push when at cap.
        if t.recent_activities.len() >= 5 {
            t.recent_activities.pop_front();
        }
        t.recent_activities.push_back(ToolActivity {
            tool_name: format!("tool-{i}"),
            timestamp: chrono::Utc::now(),
        });
    }
    assert_eq!(t.recent_activities.len(), 5);
    // Oldest two ("tool-0", "tool-1") should have been evicted.
    assert_eq!(t.recent_activities.front().unwrap().tool_name, "tool-2");
    assert_eq!(t.recent_activities.back().unwrap().tool_name, "tool-6");
}

#[test]
fn progress_tracker_accumulates_usage_from_message_start() {
    // Simulate the same accumulation the runner performs in its
    // MessageStart / MessageDelta arms.
    let usages = [
        archon_llm::types::Usage {
            input_tokens: 100,
            output_tokens: 5,
            cache_creation_input_tokens: 10,
            cache_read_input_tokens: 20,
        },
        archon_llm::types::Usage {
            input_tokens: 50,
            output_tokens: 25,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 5,
        },
        archon_llm::types::Usage {
            input_tokens: 0,
            output_tokens: 12,
            cache_creation_input_tokens: 2,
            cache_read_input_tokens: 0,
        },
    ];

    let mut t = ProgressTracker::default();
    for u in &usages {
        t.cumulative_input_tokens += u.input_tokens;
        t.cumulative_output_tokens += u.output_tokens;
        t.cumulative_cache_creation_tokens += u.cache_creation_input_tokens;
        t.cumulative_cache_read_tokens += u.cache_read_input_tokens;
        t.last_update = chrono::Utc::now();
    }

    assert_eq!(t.cumulative_input_tokens, 150);
    assert_eq!(t.cumulative_output_tokens, 42);
    assert_eq!(t.cumulative_cache_creation_tokens, 12);
    assert_eq!(t.cumulative_cache_read_tokens, 25);
}

#[test]
fn subagent_manager_get_progress_returns_snapshot() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    let snap = mgr
        .get_progress(&id)
        .expect("snapshot exists for live agent");
    assert_eq!(snap.tool_use_count, 0);
    assert_eq!(snap.cumulative_input_tokens, 0);
    assert_eq!(snap.cumulative_output_tokens, 0);
    assert_eq!(snap.cumulative_cache_creation_tokens, 0);
    assert_eq!(snap.cumulative_cache_read_tokens, 0);
    assert!(snap.recent_activities.is_empty());

    // Unknown id returns None
    assert!(mgr.get_progress("not-a-real-id").is_none());
}

#[test]
fn subagent_manager_get_progress_tracker_arc_clones_same_arc() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    let arc1 = mgr.get_progress_tracker_arc(&id).expect("arc1");
    let arc2 = mgr.get_progress_tracker_arc(&id).expect("arc2");

    // Mutate via arc1, observe via arc2 — proves both point to the same inner mutex.
    {
        let mut g = arc1.lock().unwrap();
        g.tool_use_count = 42;
        g.cumulative_input_tokens = 1234;
    }
    {
        let g = arc2.lock().unwrap();
        assert_eq!(g.tool_use_count, 42);
        assert_eq!(g.cumulative_input_tokens, 1234);
    }

    // And the manager's snapshot view also reflects the mutation.
    let snap = mgr.get_progress(&id).unwrap();
    assert_eq!(snap.tool_use_count, 42);
    assert_eq!(snap.cumulative_input_tokens, 1234);
}
