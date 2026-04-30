//! Tests for `archon_pipeline::session` — session management, checkpoint,
//! and crash-recovery logic.
//!
//! Gate 1 (tests-written-first): these tests are written before the
//! implementation exists, so they will not compile until the `session` module
//! is created.

use archon_pipeline::runner::PipelineType;
use archon_pipeline::session::{
    PipelineCheckpoint, SessionStatus, abort, checkpoint, detect_interrupted, list_sessions,
    mark_completed, new_session, record_agent_completion, resume,
};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a fresh temp directory to act as `state_dir`.
fn tmp_state_dir() -> TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

// ---------------------------------------------------------------------------
// 1. new_session basics
// ---------------------------------------------------------------------------

#[test]
fn test_new_session_has_uuid_and_running_status() {
    let cp = new_session(PipelineType::Coding, "implement auth module");

    // session_id should be a valid UUID (36 chars with hyphens).
    assert_eq!(cp.session_id.len(), 36, "session_id should be a UUID");
    assert!(
        cp.session_id.chars().filter(|c| *c == '-').count() == 4,
        "UUID should have 4 hyphens"
    );

    assert_eq!(cp.status, SessionStatus::Running);
    assert!(cp.completed_agents.is_empty());
    assert_eq!(cp.pipeline_type, PipelineType::Coding);
    assert_eq!(cp.task, "implement auth module");
    assert_eq!(cp.total_cost_usd, 0.0);
    assert!(cp.current_agent_key.is_none());
    assert!(cp.rlm_snapshot_path.is_none());
}

// ---------------------------------------------------------------------------
// 2. Serialization round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_checkpoint_serialization_round_trip() {
    let mut cp = new_session(PipelineType::Research, "survey LLM routing");

    // Add a completed agent so we exercise nested serialization.
    record_agent_completion(
        &mut cp,
        "requirement-extractor",
        "some output text",
        0.85,
        0.012,
    );

    let json = serde_json::to_string_pretty(&cp).expect("serialize failed");
    let deserialized: PipelineCheckpoint = serde_json::from_str(&json).expect("deserialize failed");

    assert_eq!(deserialized.session_id, cp.session_id);
    assert_eq!(deserialized.pipeline_type, cp.pipeline_type);
    assert_eq!(deserialized.task, cp.task);
    assert_eq!(deserialized.status, cp.status);
    assert_eq!(deserialized.completed_agents.len(), 1);
    assert_eq!(
        deserialized.completed_agents[0].agent_key,
        "requirement-extractor"
    );
}

// ---------------------------------------------------------------------------
// 3. checkpoint writes to disk
// ---------------------------------------------------------------------------

#[test]
fn test_checkpoint_writes_to_disk() {
    let dir = tmp_state_dir();
    let cp = new_session(PipelineType::Learning, "learn Rust macros");

    checkpoint(&cp, dir.path()).expect("checkpoint failed");

    let path = dir
        .path()
        .join(".pipeline-state")
        .join(&cp.session_id)
        .join("checkpoint.json");
    assert!(path.exists(), "checkpoint file should exist on disk");

    // The file should contain valid JSON.
    let contents = fs::read_to_string(&path).expect("read checkpoint file");
    let parsed: PipelineCheckpoint =
        serde_json::from_str(&contents).expect("checkpoint file is not valid JSON");
    assert_eq!(parsed.session_id, cp.session_id);
}

// ---------------------------------------------------------------------------
// 4. Atomic write — no leftover .tmp files
// ---------------------------------------------------------------------------

#[test]
fn test_checkpoint_atomic_write() {
    let dir = tmp_state_dir();
    let cp = new_session(PipelineType::Coding, "add error handling");

    checkpoint(&cp, dir.path()).expect("checkpoint failed");

    let session_dir = dir.path().join(".pipeline-state").join(&cp.session_id);

    // Walk the session directory; there should be no .tmp files left behind.
    for entry in fs::read_dir(&session_dir).expect("read session dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        assert!(
            !name_str.ends_with(".tmp"),
            "leftover temp file found: {name_str}"
        );
    }

    // The checkpoint.json file itself should exist.
    assert!(session_dir.join("checkpoint.json").exists());
}

// ---------------------------------------------------------------------------
// 5. detect_interrupted finds Running sessions only
// ---------------------------------------------------------------------------

#[test]
fn test_detect_interrupted_finds_running_sessions() {
    let dir = tmp_state_dir();

    let running = new_session(PipelineType::Coding, "running task");
    checkpoint(&running, dir.path()).expect("checkpoint running");

    let mut completed = new_session(PipelineType::Research, "completed task");
    mark_completed(&mut completed, dir.path()).expect("mark completed");

    let interrupted = detect_interrupted(dir.path()).expect("detect_interrupted");
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].session_id, running.session_id);
    assert_eq!(interrupted[0].status, SessionStatus::Running);
}

