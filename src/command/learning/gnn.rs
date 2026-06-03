//! GNN auto-trainer status surface for CLI and slash commands.

use anyhow::Result;

use archon_memory::{MemoryTrait, MemoryType, SearchFilter};
use archon_pipeline::learning::gnn::auto_trainer::AutoTrainerStatus;

#[derive(Debug, Clone, Copy)]
pub(crate) struct DurableMemoryStats {
    total_memories: u64,
    total_corrections: u64,
}

pub(crate) async fn print_gnn_status(config: &archon_core::config::ArchonConfig) -> Result<()> {
    let durable = open_durable_memory_stats(config).await;
    println!("{}", render_gnn_status_with_durable(config, None, durable));
    Ok(())
}

pub(crate) fn render_gnn_status_with_durable(
    config: &archon_core::config::ArchonConfig,
    live: Option<&AutoTrainerStatus>,
    durable: Option<DurableMemoryStats>,
) -> String {
    let at = &config.learning.gnn.auto_trainer;
    let enabled = config.learning.gnn.enabled && live.map(|s| s.enabled).unwrap_or(at.enabled);
    let daemon = daemon_gnn_status(config);
    let total_memories = live
        .map(|s| s.total_memories)
        .or_else(|| durable.map(|s| s.total_memories))
        .unwrap_or(0);
    let total_corrections = live
        .map(|s| s.total_corrections)
        .or_else(|| durable.map(|s| s.total_corrections))
        .unwrap_or(0);
    let training_count = live
        .map(|s| s.training_count)
        .unwrap_or(daemon.training_count);
    let no_data_count = live
        .map(|s| s.no_data_count)
        .unwrap_or(daemon.no_data_count);
    let memories_since = live
        .map(|s| s.memories_since_last_train)
        .or_else(|| daemon.memories_since_last_training(total_memories))
        .or_else(|| durable.map(|s| s.total_memories))
        .unwrap_or(0);
    let corrections_since = live
        .map(|s| s.corrections_since_last_train)
        .or_else(|| daemon.corrections_since_last_training(total_corrections))
        .or_else(|| durable.map(|s| s.total_corrections))
        .unwrap_or(0);
    let seconds_since_last = live
        .and_then(|s| s.seconds_since_last_train)
        .or(daemon.seconds_since_last_training);
    let seconds_since_last_attempt = live
        .and_then(|s| s.seconds_since_last_attempt)
        .or(daemon.seconds_since_last_attempt);
    let in_progress = live.map(|s| s.training_in_progress).unwrap_or(false);
    let last_sources = live.and_then(|s| s.last_outcome.as_ref().map(|o| o.data_sources.clone()));
    let last_no_data_reason = live.and_then(|s| s.last_no_data_reason.as_deref());

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
    let elapsed_gate = match seconds_since_last_attempt {
        Some(seconds) => gate(seconds * 1000, trigger_elapsed_ms),
        None => "n/a (no last-run timestamp)".into(),
    };
    let throttle_gate = match seconds_since_last_attempt {
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
    let last_attempt = match seconds_since_last_attempt {
        Some(seconds) => format!("{seconds}s ago"),
        None => "never".into(),
    };
    let last_data_sources = match last_sources {
        Some(sources) => format!(
            "SONA trajectories={}, SONA triplets={}, meaning triplets={}",
            sources.sona_trajectories, sources.sona_triplets, sources.meaning_triplets,
        ),
        None => "n/a".into(),
    };
    let last_no_data = last_no_data_reason.unwrap_or("none");
    let status_source = if live.is_some() {
        "live auto-trainer"
    } else if durable.is_some() && daemon.attempt_count > 0 {
        "durable memory graph + daemon ledger"
    } else if durable.is_some() {
        "durable memory graph"
    } else {
        "config fallback"
    };
    let daemon_status = daemon.status_line;

    format!(
        "GNN Auto-Trainer Status\n\
         =======================\n\
         Enabled:           {enabled}\n\
         Status source:     {status_source}\n\
         Total memories:    {total_memories}\n\
         Total corrections: {total_corrections}\n\
         Training runs:     {training_count}\n\
         No-data ticks:     {no_data_count}\n\
         Last data:         {last_data_sources}\n\
         Last no-data:      {last_no_data}\n\
         Last attempt:      {last_attempt}\n\
         Last training:     {last_training}\n\
         First-run gate:    {first_run_gate}\n\
         New-memory gate:   {new_memory_gate}\n\
         Correction gate:   {correction_gate}\n\
         Elapsed gate:      {elapsed_gate}\n\
         Throttle gate:     {throttle_gate}\n\
         Daemon trainer:    {daemon_status}"
    )
}

fn daemon_status_line(config: &archon_core::config::ArchonConfig) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let root = expand_path(&cwd, &config.learning.cognitive.ledger_dir);
    crate::command::cognitive_daemon_learning_ledger::latest_for_job(&root, "gnn_auto_trainer")
        .map(|event| {
            format!(
                "{} {} — {}",
                event.created_at.to_rfc3339(),
                event.status,
                event.summary
            )
        })
        .unwrap_or_else(|| "no daemon trainer tick recorded".into())
}

