#[cfg(test)]
mod tests_eval_run_store {
    use super::*;
    use std::collections::HashMap;

    /// Build a minimal JepaEvalRunRecord for testing.
    fn make_record(run_id: &str, candidate_id: &str) -> crate::jepa::JepaEvalRunRecord {
        crate::jepa::JepaEvalRunRecord {
            run_id: run_id.to_string(),
            candidate_id: candidate_id.to_string(),
            corpus_fingerprint: None,
            training_corpus_fingerprint: None,
            mode: crate::jepa::RuntimeEvalMode::Quick,
            backend: crate::jepa::JepaEvalBackendKind::Cpu,
            status: crate::jepa::EvalRunStatus::Running,
            current_stage: crate::jepa::EvalRunStage::Tier0,
            stage_timings: HashMap::new(),
            baseline_skipped: false,
            skipped_reason: None,
            pid: std::process::id(),
            host: "test-host".to_string(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
            rows_total: 0,
            rows_completed: 0,
            transitions_total: 0,
            transitions_completed: 0,
            embeddings_total: 0,
            embeddings_completed: 0,
            cache_hits: 0,
            cache_misses: 0,
            backend_parity_examples: 0,
            failure_reason: None,
            partial_gates: serde_json::Value::Null,
            result_paths: HashMap::new(),
        }
    }

    /// Write a run record then read it back; the run_id must survive the roundtrip.
    #[test]
    fn atomic_write_produces_valid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JepaEvalRunStore::new(tmp.path().to_path_buf()).unwrap();
        let run_id = JepaEvalRunStore::generate_run_id();
        let record = make_record(&run_id, "candidate-A");

        store.write_run(&record).unwrap();
        let read_back = store.read_run(&run_id).unwrap();

        assert_eq!(read_back.run_id, run_id);
        assert_eq!(read_back.candidate_id, "candidate-A");
    }

    /// Acquiring the same candidate lock twice must fail with "already in progress".
    #[test]
    fn o_excl_rejects_second_acquire() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JepaEvalRunStore::new(tmp.path().to_path_buf()).unwrap();

        let run_id1 = JepaEvalRunStore::generate_run_id();
        let run_id2 = JepaEvalRunStore::generate_run_id();

        let _guard = store
            .acquire_candidate_lock("candidate-B", &run_id1)
            .expect("first acquire must succeed");

        let result = store.acquire_candidate_lock("candidate-B", &run_id2);
        assert!(result.is_err(), "second acquire must fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already in progress"),
            "error message must contain 'already in progress'; got: {msg}"
        );
    }

    /// A lock file holding a dead pid (999999) with an expired heartbeat must
    /// be reclaimed when staleness is requested.
    ///
    /// pid 999999 is almost certainly not alive.  We use budget=0 (unlimited)
    /// and stale_heartbeat_ms=0 so that the heartbeat check also fires, giving
    /// the test two independent reclaim triggers.
    #[test]
    fn stale_dead_pid_reclaimed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JepaEvalRunStore::new(tmp.path().to_path_buf()).unwrap();

        let run_id = JepaEvalRunStore::generate_run_id();
        let candidate_id = "candidate-C";

        // Write a lock record with a very unlikely-alive pid.
        let dead_lock = CandidateLockRecord {
            pid: 999_999,
            run_id: run_id.clone(),
            host: "test-host".to_string(),
            acquired_at: Utc::now(),
        };
        std::fs::write(
            store.lock_path(candidate_id),
            serde_json::to_string(&dead_lock).unwrap(),
        )
        .unwrap();

        // Also write a run record with an ancient updated_at so heartbeat fires.
        let mut record = make_record(&run_id, candidate_id);
        record.pid = 999_999;
        // Set updated_at far in the past so heartbeat_age > stale_heartbeat_ms=0.
        record.updated_at =
            chrono::DateTime::from_timestamp(0, 0).unwrap_or_else(Utc::now);
        store.write_run(&record).unwrap();

        // budget_ms=0 (unlimited) + stale_heartbeat_ms=0 (immediate expiry).
        let reclaimed = store
            .reclaim_stale_lock_if_applicable(candidate_id, 0, 0)
            .unwrap();

        assert!(reclaimed, "stale lock with dead pid must be reclaimed");
        assert!(
            !store.lock_path(candidate_id).exists(),
            "lock file must be removed after reclamation"
        );
    }

    /// Writing a cancel sentinel then checking existence, then removing it.
    #[test]
    fn cancel_sentinel_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JepaEvalRunStore::new(tmp.path().to_path_buf()).unwrap();
        let run_id = JepaEvalRunStore::generate_run_id();

        assert!(
            !store.cancel_sentinel_exists(&run_id),
            "sentinel must not exist before write"
        );

        store.write_cancel_sentinel(&run_id).unwrap();
        assert!(
            store.cancel_sentinel_exists(&run_id),
            "sentinel must exist after write"
        );

        // Remove it manually (simulates the worker consuming it).
        std::fs::remove_file(store.cancel_path(&run_id)).unwrap();
        assert!(
            !store.cancel_sentinel_exists(&run_id),
            "sentinel must not exist after removal"
        );
    }
}
