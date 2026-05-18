// TASK-JEVAL-014 — CLI status commands for JEPA eval runs
//
// Implements:
//   render_eval_jepa_status  — print all JepaEvalRunRecord fields, human-readable
//   render_eval_jepa_runs    — tabular list sorted by started_at desc, --limit N
//   render_eval_jepa_cancel  — write sentinel; reject if run already terminal
//
// PRD-006C §6.2/§10; DEC-JEVAL-11 (include!() module pattern)

use anyhow::Context;
use archon_world_model::jepa::{EvalRunStatus, JepaEvalRunStore};

/// Print all fields from a `JepaEvalRunRecord` in human-readable form.
///
/// Returns an actionable error when `run_id` does not exist, suggesting
/// `archon world eval-jepa-runs` to list all known runs.
pub(super) fn render_eval_jepa_status(root: &Path, run_id: &str) -> Result<String> {
    let run_dir = root.join("jepa/eval-runs");
    let store = JepaEvalRunStore::new(run_dir)?;
    let record = store.read_run(run_id).with_context(|| {
        format!(
            "Run {run_id} not found. \
             List all runs: archon world eval-jepa-runs"
        )
    })?;

    let mut out = String::new();
    out += &format!("Run ID:         {}\n", record.run_id);
    out += &format!("Candidate:      {}\n", record.candidate_id);
    out += &format!("Mode:           {:?}\n", record.mode);
    out += &format!("Backend:        {:?}\n", record.backend);
    out += &format!("Status:         {:?}\n", record.status);
    out += &format!("Stage:          {:?}\n", record.current_stage);
    out += &format!("Started:        {}\n", record.started_at);
    out += &format!("Updated:        {}\n", record.updated_at);
    if let Some(c) = record.completed_at {
        out += &format!("Completed:      {}\n", c);
    }
    out += &format!(
        "Rows:           {} / {}\n",
        record.rows_completed, record.rows_total
    );
    out += &format!("Transitions:    {}\n", record.transitions_total);
    out += &format!(
        "Embeddings:     {} / {}\n",
        record.embeddings_completed, record.embeddings_total
    );
    out += &format!("Cache hits:     {}\n", record.cache_hits);
    out += &format!("Cache misses:   {}\n", record.cache_misses);
    out += &format!("Baseline skip:  {}\n", record.baseline_skipped);
    if let Some(reason) = &record.skipped_reason {
        out += &format!("Skip reason:    {}\n", reason);
    }
    out += "Stage timings:\n";
    for (stage, ms) in &record.stage_timings {
        out += &format!("  {stage}: {ms}ms\n");
    }
    if let Some(fail) = &record.failure_reason {
        out += &format!("Failure:        {}\n", fail);
    }
    Ok(out)
}

/// List up to `limit` recent eval runs, sorted newest-first by `started_at`.
///
/// Returns `"No eval runs found.\n"` when the run directory is empty or
/// contains no valid run records.
pub(super) fn render_eval_jepa_runs(root: &Path, limit: usize) -> Result<String> {
    let run_dir = root.join("jepa/eval-runs");
    let store = JepaEvalRunStore::new(run_dir)?;
    let records = store.list_runs(limit)?;

    if records.is_empty() {
        return Ok("No eval runs found.\n".to_string());
    }

    let mut out = format!(
        "{:<40} {:<40} {:<10} {:<8} {:<12} {}\n",
        "Run ID", "Candidate", "Mode", "Backend", "Status", "Started"
    );
    out += &"-".repeat(120);
    out += "\n";
    for r in &records {
        out += &format!(
            "{:<40} {:<40} {:<10} {:<8} {:<12} {}\n",
            r.run_id,
            r.candidate_id,
            format!("{:?}", r.mode),
            format!("{:?}", r.backend),
            format!("{:?}", r.status),
            r.started_at.format("%Y-%m-%d %H:%M:%S"),
        );
    }
    Ok(out)
}