#[derive(Debug, Clone)]
struct DaemonGnnStatus {
    training_count: u64,
    no_data_count: u64,
    attempt_count: u64,
    seconds_since_last_training: Option<u64>,
    seconds_since_last_attempt: Option<u64>,
    memories_at_last_training: Option<u64>,
    corrections_at_last_training: Option<u64>,
    status_line: String,
}

impl DaemonGnnStatus {
    fn memories_since_last_training(&self, total_memories: u64) -> Option<u64> {
        self.memories_at_last_training
            .map(|last| total_memories.saturating_sub(last))
    }

    fn corrections_since_last_training(&self, total_corrections: u64) -> Option<u64> {
        self.corrections_at_last_training
            .map(|last| total_corrections.saturating_sub(last))
    }
}

fn daemon_gnn_status(config: &archon_core::config::ArchonConfig) -> DaemonGnnStatus {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let root = expand_path(&cwd, &config.learning.cognitive.ledger_dir);
    let summary = crate::command::cognitive_daemon_learning_ledger::training_summary(
        &root,
        "gnn_auto_trainer",
    );
    let latest_training = summary.latest_training.as_ref();
    let latest_attempt = summary.latest_attempt.as_ref();
    DaemonGnnStatus {
        training_count: summary.training_count,
        no_data_count: crate::command::cognitive_daemon_learning_ledger::latest(&root, 512)
            .into_iter()
            .filter(|event| event.job == "gnn_auto_trainer" && event.status == "no_data")
            .count() as u64,
        attempt_count: summary.attempt_count,
        seconds_since_last_training: latest_training.map(|event| seconds_since(event.created_at)),
        seconds_since_last_attempt: latest_attempt.map(|event| seconds_since(event.created_at)),
        memories_at_last_training: latest_training.and_then(|event| event.total_memories),
        corrections_at_last_training: latest_training.and_then(|event| event.total_corrections),
        status_line: daemon_status_line(config),
    }
}

fn seconds_since(then: chrono::DateTime<chrono::Utc>) -> u64 {
    chrono::Utc::now()
        .signed_duration_since(then)
        .num_seconds()
        .max(0) as u64
}

fn expand_path(cwd: &std::path::Path, raw: &str) -> std::path::PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home).join(stripped);
    }
    let path = std::path::PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn gate(value: u64, threshold: u64) -> String {
    if value >= threshold {
        format!("{value}/{threshold} (open)")
    } else {
        format!("{value}/{threshold} (closed)")
    }
}

pub(crate) fn durable_memory_stats(memory: &dyn MemoryTrait) -> Option<DurableMemoryStats> {
    let total_memories = match memory.memory_count() {
        Ok(count) => count as u64,
        Err(e) => {
            tracing::warn!(error = %e, "GNN status memory count failed");
            return None;
        }
    };
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Correction),
        ..Default::default()
    };
    let total_corrections = match memory.search_memories(&filter) {
        Ok(corrections) => corrections.len() as u64,
        Err(e) => {
            tracing::warn!(error = %e, "GNN status correction count failed");
            0
        }
    };
    Some(DurableMemoryStats {
        total_memories,
        total_corrections,
    })
}

async fn open_durable_memory_stats(
    config: &archon_core::config::ArchonConfig,
) -> Option<DurableMemoryStats> {
    let (memory_data_dir, memory_db_path) =
        archon_memory::resolve_memory_paths(config.memory.db_path.as_deref());
    match archon_memory::open_memory_with_db_path(&memory_data_dir, &memory_db_path).await {
        Ok(memory) => durable_memory_stats(&memory),
        Err(e) => {
            tracing::warn!(error = %e, "GNN status durable memory open failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_default_status_shows_tuned_thresholds() {
        let rendered = render_gnn_status_with_durable(
            &archon_core::config::ArchonConfig::default(),
            None,
            None,
        );
        assert!(rendered.contains("Enabled:           true"));
        assert!(rendered.contains("First-run gate:    0/30 (closed)"));
        assert!(rendered.contains("New-memory gate:   0/20 (closed)"));
        assert!(rendered.contains("Correction gate:   0/3 (closed)"));
    }

    #[test]
    fn render_durable_fallback_uses_memory_graph_counts() {
        let rendered = render_gnn_status_with_durable(
            &archon_core::config::ArchonConfig::default(),
            None,
            Some(DurableMemoryStats {
                total_memories: 933,
                total_corrections: 30,
            }),
        );
        assert!(rendered.contains("Status source:     durable memory graph"));
        assert!(rendered.contains("Total memories:    933"));
        assert!(rendered.contains("First-run gate:    933/30 (open)"));
        assert!(rendered.contains("Correction gate:   30/3 (open)"));
    }
}
