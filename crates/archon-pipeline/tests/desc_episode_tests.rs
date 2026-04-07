//! Tests for DESC Episode Store — REQ-LEARN-008.
//!
//! All tests use CozoDB in-memory mode. No UCM daemon dependency.

use archon_pipeline::learning::desc::{
    ConfidenceCalculator, DescEpisode, DescEpisodeStore, EpisodeQuery, InjectionFilter,
    QualityMonitor, TrajectoryLinker, DEFAULT_MIN_INJECTION_QUALITY, DEFAULT_QUALITY_THRESHOLD,
};
use archon_pipeline::learning::schema::initialize_learning_schemas;
use cozo::DbInstance;

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

fn make_store() -> DescEpisodeStore {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();
    DescEpisodeStore::new(db)
}

fn episode(id: &str, task_type: &str, quality: f64) -> DescEpisode {
    DescEpisode {
        episode_id: id.to_string(),
        session_id: "sess-test".to_string(),
        task_type: task_type.to_string(),
        description: format!("Episode {}", id),
        solution: "fn main() {}".to_string(),
        outcome: "success".to_string(),
        quality_score: quality,
        reward: quality,
        tags: vec!["test".to_string()],
        trajectory_id: None,
        created_at: 1700000000,
        updated_at: 1700000000,
    }
}

// ---------------------------------------------------------------------------
// Test 1: store_episode returns a valid episode_id
// ---------------------------------------------------------------------------

#[test]
fn test_store_episode_returns_valid_id() {
    let store = make_store();
    let ep = episode("ep-001", "coding", 0.8);
    let returned_id = store.store_episode(&ep).unwrap();
    assert_eq!(returned_id, "ep-001", "store_episode should return the episode_id");
}

// ---------------------------------------------------------------------------
// Test 2: get_episode by id retrieves stored episode
// ---------------------------------------------------------------------------

