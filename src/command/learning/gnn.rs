//! GNN auto-trainer status surface for CLI and slash commands.

use anyhow::Result;

use archon_pipeline::learning::gnn::auto_trainer::AutoTrainerStatus;

pub(crate) fn print_gnn_status(config: &archon_core::config::ArchonConfig) -> Result<()> {
    println!("{}", render_gnn_status(config, None));
    Ok(())
}

pub(crate) fn render_gnn_status(
    config: &archon_core::config::ArchonConfig,
    live: Option<&AutoTrainerStatus>,
) -> String {
    let at = &config.learning.gnn.auto_trainer;
    let enabled = config.learning.gnn.enabled && live.map(|s| s.enabled).unwrap_or(at.enabled);
    let total_memories = live.map(|s| s.total_memories).unwrap_or(0);
    let total_corrections = live.map(|s| s.total_corrections).unwrap_or(0);
    let training_count = live.map(|s| s.training_count).unwrap_or(0);
    let memories_since = live.map(|s| s.memories_since_last_train).unwrap_or(0);
    let corrections_since = live.map(|s| s.corrections_since_last_train).unwrap_or(0);
    let seconds_since_last = live.and_then(|s| s.seconds_since_last_train);
    let in_progress = live.map(|s| s.training_in_progress).unwrap_or(false);

    let first_run_threshold = live
        .map(|s| s.first_run_threshold)
        .unwrap_or(at.first_run_threshold);
    let trigger_new_memories = live
        .map(|s| s.trigger_new_memories)
        .unwrap_or(at.trigger_new_memories);
    let trigger_corrections = live
        .map(|s| s.trigger_corrections)
        .unwrap_or(at.trigger_corrections);
    let trigger_elapsed_ms = live
        .map(|s| s.trigger_elapsed_ms)
        .unwrap_or(at.trigger_elapsed_ms);
    let min_throttle_ms = live
        .map(|s| s.min_throttle_ms)
        .unwrap_or(at.min_throttle_ms);

    let first_run_gate = if training_count == 0 {
        gate(total_memories, first_run_threshold)
    } else {
        format!("complete ({training_count} run(s))")
    };
    let new_memory_gate = gate(memories_since, trigger_new_memories);
    let correction_gate = gate(corrections_since, trigger_corrections);
    let elapsed_gate = match seconds_since_last {
        Some(seconds) => gate(seconds * 1000, trigger_elapsed_ms),
        None => "n/a (no last-run timestamp)".into(),
    };
    let throttle_gate = match seconds_since_last {
        Some(seconds) => {
            let elapsed_ms = seconds * 1000;
            if elapsed_ms >= min_throttle_ms {
                format!("{elapsed_ms}/{min_throttle_ms} ms (open)")
            } else {
                format!("{elapsed_ms}/{min_throttle_ms} ms (closed)")
            }
        }
        None => "n/a".into(),
    };
    let last_training = match (in_progress, seconds_since_last) {
        (true, _) => "in progress".to_string(),
        (false, Some(seconds)) => format!("{seconds}s ago"),
        (false, None) => "never".into(),
    };

    format!(
        "GNN Auto-Trainer Status\n\
         =======================\n\
         Enabled:           {enabled}\n\
         Total memories:    {total_memories}\n\
         Total corrections: {total_corrections}\n\
         Last training:     {last_training}\n\
         First-run gate:    {first_run_gate}\n\
         New-memory gate:   {new_memory_gate}\n\
         Correction gate:   {correction_gate}\n\
         Elapsed gate:      {elapsed_gate}\n\
         Throttle gate:     {throttle_gate}"
    )
}

fn gate(value: u64, threshold: u64) -> String {
    if value >= threshold {
        format!("{value}/{threshold} (open)")
    } else {
        format!("{value}/{threshold} (closed)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_default_status_shows_tuned_thresholds() {
        let rendered = render_gnn_status(&archon_core::config::ArchonConfig::default(), None);
        assert!(rendered.contains("Enabled:           true"));
        assert!(rendered.contains("First-run gate:    0/30 (closed)"));
        assert!(rendered.contains("New-memory gate:   0/20 (closed)"));
        assert!(rendered.contains("Correction gate:   0/3 (closed)"));
    }
}
