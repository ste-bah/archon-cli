// TASK-JEVAL-011 — JepaEvalPlanner + TieredGateEvaluator
//
// Orchestrates the staged eval pipeline per PRD-006C §6.3:
//   - Tier 0: gates from candidate metadata only (no rows loaded)
//   - Tier 1: structural gates after rows are loaded (no embedding)
//   - Tier 2: full representation baseline (expensive)
//
// Modes:
//   - Quick:     skip Tier 2 if any Tier-0/Tier-1 gate failed (fast verdict)
//   - Full:      run Tier 2 even if prior tiers failed (operator override)
//   - Promotion: always run Tier 2; fail closed
//
// Types are flat under crate::jepa::* per DEC-JEVAL-11 (include!() module pattern).
//
// NOTE: archon-world-model does NOT depend on archon-core, so WorldModelJepaConfig
// cannot be imported here. JepaEvalGateConfig (below) mirrors the promotion/eval
// threshold fields. archon-core's eval_schema_version_or_default() helper is kept
// for use by T021/T025 consumers in archon-core-dependent crates only.

use sha2::{Digest, Sha256};

/// Eval schema version constant used in config_fingerprint (F-MED-01/F-MED-02 fix).
/// Stored standalone on the eval record for cheap pre-check (PRD §11).
/// TASK-JEVAL-025 will migrate this to a field on WorldModelJepaEvalConfig in archon-core.
pub const EVAL_SCHEMA_VERSION: u32 = 1;

/// Promotion/eval gate thresholds for the tiered eval pipeline.
///
/// These are distinct from `JepaTrainingConfig` (training hyperparameters).
/// This struct mirrors the relevant fields from `WorldModelJepaConfig` in archon-core
/// so that archon-world-model can use them without a dependency on archon-core.
///
/// Callers that hold a `WorldModelJepaConfig` should construct this from its fields.
#[derive(Debug, Clone, PartialEq)]
pub struct JepaEvalGateConfig {
    /// Minimum number of training examples required (Tier-0 training_corpus_sufficient gate).
    pub min_training_examples: usize,
    /// Minimum number of heldout (eval) examples required (Tier-1 heldout gate).
    pub min_heldout_examples: usize,
    /// Minimum CUDA hardware-validation example count (Tier-0 backend_execution gate).
    pub min_cuda_validation_examples: usize,
    /// Minimum Metal hardware-validation example count (Tier-0 backend_execution gate).
    pub min_metal_validation_examples: usize,
    /// Minimum cosine similarity between CUDA/Metal and CPU backends (config fingerprint key).
    pub backend_parity_cosine_floor: f32,
    /// Whether native accelerator ops are required (config fingerprint key).
    pub require_native_accelerator_ops: bool,
    /// Whether a CPU stage is allowed for accelerated candidates (config fingerprint key).
    pub allow_accelerated_candidate_cpu_stage: bool,
    /// Maximum allowed checkpoint file size in megabytes (Tier-0 checkpoint_size gate).
    pub max_checkpoint_mb: u64,
}

impl Default for JepaEvalGateConfig {
    fn default() -> Self {
        Self {
            min_training_examples: 2_000,
            min_heldout_examples: 200,
            min_cuda_validation_examples: 512,
            min_metal_validation_examples: 512,
            backend_parity_cosine_floor: 0.99,
            require_native_accelerator_ops: true,
            allow_accelerated_candidate_cpu_stage: false,
            max_checkpoint_mb: 64,
        }
    }
}

/// Result of evaluating a single tier of gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierGateResult {
    pub all_passed: bool,
    pub failures: Vec<String>,
}

impl TierGateResult {
    pub fn all_pass() -> Self {
        Self {
            all_passed: true,
            failures: vec![],
        }
    }

    pub fn with_failures(failures: Vec<String>) -> Self {
        Self {
            all_passed: failures.is_empty(),
            failures,
        }
    }
}

/// Eval planner. Decides which tiers to run based on mode + gate results.
pub struct JepaEvalPlanner {
    pub mode: RuntimeEvalMode,
}

impl JepaEvalPlanner {
    pub fn new(mode: RuntimeEvalMode) -> Self {
        Self { mode }
    }

