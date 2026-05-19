// TASK-JEVAL-013 — ForegroundProgressReporter
//
// Foreground progress reporter for long-running JEPA evals.
// Emits stage-boundary stdout updates and structured tracing log fields.
// Also enforces the cooperative max_runtime_ms deadline.
//
// Types are flat under crate::jepa::* per DEC-JEVAL-11.
// The reporter does NOT write to JepaEvalRunStore directly — callers do that.
// It only updates its internal run_record state and emits stdout/logs.

// ---------------------------------------------------------------------------
// TickOutcome
// ---------------------------------------------------------------------------

/// Outcome returned by `ForegroundProgressReporter::tick()`.
#[derive(Debug, PartialEq)]
pub enum TickOutcome {
    /// Continue normally.
    Continue,
    /// Cooperative deadline exceeded; caller should set status = Paused.
    DeadlineExceeded { elapsed_ms: u64 },
    /// Cancel sentinel was detected; caller should set status = Cancelled.
    Cancelled,
}

// ---------------------------------------------------------------------------
// ForegroundProgressReporter
// ---------------------------------------------------------------------------

/// Foreground progress reporter for long-running JEPA evals.
///
/// Emits stage-boundary stdout updates and structured tracing log fields.
/// Also enforces the cooperative `max_runtime_ms` deadline.
///
/// The reporter maintains a local copy of the `JepaEvalRunRecord` for display
/// purposes, but does NOT write to `JepaEvalRunStore` — callers are responsible
/// for persisting the record at appropriate checkpoints.
pub struct ForegroundProgressReporter {
    pub candidate_id: String,
    pub run_id: String,
    run_start: Instant,
    stage_start: Instant,
    pub run_record: crate::jepa::JepaEvalRunRecord,
    /// 0 = unlimited.
    max_runtime_ms: u64,
    progress_interval_rows: usize,
    rows_since_last_report: usize,
}

impl ForegroundProgressReporter {
    /// Construct a new reporter. Timestamps are captured at construction time.
    pub fn new(
        run_record: crate::jepa::JepaEvalRunRecord,
        max_runtime_ms: u64,
        progress_interval_rows: usize,
    ) -> Self {
        let now = Instant::now();
        let candidate_id = run_record.candidate_id.clone();
        let run_id = run_record.run_id.clone();
        Self {
            candidate_id,
            run_id,
            run_start: now,
            stage_start: now,
            run_record,
            max_runtime_ms,
            progress_interval_rows: progress_interval_rows.max(1),
            rows_since_last_report: 0,
        }
    }

    // -----------------------------------------------------------------------
    // advance_stage
    // -----------------------------------------------------------------------

