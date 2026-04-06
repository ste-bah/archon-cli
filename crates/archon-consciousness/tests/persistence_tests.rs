//! Integration tests for `archon_consciousness::persistence`.

use archon_consciousness::inner_voice::InnerVoiceSnapshot;
use archon_consciousness::persistence::{
    PersonalitySnapshot, PersonalityTrends, RuleScoreEntry, SessionStats, TrendDirection,
    compute_trends, generate_briefing, load_latest_snapshot, prune_snapshots, save_snapshot,
};
use archon_memory::MemoryGraph;
use chrono::Utc;

// ── helper ──────────────────────────────────────────────────────

/// Build a [`PersonalitySnapshot`] with sensible defaults, letting callers
/// override the fields that matter for a given test.
fn make_snapshot(
    session_id: &str,
    confidence_end: f32,
    corrections: u32,
    struggles: Vec<String>,
    successes: Vec<String>,
) -> PersonalitySnapshot {
    PersonalitySnapshot {
        session_id: session_id.to_string(),
        timestamp: Utc::now(),
        inner_voice: InnerVoiceSnapshot {
            confidence: confidence_end,
            energy: 0.85,
            focus: "general".to_string(),
            struggles: struggles.clone(),
            successes: successes.clone(),
            turn_count: 10,
            corrections_received: corrections,
        },
        rule_scores: vec![
            RuleScoreEntry {
                rule_id: "r1".to_string(),
                rule_text: "Always confirm before acting".to_string(),
                score: 80.0,
            },
            RuleScoreEntry {
                rule_id: "r2".to_string(),
                rule_text: "Use MemoryGraph for persistence".to_string(),
                score: 65.0,
            },
        ],
        stats: SessionStats {
            total_turns: 10,
            total_corrections: corrections,
            total_tool_calls: 25,
            total_tool_failures: 2,
            confidence_start: 0.7,
            confidence_end,
            energy_end: 0.85,
            top_struggles: struggles,
            top_successes: successes,
            duration_secs: 300,
        },
    }
}

// ── tests ───────────────────────────────────────────────────────

#[test]
fn persistence_save_and_load_roundtrip() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");
    let snap = make_snapshot(
        "sess-1",
        0.8,
        2,
        vec!["shell".into()],
        vec!["code review".into()],
    );

    let id = save_snapshot(&graph, &snap).expect("save_snapshot should succeed");
    assert!(!id.is_empty(), "returned memory id should be non-empty");

    let loaded = load_latest_snapshot(&graph)
        .expect("load_latest_snapshot should succeed")
        .expect("should return Some after saving one snapshot");

    assert_eq!(loaded.session_id, snap.session_id, "session_id mismatch");
    assert_eq!(
        loaded.stats.confidence_end, snap.stats.confidence_end,
        "confidence_end mismatch"
    );
    assert_eq!(
        loaded.stats.total_corrections, snap.stats.total_corrections,
        "total_corrections mismatch"
    );
    assert_eq!(
        loaded.stats.top_struggles, snap.stats.top_struggles,
        "top_struggles mismatch"
    );
    assert_eq!(
        loaded.stats.top_successes, snap.stats.top_successes,
        "top_successes mismatch"
    );
    assert_eq!(
        loaded.rule_scores.len(),
        snap.rule_scores.len(),
        "rule_scores length mismatch"
    );
}

#[test]
fn persistence_load_empty_graph() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    let result = load_latest_snapshot(&graph).expect("load_latest_snapshot should not error");
    assert!(
        result.is_none(),
        "loading from an empty graph should return None"
    );
}

#[test]
fn persistence_load_latest_returns_newest() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    // Save 3 snapshots — the last one saved should be the newest because
    // each call to save_snapshot stores with a fresh created_at timestamp.
    save_snapshot(&graph, &make_snapshot("sess-old", 0.5, 5, vec![], vec![]))
        .expect("save snapshot 1");
    save_snapshot(&graph, &make_snapshot("sess-mid", 0.6, 3, vec![], vec![]))
        .expect("save snapshot 2");
    save_snapshot(&graph, &make_snapshot("sess-new", 0.9, 1, vec![], vec![]))
        .expect("save snapshot 3");

    let latest = load_latest_snapshot(&graph)
        .expect("load should succeed")
        .expect("should return Some");

    assert_eq!(
        latest.session_id, "sess-new",
        "latest snapshot should be the most recently saved one"
    );
}

