use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::audit::store::{PipelineBundleStore, record_file_name};
use crate::audit::types::{BundleStatus, PipelineEvent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewindReport {
    pub session_id: String,
    pub from_completed_agent_count: usize,
    pub to_completed_agent_count: usize,
    pub quarantined_agent_records: usize,
    pub quarantine_dir: String,
}

pub fn rewind_bundle(
    store: &PipelineBundleStore,
    session_id: &str,
    keep_count: usize,
    reason: &str,
) -> Result<RewindReport> {
    let mut state = store.load_state(session_id)?;
    if state.status == BundleStatus::Running {
        anyhow::bail!("cannot rewind a running pipeline; abort it or wait for it to stop first");
    }
    let records = store.list_agent_records(session_id)?;
    if state.completed_agent_count != records.len() {
        anyhow::bail!(
            "cannot rewind inconsistent bundle: state completed_agent_count={} but {} active agent record(s) exist",
            state.completed_agent_count,
            records.len()
        );
    }
    ensure_contiguous_records(&records)?;
    if keep_count > records.len() {
        anyhow::bail!(
            "cannot rewind to keep {keep_count} agent(s); bundle has only {} agent record(s)",
            records.len()
        );
    }

    let stale = records
        .iter()
        .filter(|record| record.ordinal >= keep_count)
        .collect::<Vec<_>>();
    let bundle_dir = store.bundle_dir(session_id);
    let now = Utc::now();
    let quarantine = bundle_dir.join("rewound").join(format!(
        "{}-{}-keep-{keep_count}",
        now.format("%Y%m%dT%H%M%SZ"),
        now.timestamp_subsec_nanos()
    ));
    fs::create_dir_all(&quarantine)?;

    for record in &stale {
        for path in stale_record_paths(record) {
            move_relative_path(&bundle_dir, &quarantine, &path)?;
        }
    }
    for path in stale_final_export_paths() {
        move_relative_path(&bundle_dir, &quarantine, &path)?;
    }

    let kept = records
        .iter()
        .filter(|record| record.ordinal < keep_count)
        .collect::<Vec<_>>();
    state.completed_agent_count = kept.len();
    state.total_tokens_in = kept.iter().map(|record| record.tokens_in).sum();
    state.total_tokens_out = kept.iter().map(|record| record.tokens_out).sum();
    state.total_cost_usd = kept.iter().map(|record| record.cost_usd).sum();
    state.status = BundleStatus::Failed;
    state.current_agent_key = None;
    state.completed_at = None;
    state.final_output_hash = None;
    state.completion_integrity_summary = None;
    state.completion_report_id = None;
    state.updated_at = Utc::now();
    state.last_error = Some(format!(
        "rewound to {keep_count} completed agent(s): {reason}"
    ));
    store.save_state(&state)?;
    store.append_event(
        session_id,
        PipelineEvent::RunRewound {
            from_completed_agent_count: records.len(),
            to_completed_agent_count: kept.len(),
            reason: reason.to_string(),
        },
    )?;

    Ok(RewindReport {
        session_id: session_id.to_string(),
        from_completed_agent_count: records.len(),
        to_completed_agent_count: kept.len(),
        quarantined_agent_records: stale.len(),
        quarantine_dir: quarantine.display().to_string(),
    })
}

fn ensure_contiguous_records(records: &[crate::audit::types::AgentAuditRecord]) -> Result<()> {
    for (expected, record) in records.iter().enumerate() {
        if record.ordinal != expected {
            anyhow::bail!(
                "cannot rewind non-contiguous agent records: expected ordinal {expected}, found {} for {}",
                record.ordinal,
                record.agent_key
            );
        }
    }
    Ok(())
}

fn stale_record_paths(record: &crate::audit::types::AgentAuditRecord) -> Vec<PathBuf> {
    let mut paths = HashSet::new();
    paths.insert(PathBuf::from("agents").join(record_file_name(
        record.ordinal,
        &record.agent_key,
        "json",
    )));
    insert_relative(&mut paths, &record.prompt_record_path);
    insert_relative(&mut paths, &record.output_path);
    for attempt in &record.attempts {
        if let Some(path) = &attempt.output_path {
            insert_relative(&mut paths, path);
        }
    }

    let stem = record_stem(record.ordinal, &record.agent_key);
    paths.insert(
        PathBuf::from("outputs")
            .join("markdown")
            .join(format!("{stem}.md")),
    );
    paths.insert(PathBuf::from("outputs").join("artifacts").join(&stem));
    if let Some(agent) = crate::research::agents::get_agent_by_key(&record.agent_key) {
        for namespace in crate::research::rlm::research_output_namespaces(agent) {
            paths.insert(
                PathBuf::from("outputs")
                    .join("rlm")
                    .join(safe_artifact_path(&namespace))
                    .with_extension("md"),
            );
        }
    }

    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    paths
}

fn stale_final_export_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("exports").join("final-paper.md"),
        PathBuf::from("exports").join("final-paper.pdf"),
    ]
}

fn insert_relative(paths: &mut HashSet<PathBuf>, path: &str) {
    if !path.trim().is_empty() {
        paths.insert(PathBuf::from(path));
    }
}

fn record_stem(ordinal: usize, agent_key: &str) -> String {
    record_file_name(ordinal, agent_key, "json")
        .strip_suffix(".json")
        .unwrap_or("agent")
        .to_string()
}

fn move_relative_path(bundle_dir: &Path, quarantine: &Path, relative: &Path) -> Result<()> {
    ensure_safe_relative(relative)?;
    let source = bundle_dir.join(relative);
    if !source.exists() {
        return Ok(());
    }
    let target = quarantine.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&source, &target).with_context(|| {
        format!(
            "quarantine stale pipeline artifact {} -> {}",
            source.display(),
            target.display()
        )
    })
}

fn ensure_safe_relative(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        anyhow::bail!(
            "refusing to quarantine unsafe relative path {}",
            path.display()
        );
    }
    Ok(())
}

fn safe_artifact_path(path: &str) -> PathBuf {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(safe_segment)
        .collect()
}

fn safe_segment(segment: &str) -> String {
    let mut out = String::new();
    for c in segment.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "artifact".to_string()
    } else {
        trimmed.to_string()
    }
}
