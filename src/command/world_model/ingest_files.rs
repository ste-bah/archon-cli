use std::collections::BTreeMap;
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
    persist_sources(world_root, retention, &files, labeler).await
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
    persist_sources(world_root, retention, &files, labeler).await
}

async fn persist_sources(
    world_root: &Path,
    retention: RetentionPolicy,
    files: &[SourceFile],
    labeler: &WorldModelLabelingRuntime,
) -> Result<IngestReport> {
    let store = WorldModelStore::open(world_root)?;
    let mut report = IngestReport {
        files_read: 0,
        rows_normalized: 0,
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
        labeler.apply(&mut summary).await?;
        let persisted = store.persist_rows_with_retention(&summary.rows, retention)?;
        record_summary(&mut report, &summary, &persisted);
    }

    Ok(report)
}

fn record_summary(report: &mut IngestReport, summary: &IngestSummary, persisted: &PersistSummary) {
    report.files_read += 1;
    report.rows_normalized += summary.row_count();
    report.rows_persisted += persisted.jsonl_rows;
    report.cozo_rows += persisted.cozo_rows;
    report.warnings += summary.warning_count();
    report.ledger_path = persisted.ledger_path.clone();
    report.db_path = persisted.db_path.clone();
    *report.sources.entry(summary.source.clone()).or_default() += summary.row_count();
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
}