    /// Tier 0: gates available from candidate metadata only — no row loading.
    /// PRD-006C §6.3.
    pub fn evaluate_tier0(
        candidate: &crate::registry::JepaCandidateRecord,
        config: &JepaEvalGateConfig,
    ) -> TierGateResult {
        let mut failures = Vec::new();

        // 1. Tensor finite/safety gate
        if candidate.model.validate_finite().is_err() {
            failures.push("tensor_safety".into());
        }

        // 2. Training corpus sufficient (training-time count — F-HIGH-06)
        //    Tier 1 has a separate eval_corpus_sufficient check.
        if (candidate.model.metadata.example_count as usize) < config.min_training_examples {
            failures.push("training_corpus_sufficient".into());
        }

        // 3. Collapse gate (computed at training time)
        if !candidate.outcome.collapse.passes {
            failures.push("representation_collapse".into());
        }

        // 4. Horizon consistency gate (computed at training time)
        if !candidate.outcome.horizon.passes {
            failures.push("multi_horizon_consistency".into());
        }

        // 5. Backend execution metadata gate
        let backend_failure = jepa_backend_promotion_gate_failure(
            &candidate.model.metadata,
            config.min_cuda_validation_examples,
            config.min_metal_validation_examples,
        );
        if backend_failure.is_some() {
            failures.push("backend_execution".into());
        }

        // 6. Checkpoint file size gate (candidate.checkpoint.path — JepaCheckpointRecord.path)
        if let Ok(meta) = std::fs::metadata(&candidate.checkpoint.path) {
            if meta.len() > config.max_checkpoint_mb * 1024 * 1024 {
                failures.push("checkpoint_size".into());
            }
        }

        TierGateResult::with_failures(failures)
    }

    /// Tier 1: gates requiring rows loaded but no embedding (structural only).
    /// Computes corpus_fingerprint as a side-effect.
    /// Returns (gate result, corpus_fingerprint).
    pub fn evaluate_tier1(
        rows: &[crate::WorldTraceRow],
        candidate: &crate::registry::JepaCandidateRecord,
        config: &JepaEvalGateConfig,
    ) -> (TierGateResult, String) {
        let mut failures = Vec::new();

        // Compute corpus_fingerprint over the loaded rows (canonical order)
        let corpus_fingerprint = Self::compute_corpus_fingerprint(rows);

        // 1. Transition / heldout-count gate via TraceWindowBuilder
        let builder = crate::TraceWindowBuilder::new(rows);
        let context_window = candidate.model.metadata.context_window_rows;
        let target_window = candidate.model.metadata.target_window_rows;
        let transitions = builder
            .adjacent_transitions(context_window, target_window, 1)
            .unwrap_or_default();

        // 80% train / 20% heldout split
        let total = transitions.len();
        let heldout_count = total / 5;
        if heldout_count < config.min_heldout_examples {
            failures.push(format!(
                "heldout_examples ({} < {})",
                heldout_count, config.min_heldout_examples
            ));
        }

        // 2. Eval corpus sufficient gate (current corpus, F-HIGH-06)
        if total < config.min_training_examples {
            failures.push(format!("eval_corpus_sufficient ({total} transitions)"));
        }

        (TierGateResult::with_failures(failures), corpus_fingerprint)
    }

    /// Compute corpus_fingerprint per PRD §6.2.
    /// Canonical sort: (session_id, created_at, row_id).
    /// Hashes row_id + session_id + redacted_excerpt content (F-HIGH-01:
    /// content-sensitive so mutations that preserve row_ids still change the
    /// fingerprint).
    pub fn compute_corpus_fingerprint(rows: &[crate::WorldTraceRow]) -> String {
        let mut sorted: Vec<&crate::WorldTraceRow> = rows.iter().collect();
        sorted.sort_by(|a, b| {
            a.session_id
                .cmp(&b.session_id)
                .then(a.created_at.cmp(&b.created_at))
                .then(a.row_id.cmp(&b.row_id))
        });
        let mut hasher = Sha256::new();
        for row in sorted {
            hasher.update(row.row_id.as_bytes());
            hasher.update(b"\0");
            hasher.update(row.session_id.as_bytes());
            hasher.update(b"\0");
            // Content-sensitive: WorldTraceRow text lives in redacted_excerpt
            // (NOT summary/action_text — those fields do not exist; T001 audit).
            if let Some(ref text) = row.redacted_excerpt {
                hasher.update(text.as_bytes());
            }
            hasher.update(b"\n");
        }
        format!("{:x}", hasher.finalize())
    }

