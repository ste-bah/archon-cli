use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use archon_world_model::ingest::IngestSummary;
use archon_world_model::storage::{PersistSummary, RetentionPolicy, WorldModelStore};

use super::labeling_runtime::WorldModelLabelingRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IngestReport {
    pub files_read: usize,
    pub rows_normalized: usize,
    pub rows_skipped: usize,
    pub rows_persisted: usize,
    pub cozo_rows: usize,
    pub warnings: usize,
    pub ledger_path: PathBuf,
    pub db_path: PathBuf,
    sources: BTreeMap<String, usize>,
}

impl IngestReport {
    pub fn sources_summary(&self) -> String {
        if self.sources.is_empty() {
            return "none".into();
        }
        self.sources
            .iter()
            .map(|(source, rows)| format!("{source}:{rows}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    ActivityJsonl,
    TraceExportJsonl,
    ProviderRuntimeJsonl,
    PlanJson,
    TranscriptJson,
    TranscriptJsonl,
    AgentOutputJson,
    MemoryJson,
    RetrospectiveJson,
    AgentEvolutionJson,
    GenericArtifactJson,
    GenericArtifactJsonl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFile {
    session_id: String,
    path: PathBuf,
    kind: SourceKind,
}

pub(super) async fn ingest_session(
    session_id: &str,
    sessions_root: &Path,
    world_root: &Path,
    cwd: &Path,
    retention: RetentionPolicy,
    labeler: &WorldModelLabelingRuntime,
) -> Result<IngestReport> {
    let files = session_source_files(session_id, sessions_root, cwd)?;
    persist_sources(world_root, retention, &files, labeler, false).await
}

pub(super) async fn ingest_backfill(
    sessions_root: &Path,
    world_root: &Path,
    cwd: &Path,
    retention: RetentionPolicy,
    labeler: &WorldModelLabelingRuntime,
) -> Result<IngestReport> {
    let mut files = Vec::new();
    if sessions_root.exists() {
        for entry in fs::read_dir(sessions_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let session_id = entry.file_name().to_string_lossy().to_string();
            files.extend(session_source_files(&session_id, sessions_root, cwd)?);
        }
    }
    persist_sources(world_root, retention, &files, labeler, true).await
}

async fn persist_sources(
    world_root: &Path,
    retention: RetentionPolicy,
    files: &[SourceFile],
    labeler: &WorldModelLabelingRuntime,
    skip_existing: bool,
) -> Result<IngestReport> {
    let store = WorldModelStore::open(world_root)?;
    let mut existing_row_ids = if skip_existing {
        Some(store.row_ids()?)
    } else {
        None
    };
    let mut report = IngestReport {
        files_read: 0,
        rows_normalized: 0,
        rows_skipped: 0,
        rows_persisted: 0,
        cozo_rows: 0,
        warnings: 0,
        ledger_path: world_root.join("ledgers").join("world-trace-rows.jsonl"),
        db_path: world_root.join("world-model.db"),
        sources: BTreeMap::new(),
    };

    for file in files {
        let content = fs::read_to_string(&file.path)?;
        let mut summary = normalize_file(file, &content);
        let rows_normalized = summary.row_count();
        let rows_skipped = existing_row_ids
            .as_ref()
            .map(|row_ids| retain_new_rows(&mut summary, row_ids))
            .unwrap_or(0);
        if summary.rows.is_empty() {
            record_summary(&mut report, &summary, rows_normalized, rows_skipped, None);
            continue;
        }
        labeler.apply(&mut summary).await?;
        let persisted = store.persist_rows_with_retention(&summary.rows, retention)?;
        if let Some(row_ids) = existing_row_ids.as_mut() {
            row_ids.extend(summary.rows.iter().map(|row| row.row_id.clone()));
        }
        record_summary(
            &mut report,
            &summary,
            rows_normalized,
            rows_skipped,
            Some(&persisted),
        );
    }

    Ok(report)
}

fn retain_new_rows(summary: &mut IngestSummary, existing_row_ids: &HashSet<String>) -> usize {
    let before = summary.rows.len();
    summary
        .rows
        .retain(|row| !existing_row_ids.contains(&row.row_id));
    before - summary.rows.len()
}

fn record_summary(
    report: &mut IngestReport,
    summary: &IngestSummary,
    rows_normalized: usize,
    rows_skipped: usize,
    persisted: Option<&PersistSummary>,
) {
    report.files_read += 1;
    report.rows_normalized += rows_normalized;
    report.rows_skipped += rows_skipped;
    report.warnings += summary.warning_count();
    if let Some(persisted) = persisted {
        report.rows_persisted += persisted.jsonl_rows;
        report.cozo_rows += persisted.cozo_rows;
        report.ledger_path = persisted.ledger_path.clone();
        report.db_path = persisted.db_path.clone();
    }
    *report.sources.entry(summary.source.clone()).or_default() += rows_normalized;
}

fn normalize_file(file: &SourceFile, content: &str) -> IngestSummary {
    match file.kind {
        SourceKind::ActivityJsonl => archon_world_model::ingest::normalize_activity_jsonl(content),
        SourceKind::TraceExportJsonl => {
            archon_world_model::ingest::normalize_trace_export_jsonl(content)
        }
        SourceKind::ProviderRuntimeJsonl => {
            archon_world_model::ingest::normalize_provider_runtime_jsonl(content)
        }
        SourceKind::PlanJson => {
            archon_world_model::ingest::normalize_plan_json(&file.session_id, content)
        }
        SourceKind::TranscriptJson => {
            archon_world_model::artifact_ingest::normalize_transcript_json(
                &file.session_id,
                content,
            )
        }
        SourceKind::TranscriptJsonl => {
            archon_world_model::artifact_ingest::normalize_transcript_jsonl(
                &file.session_id,
                content,
            )
        }
        SourceKind::AgentOutputJson => {
            archon_world_model::artifact_ingest::normalize_agent_output_json(
                &file.session_id,
                content,
            )
        }
        SourceKind::MemoryJson => {
            archon_world_model::artifact_ingest::normalize_memory_json(&file.session_id, content)
        }
        SourceKind::RetrospectiveJson => {
            archon_world_model::artifact_ingest::normalize_retrospective_json(
                &file.session_id,
                content,
            )
        }
        SourceKind::AgentEvolutionJson => {
            archon_world_model::artifact_ingest::normalize_agent_evolution_json(
                &file.session_id,
                content,
            )
        }
        SourceKind::GenericArtifactJson => {
            archon_world_model::artifact_ingest::normalize_agent_output_json(
                &file.session_id,
                content,
            )
        }
        SourceKind::GenericArtifactJsonl => {
            archon_world_model::artifact_ingest::normalize_transcript_jsonl(
                &file.session_id,
                content,
            )
        }
    }
}

fn session_source_files(
    session_id: &str,
    sessions_root: &Path,
    cwd: &Path,
) -> Result<Vec<SourceFile>> {
    let session_dir = sessions_root.join(session_id);
    let mut files = Vec::new();
    push_if_file(
        &mut files,
        session_id,
        session_dir.join("activity").join("events.jsonl"),
        SourceKind::ActivityJsonl,
    );
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("subagents"),
        "jsonl",
        SourceKind::TranscriptJsonl,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("subagents"),
        "json",
        SourceKind::AgentOutputJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("plans"),
        "json",
        SourceKind::PlanJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("provider-runtime"),
        "jsonl",
        SourceKind::ProviderRuntimeJsonl,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("transcripts"),
        "json",
        SourceKind::TranscriptJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("outputs"),
        "json",
        SourceKind::AgentOutputJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("memory"),
        "json",
        SourceKind::MemoryJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("retrospectives"),
        "json",
        SourceKind::RetrospectiveJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &session_dir.join("agent-evolution"),
        "json",
        SourceKind::AgentEvolutionJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &cwd.join(".archon").join("agent-evolution").join(session_id),
        "json",
        SourceKind::AgentEvolutionJson,
    )?;
    push_matching(
        &mut files,
        session_id,
        &cwd.join(".archon")
            .join("pipelines")
            .join(session_id)
            .join("exports"),
        "jsonl",
        SourceKind::TraceExportJsonl,
    )?;
    for root in [
        cwd.join(".archon").join("plugin-artifacts"),
        cwd.join(".archon").join("artifacts"),
        cwd.join(".archon").join("runs"),
    ] {
        push_plugin_artifacts(&mut files, session_id, &root)?;
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn push_plugin_artifacts(files: &mut Vec<SourceFile>, session_id: &str, dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            push_plugin_artifacts(files, session_id, &path)?;
            continue;
        }
        let path_text = path.to_string_lossy();
        if !path_text.contains(session_id) {
            continue;
        }
        match path.extension().and_then(|value| value.to_str()) {
            Some("json") => push_if_file(files, session_id, path, SourceKind::GenericArtifactJson),
            Some("jsonl") => {
                push_if_file(files, session_id, path, SourceKind::GenericArtifactJsonl)
            }
            _ => {}
        }
    }
    Ok(())
}

fn push_matching(
    files: &mut Vec<SourceFile>,
    session_id: &str,
    dir: &Path,
    extension: &str,
    kind: SourceKind,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            push_if_file(files, session_id, path, kind);
        }
    }
    Ok(())
}

fn push_if_file(files: &mut Vec<SourceFile>, session_id: &str, path: PathBuf, kind: SourceKind) {
    if path.is_file() {
        files.push(SourceFile {
            session_id: session_id.to_string(),
            path,
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_world_model::schema::{WorldActionKind, WorldTraceRow};

    async fn heuristic_labeler() -> WorldModelLabelingRuntime {
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.labeler.llm_enabled = false;
        let env_vars = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
        WorldModelLabelingRuntime::from_config(&config, &env_vars)
            .await
            .unwrap()
    }

    fn write_activity_events(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            r#"{"event_id":"e1","session_id":"s1","kind":"tool_completed","status":"completed","message":"first","created_at":"2026-05-17T00:00:00Z"}
{"event_id":"e2","session_id":"s1","kind":"tool_failed","status":"failed","message":"second failed","created_at":"2026-05-17T00:00:01Z"}
"#,
        )
        .unwrap();
    }

    fn ledger_count(world_root: &Path, row_id: &str) -> usize {
        let ledger = world_root.join("ledgers").join("world-trace-rows.jsonl");
        fs::read_to_string(ledger).unwrap().matches(row_id).count()
    }

    #[test]
    fn session_source_files_include_activity_and_subagent_transcripts() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = "s1";
        let session = temp.path().join(session_id);
        fs::create_dir_all(session.join("activity")).unwrap();
        fs::create_dir_all(session.join("subagents")).unwrap();
        fs::create_dir_all(session.join("memory")).unwrap();
        fs::create_dir_all(
            temp.path()
                .join(".archon/plugin-artifacts/custom")
                .join(session_id),
        )
        .unwrap();
        fs::write(session.join("activity").join("events.jsonl"), "").unwrap();
        fs::write(session.join("subagents").join("agent.jsonl"), "{}\n").unwrap();
        fs::write(session.join("memory").join("memories.json"), "{}").unwrap();
        fs::write(
            temp.path()
                .join(".archon/plugin-artifacts/custom")
                .join(session_id)
                .join("plugin-output.json"),
            "{}",
        )
        .unwrap();

        let files = session_source_files(session_id, temp.path(), temp.path()).unwrap();

        assert_eq!(files.len(), 4);
        assert!(
            files
                .iter()
                .any(|file| file.kind == SourceKind::ActivityJsonl)
        );
        assert!(
            files
                .iter()
                .any(|file| file.kind == SourceKind::TranscriptJsonl)
        );
        assert!(files.iter().any(|file| file.kind == SourceKind::MemoryJson));
        assert!(
            files
                .iter()
                .any(|file| file.kind == SourceKind::GenericArtifactJson)
        );
    }

    #[tokio::test]
    async fn backfill_skips_existing_rows_before_persisting() {
        let temp = tempfile::tempdir().unwrap();
        let sessions_root = temp.path().join("sessions");
        let world_root = temp.path().join("world");
        let activity_path = sessions_root
            .join("s1")
            .join("activity")
            .join("events.jsonl");
        write_activity_events(&activity_path);

        let store = WorldModelStore::open(&world_root).unwrap();
        let existing = WorldTraceRow::new("s1", WorldActionKind::ToolCall)
            .with_row_id("world-row-activity-e1");
        store.persist_rows(&[existing]).unwrap();
        drop(store);

        let labeler = heuristic_labeler().await;
        let report = ingest_backfill(
            &sessions_root,
            &world_root,
            temp.path(),
            RetentionPolicy::default(),
            &labeler,
        )
        .await
        .unwrap();

        assert_eq!(report.files_read, 1);
        assert_eq!(report.rows_normalized, 2);
        assert_eq!(report.rows_skipped, 1);
        assert_eq!(report.rows_persisted, 1);
        assert_eq!(report.cozo_rows, 1);
        assert_eq!(report.sources_summary(), "activity_jsonl:2");

        let store = WorldModelStore::open(&world_root).unwrap();
        let rows = store.load_rows().unwrap();
        let row_ids = rows
            .iter()
            .map(|row| row.row_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(rows.len(), 2);
        assert!(row_ids.contains("world-row-activity-e1"));
        assert!(row_ids.contains("world-row-activity-e2"));
        assert_eq!(ledger_count(&world_root, "world-row-activity-e1"), 1);
        assert_eq!(ledger_count(&world_root, "world-row-activity-e2"), 1);
    }

    #[tokio::test]
    async fn session_ingest_keeps_existing_row_semantics() {
        let temp = tempfile::tempdir().unwrap();
        let sessions_root = temp.path().join("sessions");
        let world_root = temp.path().join("world");
        let activity_path = sessions_root
            .join("s1")
            .join("activity")
            .join("events.jsonl");
        fs::create_dir_all(activity_path.parent().unwrap()).unwrap();
        fs::write(
            &activity_path,
            r#"{"event_id":"e1","session_id":"s1","kind":"tool_completed","status":"completed","message":"first","created_at":"2026-05-17T00:00:00Z"}
"#,
        )
        .unwrap();

        let store = WorldModelStore::open(&world_root).unwrap();
        let existing = WorldTraceRow::new("s1", WorldActionKind::ToolCall)
            .with_row_id("world-row-activity-e1");
        store.persist_rows(&[existing]).unwrap();
        drop(store);

        let labeler = heuristic_labeler().await;
        let report = ingest_session(
            "s1",
            &sessions_root,
            &world_root,
            temp.path(),
            RetentionPolicy::default(),
            &labeler,
        )
        .await
        .unwrap();

        assert_eq!(report.files_read, 1);
        assert_eq!(report.rows_normalized, 1);
        assert_eq!(report.rows_skipped, 0);
        assert_eq!(report.rows_persisted, 1);
        assert_eq!(report.cozo_rows, 1);
        assert_eq!(ledger_count(&world_root, "world-row-activity-e1"), 2);
    }
}