#[test]
fn persistence_prune_removes_oldest() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    for i in 0..5 {
        save_snapshot(
            &graph,
            &make_snapshot(
                &format!("sess-{i}"),
                0.5 + i as f32 * 0.05,
                i,
                vec![],
                vec![],
            ),
        )
        .expect("save snapshot");
    }

    let deleted = prune_snapshots(&graph, 3).expect("prune should succeed");
    assert_eq!(deleted, 2, "should delete 2 of 5 snapshots to keep 3");

    // Verify only 3 remain.
    let count = graph.memory_count().expect("memory_count should succeed");
    assert_eq!(count, 3, "graph should have exactly 3 memories after prune");

    // The newest should still be loadable.
    let latest = load_latest_snapshot(&graph)
        .expect("load should succeed")
        .expect("should return Some");
    assert_eq!(
        latest.session_id, "sess-4",
        "newest snapshot should survive pruning"
    );
}

#[test]
fn persistence_prune_no_op_when_under_limit() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    save_snapshot(&graph, &make_snapshot("s1", 0.7, 1, vec![], vec![])).expect("save 1");
    save_snapshot(&graph, &make_snapshot("s2", 0.8, 0, vec![], vec![])).expect("save 2");

    let deleted = prune_snapshots(&graph, 5).expect("prune should succeed");
    assert_eq!(
        deleted, 0,
        "prune should delete nothing when count <= limit"
    );

    let count = graph.memory_count().expect("memory_count");
    assert_eq!(count, 2, "both snapshots should still exist");
}

#[test]
fn persistence_compute_trends_empty() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    let trends = compute_trends(&graph, 10).expect("compute_trends should succeed on empty graph");

    assert_eq!(trends.total_sessions, 0, "total_sessions should be 0");
    assert!(
        (trends.avg_confidence_end - 0.0).abs() < f32::EPSILON,
        "avg_confidence_end should be 0.0"
    );
    assert!(
        (trends.avg_corrections_per_session - 0.0).abs() < f32::EPSILON,
        "avg_corrections_per_session should be 0.0"
    );
    assert_eq!(
        trends.correction_trend,
        TrendDirection::Stable,
        "trend should be Stable on empty graph"
    );
    assert!(
        trends.persistent_struggles.is_empty(),
        "persistent_struggles should be empty"
    );
    assert!(
        trends.reliable_successes.is_empty(),
        "reliable_successes should be empty"
    );
}

#[test]
fn persistence_compute_trends_basic() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    // Save 5 snapshots with known confidence_end and corrections.
    let confidences = [0.6, 0.7, 0.8, 0.9, 1.0];
    let corrections = [2, 4, 2, 4, 2];

    for i in 0..5 {
        save_snapshot(
            &graph,
            &make_snapshot(
                &format!("trend-{i}"),
                confidences[i],
                corrections[i],
                vec![],
                vec![],
            ),
        )
        .expect("save snapshot");
    }

    let trends = compute_trends(&graph, 10).expect("compute_trends should succeed");

    assert_eq!(trends.total_sessions, 5, "should have 5 sessions");

    let expected_avg_confidence = (0.6 + 0.7 + 0.8 + 0.9 + 1.0) / 5.0;
    assert!(
        (trends.avg_confidence_end - expected_avg_confidence).abs() < 0.01,
        "avg_confidence_end: expected ~{expected_avg_confidence}, got {}",
        trends.avg_confidence_end
    );

    let expected_avg_corrections = (2.0 + 4.0 + 2.0 + 4.0 + 2.0) / 5.0;
    assert!(
        (trends.avg_corrections_per_session - expected_avg_corrections).abs() < 0.01,
        "avg_corrections_per_session: expected ~{expected_avg_corrections}, got {}",
        trends.avg_corrections_per_session
    );
}

