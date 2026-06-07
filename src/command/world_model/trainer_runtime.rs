use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DaemonTrainerEvent {
    pub created_at: DateTime<Utc>,
    pub status: String,
    pub summary: String,
}

pub(crate) fn schedule_dynamic_trainer_tick(config: archon_core::config::ArchonConfig) {
    if !config.learning.world_model.enabled || !config.learning.world_model.auto_trainer.enabled {
        return;
    }
    archon_observability::spawn_named("world-model-dynamic-trainer", async move {
        let _ = run_daemon_trainer_tick(&config);
    });
}

pub(crate) fn run_daemon_trainer_tick(
    config: &archon_core::config::ArchonConfig,
) -> Result<String> {
    run_daemon_trainer_tick_controlled(config, &|| false)
}

pub(crate) fn run_daemon_trainer_tick_controlled(
    config: &archon_core::config::ArchonConfig,
    stop_requested: &dyn Fn() -> bool,
) -> Result<String> {
    let root = super::world_model_root()?;
    let auto = &config.learning.world_model.auto_trainer;
    let max_runtime_ms = trainer_runtime_limit_ms(config);
    let started = Instant::now();
    let should_stop = || {
        stop_requested()
            || (max_runtime_ms > 0 && started.elapsed().as_millis() >= u128::from(max_runtime_ms))
    };
    let progress = |stage: &str, detail: &str| {
        let summary = format!("{stage}: {detail}");
        if let Err(error) = append_event(&root, "progress", &summary) {
            tracing::warn!(error = %error, stage, "daemon trainer progress append failed");
        }
        tracing::info!(stage, detail, "world-model daemon trainer progress");
    };
    let rendered = super::candidate::render_trainer_tick_observed(
        config,
        &root,
        Some(auto.idle_required_ms),
        None,
        None,
        false,
        Some(&should_stop),
        Some(&progress),
    );
    match rendered {
        Ok(output) => {
            let summary = compact_summary(&output);
            append_event(&root, "ok", &summary)?;
            Ok(summary)
        }
        Err(error) => {
            let summary = format!("failed: {error}");
            append_event(&root, "failed", &summary)?;
            Err(error)
        }
    }
}

fn trainer_runtime_limit_ms(config: &archon_core::config::ArchonConfig) -> u64 {
    if config.learning.world_model.jepa.enabled
        || config.learning.world_model.model_kind == archon_world_model::jepa::JEPA_MODEL_KIND
    {
        config.learning.world_model.jepa.max_runtime_ms
    } else {
        config.learning.world_model.auto_trainer.max_runtime_ms
    }
}

pub(crate) fn latest_daemon_trainer_event() -> Option<DaemonTrainerEvent> {
    let root = super::world_model_root().ok()?;
    latest_event(&root)
}

fn compact_summary(output: &str) -> String {
    let mut decision = "Decision: unknown";
    let mut candidate = "Candidate: none";
    let mut promotion = "Auto promotion: none";
    for line in output.lines().map(str::trim) {
        if line.starts_with("Decision:") {
            decision = line;
        } else if line.starts_with("Candidate:") {
            candidate = line;
        } else if line.starts_with("Auto promotion:") {
            promotion = line;
        }
    }
    format!("{decision}; {candidate}; {promotion}")
}

fn append_event(root: &Path, status: &str, summary: &str) -> Result<()> {
    let dir = root.join("ledgers");
    std::fs::create_dir_all(&dir)?;
    let event = DaemonTrainerEvent {
        created_at: Utc::now(),
        status: status.into(),
        summary: summary.into(),
    };
    let mut line = serde_json::to_vec(&event)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path(root))?
        .write_all(&line)?;
    Ok(())
}

fn latest_event(root: &Path) -> Option<DaemonTrainerEvent> {
    let raw = std::fs::read_to_string(ledger_path(root)).ok()?;
    raw.lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
}

fn ledger_path(root: &Path) -> PathBuf {
    root.join("ledgers").join("daemon-trainer-events.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_summary_extracts_decision_candidate_and_promotion() {
        let summary = compact_summary(
            "World Model Trainer Tick\nDecision: NoTrigger\nCandidate: none\nAuto promotion: none",
        );

        assert_eq!(
            summary,
            "Decision: NoTrigger; Candidate: none; Auto promotion: none"
        );
    }
}