// ---------------------------------------------------------------------------
// 6. detect_interrupted on empty directory
// ---------------------------------------------------------------------------

#[test]
fn test_detect_interrupted_empty_dir() {
    let dir = tmp_state_dir();
    let interrupted = detect_interrupted(dir.path()).expect("detect_interrupted");
    assert!(interrupted.is_empty());
}

// ---------------------------------------------------------------------------
// 7. resume loads and updates status
// ---------------------------------------------------------------------------

#[test]
fn test_resume_loads_and_updates_status() {
    let dir = tmp_state_dir();

    let mut cp = new_session(PipelineType::Coding, "resume me");
    record_agent_completion(&mut cp, "task-analyzer", "analysis output", 0.9, 0.005);
    checkpoint(&cp, dir.path()).expect("checkpoint");

    let resumed = resume(&cp.session_id, dir.path()).expect("resume");
    assert_eq!(resumed.status, SessionStatus::Running);
    assert_eq!(resumed.completed_agents.len(), 1);
    assert_eq!(resumed.completed_agents[0].agent_key, "task-analyzer");
    assert_eq!(resumed.task, "resume me");
}

// ---------------------------------------------------------------------------
// 8. abort sets Failed status
// ---------------------------------------------------------------------------

#[test]
fn test_abort_sets_failed_status() {
    let dir = tmp_state_dir();

    let cp = new_session(PipelineType::Research, "abort me");
    checkpoint(&cp, dir.path()).expect("checkpoint");

    abort(&cp.session_id, dir.path()).expect("abort");

    // Reload from disk and verify.
    let path = dir
        .path()
        .join(".pipeline-state")
        .join(&cp.session_id)
        .join("checkpoint.json");
    let contents = fs::read_to_string(&path).expect("read checkpoint");
    let reloaded: PipelineCheckpoint = serde_json::from_str(&contents).expect("parse checkpoint");
    assert_eq!(reloaded.status, SessionStatus::Failed);
}

// ---------------------------------------------------------------------------
// 9. list_sessions returns all
// ---------------------------------------------------------------------------

#[test]
fn test_list_sessions_returns_all() {
    let dir = tmp_state_dir();

    let s1 = new_session(PipelineType::Coding, "task one");
    checkpoint(&s1, dir.path()).expect("checkpoint s1");

    let mut s2 = new_session(PipelineType::Research, "task two");
    mark_completed(&mut s2, dir.path()).expect("mark s2 completed");

    let s3 = new_session(PipelineType::Kb, "task three");
    checkpoint(&s3, dir.path()).expect("checkpoint s3");

    let summaries = list_sessions(dir.path()).expect("list_sessions");
    assert_eq!(summaries.len(), 3);

    // Collect session IDs.
    let ids: Vec<&str> = summaries.iter().map(|s| s.session_id.as_str()).collect();
    assert!(ids.contains(&s1.session_id.as_str()));
    assert!(ids.contains(&s2.session_id.as_str()));
    assert!(ids.contains(&s3.session_id.as_str()));

    // Verify the completed session summary reports correct status.
    let completed_summary = summaries
        .iter()
        .find(|s| s.session_id == s2.session_id)
        .expect("s2 summary missing");
    assert_eq!(completed_summary.status, SessionStatus::Completed);
}

// ---------------------------------------------------------------------------
// 10. record_agent_completion
// ---------------------------------------------------------------------------

#[test]
fn test_record_agent_completion() {
    let mut cp = new_session(PipelineType::Coding, "agent completions");

    let agent1 = record_agent_completion(
        &mut cp,
        "requirement-extractor",
        "extracted requirements text",
        0.92,
        0.015,
    );
    let agent2 = record_agent_completion(
        &mut cp,
        "architecture-planner",
        "architecture plan output",
        0.88,
        0.023,
    );

    assert_eq!(cp.completed_agents.len(), 2);

    // First agent.
    assert_eq!(agent1.agent_key, "requirement-extractor");
    assert!(
        !agent1.output_hash.is_empty(),
        "output_hash should be non-empty"
    );
    assert!((agent1.quality_score - 0.92).abs() < f64::EPSILON);
    assert!((agent1.cost_usd - 0.015).abs() < f64::EPSILON);

    // Second agent.
    assert_eq!(agent2.agent_key, "architecture-planner");
    assert!(
        !agent2.output_hash.is_empty(),
        "output_hash should be non-empty"
    );
    assert!((agent2.quality_score - 0.88).abs() < f64::EPSILON);
    assert!((agent2.cost_usd - 0.023).abs() < f64::EPSILON);

    // Total cost should be accumulated.
    let expected_cost = 0.015 + 0.023;
    assert!(
        (cp.total_cost_usd - expected_cost).abs() < f64::EPSILON,
        "total_cost_usd should be sum of agent costs"
    );

    // Output hashes for different outputs should be different.
    assert_ne!(
        agent1.output_hash, agent2.output_hash,
        "different outputs should produce different hashes"
    );
}