    /// Call at each stage boundary (tier0 done, tier1 done, etc.).
    ///
    /// Records the wall-clock ms for the completing stage, transitions the
    /// internal run record to `new_stage`, resets the stage timer, prints a
    /// stage-boundary line to stdout, and emits a structured tracing log.
    pub fn advance_stage(&mut self, new_stage: crate::jepa::EvalRunStage) -> Result<()> {
        let stage_elapsed_ms = self.stage_start.elapsed().as_millis() as u64;
        let elapsed_ms = self.run_start.elapsed().as_millis() as u64;

        // Record timing for the stage we are leaving.
        let leaving_stage_name = stage_name_str(self.run_record.current_stage);
        self.run_record
            .stage_timings
            .insert(leaving_stage_name.clone(), stage_elapsed_ms);

        // Transition.
        self.run_record.current_stage = new_stage;
        self.stage_start = Instant::now();

        // Stage-boundary stdout line.
        let new_stage_str = stage_name_str(new_stage);
        println!(
            "\n--- Stage: {new_stage_str} (prev: {leaving_stage_name} {stage_elapsed_ms}ms, total elapsed: {elapsed_str}) ---",
            elapsed_str = format_mmss(elapsed_ms / 1000),
        );

        // Structured log.
        self.emit_structured_log(elapsed_ms, 0);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // tick
    // -----------------------------------------------------------------------

    /// Call at each row-level progress checkpoint.
    ///
    /// Updates internal counters on `run_record`. Emits a progress line to
    /// stdout and a structured tracing log every `progress_interval_rows` rows.
    ///
    /// Returns `TickOutcome::DeadlineExceeded` if `max_runtime_ms` is exceeded
    /// (caller should set `status = Paused`).
    ///
    /// The cancel sentinel is NOT checked here — call `check_cancel_sentinel`
    /// separately when you hold a `JepaEvalRunStore` reference.
    pub fn tick(
        &mut self,
        rows_completed: usize,
        transitions_completed: usize,
        embeddings_completed: usize,
        cache_hits: usize,
        cache_misses: usize,
    ) -> Result<TickOutcome> {
        // Update internal record counters.
        self.run_record.rows_completed = rows_completed;
        self.run_record.transitions_completed = transitions_completed;
        self.run_record.embeddings_completed = embeddings_completed;
        self.run_record.cache_hits = cache_hits;
        self.run_record.cache_misses = cache_misses;

        // Check cooperative deadline first (before any I/O).
        if self.max_runtime_ms > 0 {
            let elapsed_ms = self.run_start.elapsed().as_millis();
            if elapsed_ms > self.max_runtime_ms as u128 {
                let elapsed_ms = elapsed_ms as u64;
                return Ok(TickOutcome::DeadlineExceeded { elapsed_ms });
            }
        }

        // Throttle stdout/log output to every progress_interval_rows.
        self.rows_since_last_report += 1;
        if self.rows_since_last_report < self.progress_interval_rows {
            return Ok(TickOutcome::Continue);
        }
        self.rows_since_last_report = 0;

        let elapsed_ms = self.run_start.elapsed().as_millis() as u64;
        let stage_elapsed_ms = self.stage_start.elapsed().as_millis() as u64;

        self.print_progress();
        self.emit_structured_log(elapsed_ms, stage_elapsed_ms);

        Ok(TickOutcome::Continue)
    }

    // -----------------------------------------------------------------------
    // check_cancel_sentinel
    // -----------------------------------------------------------------------

    /// Check whether the cancel sentinel file exists for this run.
    ///
    /// Returns `true` if the sentinel is present. The caller is responsible
    /// for setting `run_record.status = Cancelled` and persisting the record.
    pub fn check_cancel_sentinel(&self, store: &crate::jepa::JepaEvalRunStore) -> bool {
        store.cancel_sentinel_exists(&self.run_record.run_id)
    }

    // -----------------------------------------------------------------------
    // print_progress (private)
    // -----------------------------------------------------------------------

    /// Print the current progress block to stdout.
    ///
    /// Output format matches PRD §10 (mm:ss elapsed, F-MED-03 fix):
    ///
    /// ```text
    /// Stage: baseline_embed
    /// Rows: 4500 / 17105
    /// Transitions: 4490 / 17055
    /// Embeddings: 13470 / 51165
    /// Cache hits: 9120
    /// Cache misses: 4350
    /// Backend: fastembed cpu
    /// Elapsed: 04:12  (baseline_embed 03:58)
    /// ```
    fn print_progress(&self) {
        let elapsed_secs = self.run_start.elapsed().as_secs();
        let stage_elapsed_secs = self.stage_start.elapsed().as_secs();
        let stage_str = stage_name_str(self.run_record.current_stage);
        let backend_str = backend_name_str(self.run_record.backend);

        println!(
            "Stage: {stage}\n\
             Rows: {rc} / {rt}\n\
             Transitions: {tc} / {tt}\n\
             Embeddings: {ec} / {et}\n\
             Cache hits: {ch}\n\
             Cache misses: {cm}\n\
             Backend: {backend}\n\
             Elapsed: {elapsed}  ({stage_name} {stage_elapsed})",
            stage = stage_str,
            rc = self.run_record.rows_completed,
            rt = self.run_record.rows_total,
            tc = self.run_record.transitions_completed,
            tt = self.run_record.transitions_total,
            ec = self.run_record.embeddings_completed,
            et = self.run_record.embeddings_total,
            ch = self.run_record.cache_hits,
            cm = self.run_record.cache_misses,
            backend = backend_str,
            elapsed = format_mmss(elapsed_secs),
            stage_name = stage_str,
            stage_elapsed = format_mmss(stage_elapsed_secs),
        );
    }

    // -----------------------------------------------------------------------
    // emit_structured_log (private)
    // -----------------------------------------------------------------------

    /// Emit a structured tracing log with all 13 required fields.
    fn emit_structured_log(&self, elapsed_ms: u64, stage_elapsed_ms: u64) {
        let stage_str = stage_name_str(self.run_record.current_stage);
        let backend_str = backend_name_str(self.run_record.backend);

        tracing::info!(
            candidate_id = self.run_record.candidate_id.as_str(),
            run_id = self.run_record.run_id.as_str(),
            stage = stage_str.as_str(),
            rows_total = self.run_record.rows_total,
            rows_completed = self.run_record.rows_completed,
            transitions_total = self.run_record.transitions_total,
            transitions_completed = self.run_record.transitions_completed,
            embeddings_total = self.run_record.embeddings_total,
            embeddings_completed = self.run_record.embeddings_completed,
            cache_hits = self.run_record.cache_hits,
            cache_misses = self.run_record.cache_misses,
            backend = backend_str.as_str(),
            elapsed_ms = elapsed_ms,
            stage_elapsed_ms = stage_elapsed_ms,
            "jepa_eval_progress"
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format seconds as mm:ss per PRD §10 (F-MED-03: NOT Debug/`?` format).
fn format_mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Lowercase snake_case string for an `EvalRunStage`.
fn stage_name_str(stage: crate::jepa::EvalRunStage) -> String {
    match stage {
        crate::jepa::EvalRunStage::Tier0 => "tier0".to_string(),
        crate::jepa::EvalRunStage::Tier1 => "tier1".to_string(),
        crate::jepa::EvalRunStage::BaselineEmbed => "baseline_embed".to_string(),
        crate::jepa::EvalRunStage::TransitionFit => "transition_fit".to_string(),
        crate::jepa::EvalRunStage::Report => "report".to_string(),
    }
}

/// Human-readable string for a `JepaEvalBackendKind`.
fn backend_name_str(backend: crate::jepa::JepaEvalBackendKind) -> String {
    match backend {
        crate::jepa::JepaEvalBackendKind::Cpu => "fastembed cpu".to_string(),
        crate::jepa::JepaEvalBackendKind::MlxMetal => "mlx metal".to_string(),
        crate::jepa::JepaEvalBackendKind::Cuda => "cuda".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests_eval_progress {
    use super::*;
    use std::collections::HashMap;

    /// Build a minimal run record for testing.
    fn make_record() -> crate::jepa::JepaEvalRunRecord {
        crate::jepa::JepaEvalRunRecord {
            run_id: "jeval-test-001".to_string(),
            candidate_id: "candidate-test".to_string(),
            corpus_fingerprint: None,
            training_corpus_fingerprint: None,
            mode: crate::jepa::RuntimeEvalMode::Quick,
            backend: crate::jepa::JepaEvalBackendKind::Cpu,
            status: crate::jepa::EvalRunStatus::Running,
            current_stage: crate::jepa::EvalRunStage::BaselineEmbed,
            stage_timings: HashMap::new(),
            baseline_skipped: false,
            skipped_reason: None,
            pid: std::process::id(),
            host: "test-host".to_string(),
            started_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
            rows_total: 17105,
            rows_completed: 0,
            transitions_total: 17055,
            transitions_completed: 0,
            embeddings_total: 51165,
            embeddings_completed: 0,
            cache_hits: 0,
            cache_misses: 0,
            backend_parity_examples: 0,
            failure_reason: None,
            partial_gates: serde_json::Value::Null,
            result_paths: HashMap::new(),
        }
    }

    /// Verify that elapsed times are formatted as "mm:ss" not Debug Duration format.
    ///
    /// PRD §10 / F-MED-03: output must be "04:12", never "Duration { secs: 252, ... }".
    #[test]
    fn progress_format_is_mmss() {
        // format_mmss(252 secs) == 4 minutes 12 seconds == "04:12"
        let formatted = format_mmss(252);
        assert_eq!(
            formatted, "04:12",
            "expected mm:ss format '04:12', got '{formatted}'"
        );

        // Verify it does NOT look like a Debug Duration.
        assert!(
            !formatted.contains("Duration"),
            "must not contain 'Duration'"
        );
        assert!(
            !formatted.contains("secs"),
            "must not contain 'secs'"
        );

        // Also check the reporter emits mm:ss via print_progress (smoke test).
        let record = make_record();
        let mut reporter = ForegroundProgressReporter::new(record, 0, 1);
        // Single tick to exercise the code path; no panic == pass.
        let outcome = reporter.tick(100, 99, 300, 50, 50).unwrap();
        assert_eq!(outcome, TickOutcome::Continue);
    }

    /// When max_runtime_ms=1 and we sleep 2ms, tick() must return DeadlineExceeded.
    #[test]
    fn deadline_exceeded_returns_err() {
        let record = make_record();
        let mut reporter = ForegroundProgressReporter::new(record, 1, 1);

        // Sleep to ensure the deadline fires.
        std::thread::sleep(std::time::Duration::from_millis(2));

        let outcome = reporter.tick(1, 1, 3, 1, 0).unwrap();
        match outcome {
            TickOutcome::DeadlineExceeded { elapsed_ms } => {
                assert!(
                    elapsed_ms >= 1,
                    "elapsed_ms should be at least 1ms, got {elapsed_ms}"
                );
            }
            other => panic!("expected DeadlineExceeded, got {other:?}"),
        }
    }

    /// When max_runtime_ms=0 (unlimited), many ticks must never return DeadlineExceeded.
    #[test]
    fn unlimited_budget_never_times_out() {
        let record = make_record();
        let mut reporter = ForegroundProgressReporter::new(record, 0, 1);

        for i in 0..1000usize {
            let outcome = reporter.tick(i, i, i * 3, i / 2, i / 2).unwrap();
            assert_ne!(
                std::mem::discriminant(&outcome),
                std::mem::discriminant(&TickOutcome::DeadlineExceeded { elapsed_ms: 0 }),
                "unlimited budget must never return DeadlineExceeded at tick {i}"
            );
        }
    }
}
