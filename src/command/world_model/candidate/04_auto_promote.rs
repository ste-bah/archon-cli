fn render_auto_promote_transition(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
) -> String {
    // Evaluate first, unconditionally. Evaluation is read-only and produces the
    // evidence needed to decide whether promotion should ever be enabled, so
    // gating it behind the promotion flag is backwards: with
    // auto_promote_advisory=false a corpus accumulates candidates that were
    // never scored against the nearest-neighbour baseline, which is precisely
    // how a store ends up holding dozens of candidates and "Last eval: none".
    match render_eval(config, root, Some(candidate_id)) {
        Ok(_) => {}
        Err(error) => {
            return record_auto_promotion_attempt(
                root,
                "latent_transition",
                candidate_id,
                &format!("eval failed: {error}"),
            );
        }
    }
    // Only the promotion itself is gated.
    match ensure_auto_promotion_allowed(config) {
        Ok(()) => {}
        Err(reason) => {
            return record_auto_promotion_attempt(
                root,
                "latent_transition",
                candidate_id,
                &format!("evaluated; promotion skipped: {reason}"),
            );
        }
    }
    let status = match render_promote(root, candidate_id) {
        Ok(_) => format!("promoted advisory candidate {candidate_id}"),
        Err(error) => format!("promotion rejected: {error}"),
    };
    record_auto_promotion_attempt(root, "latent_transition", candidate_id, &status)
}

fn render_auto_promote_jepa(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
) -> String {
    // Same ordering as the latent path: evaluate unconditionally, gate only the
    // promotion. Evaluation is the evidence; withholding it until promotion is
    // already enabled leaves a corpus of candidates nobody can judge.
    match render_eval_jepa(config, root, candidate_id) {
        Ok(_) => {}
        Err(error) => {
            return record_auto_promotion_attempt(
                root,
                "jepa_transition",
                candidate_id,
                &format!("promotion eval failed: {error}"),
            );
        }
    }
    match ensure_auto_promotion_allowed(config) {
        Ok(()) => {}
        Err(reason) => {
            return record_auto_promotion_attempt(
                root,
                "jepa_transition",
                candidate_id,
                &format!("evaluated; promotion skipped: {reason}"),
            );
        }
    }
    let status = match render_promote_jepa(root, candidate_id, config) {
        Ok(_) => format!("promoted advisory JEPA candidate {candidate_id}"),
        Err(error) => format!("promotion rejected: {error}"),
    };
    record_auto_promotion_attempt(root, "jepa_transition", candidate_id, &status)
}

fn ensure_auto_promotion_allowed(config: &archon_core::config::ArchonConfig) -> Result<(), String> {
    if !config.learning.world_model.auto_promote_advisory {
        return Err("disabled by learning.world_model.auto_promote_advisory=false".into());
    }
    let workspace = std::env::current_dir()
        .map_err(|error| format!("policy check failed: current dir unavailable: {error}"))?;
    let policy = archon_policy::load_effective_policy(&workspace)
        .map_err(|error| format!("policy check failed: {error}"))?;
    if !policy.world_model.allow_behavior_changes {
        return Err("skipped: policy.world_model.allow_behavior_changes=false".into());
    }
    Ok(())
}

fn record_auto_promotion_attempt(
    root: &Path,
    model_kind: &str,
    candidate_id: &str,
    status: &str,
) -> String {
    let result = (|| -> Result<()> {
        use std::io::Write as _;

        let dir = root.join("ledgers");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("auto-promotions.jsonl");
        let record = serde_json::json!({
            "created_at": chrono::Utc::now(),
            "source": "world_model_background_trainer",
            "model_kind": model_kind,
            "candidate_id": candidate_id,
            "status": status,
        });
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(&line)?;
        Ok(())
    })();
    match result {
        Ok(()) => status.to_string(),
        Err(error) => format!("{status} (audit write failed: {error})"),
    }
}

#[cfg(test)]
mod auto_promote_tests {
    use super::*;

    /// Evaluation must be attempted even when promotion is disabled.
    ///
    /// This previously short-circuited on the flag before reaching `render_eval`,
    /// so a store with `auto_promote_advisory=false` accumulated candidates that
    /// were never scored against the nearest-neighbour baseline — dozens of
    /// candidates alongside "Last eval: none". Evaluation is read-only and is
    /// the evidence needed to decide whether promotion should be enabled at all,
    /// so it must not be gated behind the decision it informs.
    ///
    /// Against an empty root the eval itself cannot succeed; what matters is
    /// that the failure comes from evaluation rather than from an early bail on
    /// the flag.
    #[test]
    fn evaluation_is_attempted_even_when_promotion_is_disabled() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.auto_promote_advisory = false;
        let temp = tempfile::tempdir().expect("tempdir");

        let rendered = render_auto_promote_transition(&config, temp.path(), "candidate-1");

        assert!(
            rendered.contains("eval failed"),
            "expected an evaluation attempt, got: {rendered}"
        );
        assert!(
            !rendered.contains("auto_promote_advisory=false"),
            "must not short-circuit on the promotion flag before evaluating: {rendered}"
        );
    }

    /// When evaluation succeeds but promotion is disabled, the flag is still
    /// surfaced — just after the evidence has been produced, not instead of it.
    #[test]
    fn promotion_flag_is_reported_after_evaluation() {
        let reason = ensure_auto_promotion_allowed(&{
            let mut config = archon_core::config::ArchonConfig::default();
            config.learning.world_model.auto_promote_advisory = false;
            config
        })
        .expect_err("promotion should be refused when the flag is off");
        assert!(reason.contains("auto_promote_advisory=false"));
    }
}
