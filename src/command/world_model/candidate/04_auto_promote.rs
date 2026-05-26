fn render_auto_promote_transition(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
) -> String {
    match ensure_auto_promotion_allowed(config) {
        Ok(()) => {}
        Err(reason) => {
            return record_auto_promotion_attempt(root, "latent_transition", candidate_id, &reason);
        }
    }
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
    match ensure_auto_promotion_allowed(config) {
        Ok(()) => {}
        Err(reason) => {
            return record_auto_promotion_attempt(root, "jepa_transition", candidate_id, &reason);
        }
    }
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

    #[test]
    fn auto_promote_reports_disabled_flag_before_policy_lookup() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.auto_promote_advisory = false;
        let temp = tempfile::tempdir().expect("tempdir");

        let rendered = render_auto_promote_transition(&config, temp.path(), "candidate-1");

        assert!(rendered.contains("auto_promote_advisory=false"));
    }
}