#[test]
fn test_get_episode_by_id() {
    let store = make_store();
    let ep = episode("ep-002", "research", 0.75);
    store.store_episode(&ep).unwrap();

    let retrieved = store.get_episode("ep-002").unwrap();
    assert!(retrieved.is_some(), "stored episode should be retrievable");

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.episode_id, "ep-002");
    assert_eq!(retrieved.task_type, "research");
    assert_eq!(retrieved.session_id, "sess-test");
    assert_eq!(retrieved.outcome, "success");
    assert!((retrieved.quality_score - 0.75).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Test 3: find_episodes returns results for matching task_type
// ---------------------------------------------------------------------------

#[test]
fn test_find_similar_episodes_by_task_type() {
    let store = make_store();
    store.store_episode(&episode("ep-003a", "coding", 0.9)).unwrap();
    store.store_episode(&episode("ep-003b", "coding", 0.7)).unwrap();
    store.store_episode(&episode("ep-003c", "research", 0.8)).unwrap();

    let query = EpisodeQuery {
        task_type: Some("coding".to_string()),
        min_quality: None,
        limit: 10,
    };
    let results = store.find_episodes(&query).unwrap();
    assert_eq!(results.len(), 2, "should find 2 coding episodes");
    for ep in &results {
        assert_eq!(ep.task_type, "coding");
    }
}

// ---------------------------------------------------------------------------
// Test 4: update_quality changes quality_score
// ---------------------------------------------------------------------------

#[test]
fn test_update_quality_changes_score() {
    let store = make_store();
    let ep = episode("ep-004", "analysis", 0.5);
    store.store_episode(&ep).unwrap();

    store.update_quality("ep-004", 0.95).unwrap();

    let retrieved = store.get_episode("ep-004").unwrap().unwrap();
    assert!(
        (retrieved.quality_score - 0.95).abs() < 1e-9,
        "quality_score should be updated to 0.95, got {}",
        retrieved.quality_score
    );
}

// ---------------------------------------------------------------------------
// Test 5: InjectionFilter returns top-k episodes sorted by quality * similarity
// ---------------------------------------------------------------------------

#[test]
fn test_injection_filter_top_k_sorted_by_score() {
    let store = make_store();
    store.store_episode(&episode("ep-005a", "coding", 0.9)).unwrap();
    store.store_episode(&episode("ep-005b", "coding", 0.6)).unwrap();
    store.store_episode(&episode("ep-005c", "research", 0.8)).unwrap();

    let result =
        InjectionFilter::filter_for_injection(&store, "coding", 2, DEFAULT_MIN_INJECTION_QUALITY)
            .unwrap();

    assert!(result.episodes.len() <= 2, "should return at most 2 episodes");
    assert!(!result.episodes.is_empty(), "should have at least one episode");

    // Scores should be non-increasing
    for i in 1..result.episodes.len() {
        assert!(
            result.episodes[i - 1].score >= result.episodes[i].score,
            "episodes should be sorted by score descending"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 6: InjectionFilter respects min quality threshold
// ---------------------------------------------------------------------------

#[test]
fn test_injection_filter_respects_min_quality() {
    let store = make_store();
    store.store_episode(&episode("ep-006a", "coding", 0.9)).unwrap();
    store.store_episode(&episode("ep-006b", "coding", 0.1)).unwrap(); // below threshold

    let result = InjectionFilter::filter_for_injection(&store, "coding", 10, 0.5).unwrap();

    for ep in &result.episodes {
        assert!(
            ep.episode.quality_score >= 0.5,
            "all returned episodes should have quality >= 0.5, got {}",
            ep.episode.quality_score
        );
    }
}

// ---------------------------------------------------------------------------
// Test 7: TrajectoryLinker links episode to SONA trajectory
// ---------------------------------------------------------------------------

#[test]
fn test_trajectory_linker_links_episode() {
    let store = make_store();
    let ep = episode("ep-007", "coding", 0.8);
    store.store_episode(&ep).unwrap();

    TrajectoryLinker::link_episode_to_trajectory(&store, "ep-007", "traj-abc").unwrap();

    let retrieved = store.get_episode("ep-007").unwrap().unwrap();
    assert_eq!(
        retrieved.trajectory_id.as_deref(),
        Some("traj-abc"),
        "trajectory_id should be set after linking"
    );
}

// ---------------------------------------------------------------------------
// Test 8: TrajectoryLinker get_linked_trajectories returns linked IDs
// ---------------------------------------------------------------------------

#[test]
fn test_trajectory_linker_get_linked_trajectories() {
    let store = make_store();
    let ep = episode("ep-008", "research", 0.7);
    store.store_episode(&ep).unwrap();

    TrajectoryLinker::link_episode_to_trajectory(&store, "ep-008", "traj-xyz").unwrap();

    let trajectories = TrajectoryLinker::get_linked_trajectories(&store, "ep-008").unwrap();
    assert!(
        trajectories.contains(&"traj-xyz".to_string()),
        "linked trajectory should be returned"
    );
}

// ---------------------------------------------------------------------------
// Test 9: QualityMonitor detects degradation when mean quality drops below threshold
// ---------------------------------------------------------------------------

#[test]
fn test_quality_monitor_detects_degradation() {
    let store = make_store();
    // Insert low-quality episodes
    store.store_episode(&episode("ep-009a", "coding", 0.2)).unwrap();
    store.store_episode(&episode("ep-009b", "coding", 0.3)).unwrap();
    store.store_episode(&episode("ep-009c", "coding", 0.25)).unwrap();

    let report = QualityMonitor::check(&store, DEFAULT_QUALITY_THRESHOLD).unwrap();

    assert!(
        report.degradation_detected,
        "degradation should be detected when mean quality ({}) < threshold ({})",
        report.mean_quality,
        DEFAULT_QUALITY_THRESHOLD
    );
    assert_eq!(report.total_episodes, 3);
    assert!((report.degradation_threshold - DEFAULT_QUALITY_THRESHOLD).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Test 10: Episode creation with all fields populated
// ---------------------------------------------------------------------------

#[test]
fn test_episode_all_fields_populated() {
    let store = make_store();
    let ep = DescEpisode {
        episode_id: "ep-010".to_string(),
        session_id: "sess-full".to_string(),
        task_type: "analysis".to_string(),
        description: "Full field test".to_string(),
        solution: "impl Solution {}".to_string(),
        outcome: "partial".to_string(),
        quality_score: 0.65,
        reward: 1.2,
        tags: vec!["tag1".to_string(), "tag2".to_string(), "tag3".to_string()],
        trajectory_id: Some("traj-full".to_string()),
        created_at: 1700000001,
        updated_at: 1700000002,
    };

    store.store_episode(&ep).unwrap();

    let retrieved = store.get_episode("ep-010").unwrap().unwrap();
    assert_eq!(retrieved.episode_id, "ep-010");
    assert_eq!(retrieved.session_id, "sess-full");
    assert_eq!(retrieved.task_type, "analysis");
    assert_eq!(retrieved.description, "Full field test");
    assert_eq!(retrieved.solution, "impl Solution {}");
    assert_eq!(retrieved.outcome, "partial");
    assert!((retrieved.quality_score - 0.65).abs() < 1e-9);
    assert!((retrieved.reward - 1.2).abs() < 1e-9);
    assert_eq!(retrieved.tags.len(), 3);
    assert_eq!(retrieved.trajectory_id.as_deref(), Some("traj-full"));
}

// ---------------------------------------------------------------------------
// Test 11: No UCM daemon dependency — all operations use direct CozoDB calls
// ---------------------------------------------------------------------------

#[test]
fn test_no_ucm_daemon_dependency() {
    // This test verifies that the store can be constructed and used
    // purely from a DbInstance with no external daemon or process.
    // If this test passes, the implementation is daemon-free.
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();
    let store = DescEpisodeStore::new(db);

    let ep = episode("ep-011", "planning", 0.85);
    let id = store.store_episode(&ep).unwrap();
    assert_eq!(id, "ep-011");

    let found = store.get_episode("ep-011").unwrap();
    assert!(found.is_some());
}

// ---------------------------------------------------------------------------
// Test 12: ConfidenceCalculator basic sanity
// ---------------------------------------------------------------------------

#[test]
fn test_confidence_calculator_basic() {
    // Fresh episode (age_secs=0): recency=1.0, result = quality * similarity * 1.0
    let score = ConfidenceCalculator::calculate(0.8, 1.0, 0);
    assert!((score - 0.8).abs() < 1e-9, "score should be quality*similarity for fresh episode");

    // Clamped to [0,1]
    let clamped = ConfidenceCalculator::calculate(2.0, 2.0, 0);
    assert!((clamped - 1.0).abs() < 1e-9, "score should clamp to 1.0");

    // Zero quality
    let zero = ConfidenceCalculator::calculate(0.0, 1.0, 0);
    assert!((zero).abs() < 1e-9, "zero quality gives zero score");
}

// ---------------------------------------------------------------------------
// Test 13: QualityMonitor on empty store
// ---------------------------------------------------------------------------

#[test]
fn test_quality_monitor_empty_store() {
    let store = make_store();
    let report = QualityMonitor::check(&store, DEFAULT_QUALITY_THRESHOLD).unwrap();
    assert_eq!(report.total_episodes, 0);
    assert!(!report.degradation_detected, "no degradation on empty store");
    assert!((report.mean_quality).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Test 14: get_episode returns None for missing id
// ---------------------------------------------------------------------------

#[test]
fn test_get_episode_missing_id() {
    let store = make_store();
    let result = store.get_episode("nonexistent-id").unwrap();
    assert!(result.is_none(), "should return None for unknown episode_id");
}

// ---------------------------------------------------------------------------
// Test 15: find_episodes with default query returns all episodes up to limit
// ---------------------------------------------------------------------------

#[test]
fn test_find_episodes_default_query() {
    let store = make_store();
    for i in 0..5 {
        store
            .store_episode(&episode(&format!("ep-015-{}", i), "coding", 0.5 + i as f64 * 0.05))
            .unwrap();
    }

    let results = store.find_episodes(&EpisodeQuery::default()).unwrap();
    assert_eq!(results.len(), 5, "default query should return all 5 episodes");
}