/// Write a cancel sentinel for `run_id` and print a confirmation.
///
/// If the run is already in a terminal state (`Completed`, `Failed`,
/// `Cancelled`, or `Stale`), returns an informational message without
/// writing the sentinel file.
///
/// Returns an actionable error if `run_id` does not exist.
pub(super) fn render_eval_jepa_cancel(root: &Path, run_id: &str) -> Result<String> {
    let run_dir = root.join("jepa/eval-runs");
    let store = JepaEvalRunStore::new(run_dir)?;

    // Verify run exists and read its current status.
    let record = store
        .read_run(run_id)
        .with_context(|| format!("Run {run_id} not found."))?;

    match record.status {
        EvalRunStatus::Completed | EvalRunStatus::Failed | EvalRunStatus::Cancelled
        | EvalRunStatus::Stale => {
            return Ok(format!(
                "Run {run_id} is already {:?}; nothing to cancel.\n",
                record.status
            ));
        }
        EvalRunStatus::Pending | EvalRunStatus::Running | EvalRunStatus::Paused => {}
    }

    store.write_cancel_sentinel(run_id)?;
    Ok(format!(
        "Cancellation requested for run {run_id}.\n\
         The worker will observe the cancel signal within 100 processed rows.\n\
         Check status: archon world eval-jepa-status {run_id}\n"
    ))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests_eval_status {
    use std::collections::HashMap;

    use archon_world_model::jepa::{
        EvalRunStatus, EvalRunStage, JepaEvalBackendKind, JepaEvalRunRecord, JepaEvalRunStore,
        RuntimeEvalMode,
    };
    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;

    /// Build a minimal `JepaEvalRunRecord` for test use.
    fn make_record(
        run_id: &str,
        candidate_id: &str,
        status: EvalRunStatus,
    ) -> JepaEvalRunRecord {
        JepaEvalRunRecord {
            run_id: run_id.to_string(),
            candidate_id: candidate_id.to_string(),
            corpus_fingerprint: None,
            training_corpus_fingerprint: None,
            mode: RuntimeEvalMode::Quick,
            backend: JepaEvalBackendKind::Cpu,
            status,
            current_stage: EvalRunStage::Tier0,
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

    /// A non-existent run-id must return an error that contains "not found"
    /// and hints the user toward `eval-jepa-runs`.
    #[test]
    fn status_for_missing_run_returns_actionable_error() {
        let tmp = TempDir::new().unwrap();
        let result = render_eval_jepa_status(tmp.path(), "jeval-does-not-exist");
        assert!(result.is_err(), "must error for unknown run-id");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found"),
            "error must say 'not found'; got: {msg}"
        );
        assert!(
            msg.contains("eval-jepa-runs"),
            "error must hint 'eval-jepa-runs'; got: {msg}"
        );
    }

    /// Calling cancel on a `Completed` run must NOT write a sentinel file.
    #[test]
    fn cancel_on_completed_run_does_not_write_sentinel() {
        let tmp = TempDir::new().unwrap();
        let run_dir = tmp.path().join("jepa/eval-runs");
        let store = JepaEvalRunStore::new(run_dir.clone()).unwrap();

        let run_id = JepaEvalRunStore::generate_run_id();
        let record = make_record(&run_id, "candidate-X", EvalRunStatus::Completed);
        store.write_run(&record).unwrap();

        let result = render_eval_jepa_cancel(tmp.path(), &run_id).unwrap();
        assert!(
            result.contains("already"),
            "output must say 'already'; got: {result}"
        );
        assert!(
            !store.cancel_sentinel_exists(&run_id),
            "sentinel must NOT be written for a completed run"
        );
    }

    /// An empty eval-runs directory must produce "No eval runs found."
    #[test]
    fn runs_list_returns_empty_string_when_no_runs() {
        let tmp = TempDir::new().unwrap();
        let result = render_eval_jepa_runs(tmp.path(), 10).unwrap();
        assert_eq!(
            result, "No eval runs found.\n",
            "empty dir must return sentinel message; got: {result:?}"
        );
    }
}