    /// Compute config_fingerprint per PRD §11.
    /// Hashes the 5 enumerated promotion keys + eval_schema_version (hashed
    /// ONCE — F-MED-01 / F-MED-02 fix).
    pub fn compute_config_fingerprint(jepa_config: &JepaEvalGateConfig) -> String {
        let mut hasher = Sha256::new();
        hasher.update(
            jepa_config
                .require_native_accelerator_ops
                .to_string()
                .as_bytes(),
        );
        hasher.update(b"|");
        hasher.update(
            jepa_config
                .allow_accelerated_candidate_cpu_stage
                .to_string()
                .as_bytes(),
        );
        hasher.update(b"|");
        hasher.update(
            jepa_config
                .min_metal_validation_examples
                .to_string()
                .as_bytes(),
        );
        hasher.update(b"|");
        hasher.update(
            jepa_config
                .min_cuda_validation_examples
                .to_string()
                .as_bytes(),
        );
        hasher.update(b"|");
        hasher.update(
            jepa_config
                .backend_parity_cosine_floor
                .to_string()
                .as_bytes(),
        );
        hasher.update(b"|");
        // eval_schema_version hashed ONCE (also stored standalone on the eval
        // record for cheap pre-check — PRD §11).
        hasher.update(EVAL_SCHEMA_VERSION.to_string().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Decide whether to run Tier 2 based on mode + tier results.
    /// Quick: skip on any prior failure.
    /// Full / Promotion: always run.
    pub fn should_run_tier2(&self, tier0: &TierGateResult, tier1: &TierGateResult) -> bool {
        match self.mode {
            RuntimeEvalMode::Quick => tier0.all_passed && tier1.all_passed,
            RuntimeEvalMode::Full | RuntimeEvalMode::Promotion => true,
        }
    }
}

/// One-line migration notice for PRD §9 (eval-jepa default changed to quick).
pub const MIGRATION_NOTICE: &str =
    "Note: eval-jepa now defaults to --quick mode. Use --full to run the full \
     representation baseline (previous default behaviour).";

#[cfg(test)]
mod tests_eval_planner {
    use super::*;
    use crate::schema::WorldActionKind;

    fn make_row(session: &str, row_id: &str, text: Option<&str>) -> crate::WorldTraceRow {
        let mut row = crate::WorldTraceRow::new(session, WorldActionKind::Unknown).with_row_id(row_id);
        row.redacted_excerpt = text.map(|s| s.to_string());
        row
    }

    #[test]
    fn corpus_fingerprint_changes_with_content() {
        let row1 = make_row("s1", "r1", Some("original"));
        let row1_modified = make_row("s1", "r1", Some("different content"));
        let fp1 = JepaEvalPlanner::compute_corpus_fingerprint(&[row1]);
        let fp2 = JepaEvalPlanner::compute_corpus_fingerprint(&[row1_modified]);
        assert_ne!(fp1, fp2, "fingerprint must change when redacted_excerpt changes");
    }

    #[test]
    fn corpus_fingerprint_stable_under_reordering() {
        let row1 = make_row("s1", "r1", Some("content-a"));
        let row2 = make_row("s1", "r2", Some("content-b"));
        let fp1 = JepaEvalPlanner::compute_corpus_fingerprint(&[row1.clone(), row2.clone()]);
        let fp2 = JepaEvalPlanner::compute_corpus_fingerprint(&[row2, row1]);
        assert_eq!(fp1, fp2, "fingerprint must be order-independent (canonical sort)");
    }

    #[test]
    fn corpus_fingerprint_changes_when_row_added() {
        let row1 = make_row("s1", "r1", Some("a"));
        let row2 = make_row("s1", "r2", Some("b"));
        let fp1 = JepaEvalPlanner::compute_corpus_fingerprint(&[row1.clone()]);
        let fp2 = JepaEvalPlanner::compute_corpus_fingerprint(&[row1, row2]);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn config_fingerprint_is_stable() {
        let config = JepaEvalGateConfig::default();
        let fp1 = JepaEvalPlanner::compute_config_fingerprint(&config);
        let fp2 = JepaEvalPlanner::compute_config_fingerprint(&config);
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64, "sha256 hex is 64 chars");
    }

    #[test]
    fn config_fingerprint_changes_with_promotion_key() {
        let config_a = JepaEvalGateConfig::default();
        let mut config_b = config_a.clone();
        config_b.backend_parity_cosine_floor = 0.95; // different value
        let fp_a = JepaEvalPlanner::compute_config_fingerprint(&config_a);
        let fp_b = JepaEvalPlanner::compute_config_fingerprint(&config_b);
        assert_ne!(fp_a, fp_b);
    }

    #[test]
    fn quick_mode_skips_tier2_on_tier0_failure() {
        let planner = JepaEvalPlanner::new(RuntimeEvalMode::Quick);
        let tier0_fail = TierGateResult::with_failures(vec!["representation_collapse".into()]);
        let tier1_pass = TierGateResult::all_pass();
        assert!(!planner.should_run_tier2(&tier0_fail, &tier1_pass));
    }

    #[test]
    fn quick_mode_runs_tier2_on_all_pass() {
        let planner = JepaEvalPlanner::new(RuntimeEvalMode::Quick);
        assert!(planner.should_run_tier2(&TierGateResult::all_pass(), &TierGateResult::all_pass()));
    }

    #[test]
    fn full_mode_runs_tier2_even_after_failure() {
        let planner = JepaEvalPlanner::new(RuntimeEvalMode::Full);
        let tier0_fail = TierGateResult::with_failures(vec!["representation_collapse".into()]);
        assert!(planner.should_run_tier2(&tier0_fail, &TierGateResult::all_pass()));
    }

    #[test]
    fn promotion_mode_always_runs_tier2() {
        let planner = JepaEvalPlanner::new(RuntimeEvalMode::Promotion);
        let tier0_fail = TierGateResult::with_failures(vec!["x".into()]);
        let tier1_fail = TierGateResult::with_failures(vec!["y".into()]);
        assert!(planner.should_run_tier2(&tier0_fail, &tier1_fail));
    }
}