#[test]
fn persistence_compute_trends_direction() {
    // Rising: first half low corrections, second half high corrections.
    {
        let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");
        let corrections = [1, 1, 10, 10, 10];
        for (i, &c) in corrections.iter().enumerate() {
            save_snapshot(
                &graph,
                &make_snapshot(&format!("rise-{i}"), 0.7, c, vec![], vec![]),
            )
            .expect("save snapshot");
        }
        let trends = compute_trends(&graph, 10).expect("compute_trends");
        assert_eq!(
            trends.correction_trend,
            TrendDirection::Rising,
            "corrections going from low to high should be Rising"
        );
    }

    // Falling: first half high corrections, second half low corrections.
    {
        let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");
        let corrections = [10, 10, 1, 1, 1];
        for (i, &c) in corrections.iter().enumerate() {
            save_snapshot(
                &graph,
                &make_snapshot(&format!("fall-{i}"), 0.7, c, vec![], vec![]),
            )
            .expect("save snapshot");
        }
        let trends = compute_trends(&graph, 10).expect("compute_trends");
        assert_eq!(
            trends.correction_trend,
            TrendDirection::Falling,
            "corrections going from high to low should be Falling"
        );
    }
}

#[test]
fn persistence_persistent_struggles() {
    let graph = MemoryGraph::in_memory().expect("failed to create in-memory graph");

    // "shell execution" appears in 4 of 5 snapshots.
    let struggles_with_shell = vec!["shell execution".to_string()];
    let no_struggles: Vec<String> = vec![];

    for i in 0..5 {
        let s = if i != 2 {
            struggles_with_shell.clone()
        } else {
            no_struggles.clone()
        };
        save_snapshot(
            &graph,
            &make_snapshot(&format!("struggle-{i}"), 0.7, 1, s, vec![]),
        )
        .expect("save snapshot");
    }

    let trends = compute_trends(&graph, 10).expect("compute_trends");

    let shell_entry = trends
        .persistent_struggles
        .iter()
        .find(|(area, _)| area == "shell execution");

    assert!(
        shell_entry.is_some(),
        "\"shell execution\" should appear in persistent_struggles; got: {:?}",
        trends.persistent_struggles
    );

    let (_, count) = shell_entry.expect("already checked is_some");
    assert_eq!(
        *count, 4,
        "\"shell execution\" should have count 4, got {count}"
    );
}

#[test]
fn persistence_generate_briefing_format() {
    let trends = PersonalityTrends {
        avg_confidence_end: 0.82,
        avg_corrections_per_session: 2.5,
        correction_trend: TrendDirection::Falling,
        persistent_struggles: vec![("shell execution".to_string(), 4)],
        reliable_successes: vec![("code review".to_string(), 3)],
        total_sessions: 5,
    };

    let last = make_snapshot(
        "brief-sess",
        0.85,
        2,
        vec!["shell execution".into()],
        vec!["code review".into()],
    );

    let briefing = generate_briefing(&trends, &last);

    assert!(
        briefing.contains("<personality_briefing>"),
        "briefing should contain opening XML tag"
    );
    assert!(
        briefing.contains("</personality_briefing>"),
        "briefing should contain closing XML tag"
    );
    assert!(
        briefing.contains("5 total"),
        "briefing should mention session count; got:\n{briefing}"
    );
    // The briefing includes "confidence X.X -> Y.Y" from the last snapshot's
    // stats. confidence_start is 0.7; confidence_end is 0.85 which may round
    // to 0.8 or 0.9 at one decimal place depending on f32 representation.
    assert!(
        briefing.contains("confidence"),
        "briefing should mention confidence; got:\n{briefing}"
    );
    assert!(
        briefing.contains("0.7"),
        "briefing should contain confidence_start (0.7); got:\n{briefing}"
    );
    assert!(
        briefing.contains("falling"),
        "briefing should contain trend word 'falling'; got:\n{briefing}"
    );
}

#[test]
fn persistence_generate_briefing_empty() {
    let trends = PersonalityTrends {
        avg_confidence_end: 0.0,
        avg_corrections_per_session: 0.0,
        correction_trend: TrendDirection::Stable,
        persistent_struggles: vec![],
        reliable_successes: vec![],
        total_sessions: 0,
    };

    let mut last = make_snapshot("empty-sess", 0.7, 0, vec![], vec![]);
    last.rule_scores.clear(); // empty rules too

    let briefing = generate_briefing(&trends, &last);

    // With empty struggles, successes, and rules the briefing should show "none".
    let none_count = briefing.matches("none").count();
    assert!(
        none_count >= 3,
        "briefing should contain 'none' at least 3 times (struggles, successes, rules); \
         found {none_count} in:\n{briefing}"
    );
}
