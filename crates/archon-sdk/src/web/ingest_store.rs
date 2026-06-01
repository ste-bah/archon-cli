use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::{
    WebRuntimePaths,
    api::EffectivePolicySummary,
    ingest::{
        WebDocStoreItem, WebIndexFailureItem, WebIndexJobItem, WebIndexQueueSummary, WebIngestJob,
        WebIngestSummary, WebKbCreateRequest, WebKnowledgeBaseItem, WebKnowledgeStats,
        WebVideoStoreItem, ingest_allowed,
    },
    inspect::PathProbe,
};

pub(crate) fn summary(
    paths: &WebRuntimePaths,
    policy: &EffectivePolicySummary,
    jobs: Vec<WebIngestJob>,
) -> WebIngestSummary {
    let (allowed, policy_reason) = ingest_allowed(policy);
    let mut warnings = Vec::new();
    let db_path = evidence_db_path(paths);
    let mut documents = Vec::new();
    let mut videos = Vec::new();
    let mut knowledge_stats = WebKnowledgeStats::default();
    if let Ok(db) = open_docs_db(paths) {
        documents = doc_items(&db, &mut warnings);
        videos = video_items(&db, &mut warnings);
        knowledge_stats = kb_stats(&db, &mut warnings);
        let index_queue = index_queue_summary(&db, &mut warnings);
        let index_jobs = index_jobs(&db, &mut warnings);
        let index_failures = index_failures(&db, &mut warnings);
        return WebIngestSummary {
            allowed,
            policy_reason,
            stores: stores(paths, db_path),
            documents,
            videos,
            knowledge_bases: knowledge_bases(paths),
            kb_stats: knowledge_stats,
            jobs,
            index_queue,
            index_jobs,
            index_failures,
            warnings,
        };
    } else {
        warnings.push("document store is not readable right now".into());
    }
    WebIngestSummary {
        allowed,
        policy_reason,
        stores: stores(paths, db_path),
        documents,
        videos,
        knowledge_bases: knowledge_bases(paths),
        kb_stats: knowledge_stats,
        jobs,
        index_queue: WebIndexQueueSummary::default(),
        index_jobs: Vec::new(),
        index_failures: Vec::new(),
        warnings,
    }
}

fn stores(paths: &WebRuntimePaths, db_path: PathBuf) -> Vec<PathProbe> {
    vec![
        probe("document store", db_path),
        probe("project docs", paths.cwd.join(".archon/docs")),
        probe("project kb", paths.cwd.join(".archon/kb")),
        probe("video artifacts", paths.cwd.join(".archon/video-artifacts")),
    ]
}

pub(crate) fn create_kb(
    paths: &WebRuntimePaths,
    request: &WebKbCreateRequest,
) -> Result<WebKnowledgeBaseItem> {
    let name = request.name.trim();
    if name.is_empty() {
        anyhow::bail!("knowledge base name is required");
    }
    let scope = if request.scope == "home" {
        "home"
    } else {
        "project"
    };
    let root = if scope == "home" {
        home_archon().join("kb")
    } else {
        paths.cwd.join(".archon/kb")
    };
    let dir = root.join(slugify(name));
    fs::create_dir_all(&dir)?;
    let readme = dir.join("README.md");
    if !readme.exists() {
        fs::write(
            &readme,
            format!(
                "# {name}\n\n{}\n",
                request
                    .description
                    .as_deref()
                    .unwrap_or("Knowledge base notes.")
            ),
        )?;
    }
    Ok(kb_item(name, scope, &dir))
}

fn doc_items(db: &DbInstance, warnings: &mut Vec<String>) -> Vec<WebDocStoreItem> {
    match db.run_script(
        "?[document_id, source_path, media_type, discovered_at, status] :=
            *doc_sources{document_id, source_path, media_type, discovered_at, status}",
        BTreeMap::new(),
        ScriptMutability::Immutable,
    ) {
        Ok(result) => result
            .rows
            .into_iter()
            .take(100)
            .map(|row| {
                let document_id = str_cell(&row, 0);
                WebDocStoreItem {
                    chunks: count_by_doc(db, "doc_chunks", "chunk_id", &document_id),
                    pages: count_by_doc(db, "doc_pages", "page_id", &document_id),
                    artifacts: count_by_doc(db, "doc_artifacts", "artifact_id", &document_id),
                    ocr_runs: count_by_doc(db, "doc_ocr_runs", "ocr_run_id", &document_id),
                    document_id,
                    source_path: str_cell(&row, 1),
                    media_type: str_cell(&row, 2),
                    discovered_at: str_cell(&row, 3),
                    status: str_cell(&row, 4),
                }
            })
            .collect(),
        Err(err) => {
            warnings.push(format!("document listing failed: {err}"));
            Vec::new()
        }
    }
}

fn video_items(db: &DbInstance, warnings: &mut Vec<String>) -> Vec<WebVideoStoreItem> {
    match db.run_script(
        "?[video_id, document_id, source_url, title, duration_ms, ingest_status] :=
            *video_sources{video_id, document_id, source_url, title, duration_ms, ingest_status}",
        BTreeMap::new(),
        ScriptMutability::Immutable,
    ) {
        Ok(result) => result
            .rows
            .into_iter()
            .take(50)
            .map(|row| {
                let video_id = str_cell(&row, 0);
                let document_id = str_cell(&row, 1);
                WebVideoStoreItem {
                    chunks: count_by_doc(db, "doc_chunks", "chunk_id", &document_id),
                    transcript_segments: count_by_video(
                        db,
                        "video_transcript_segments",
                        "segment_id",
                        &video_id,
                    ),
                    frames: count_by_video(db, "video_frame_descriptions", "frame_id", &video_id),
                    video_id,
                    document_id,
                    source: str_cell(&row, 2),
                    title: str_cell(&row, 3),
                    duration_ms: int_cell(&row, 4),
                    status: str_cell(&row, 5),
                }
            })
            .collect(),
        Err(err) => {
            warnings.push(format!("video listing failed: {err}"));
            Vec::new()
        }
    }
}

fn kb_stats(db: &DbInstance, warnings: &mut Vec<String>) -> WebKnowledgeStats {
    let stats = WebKnowledgeStats {
        chunks: count_relation(db, "doc_chunks", "chunk_id"),
        claims: count_relation(db, "kb_claims", "claim_id"),
        entities: count_relation(db, "kb_entities", "entity_id"),
        relations: count_relation(db, "kb_relations", "relation_id"),
        contradictions: count_relation(db, "kb_contradictions", "contradiction_id"),
    };
    if stats == WebKnowledgeStats::default() {
        warnings.push("knowledge graph relations are empty or not initialized yet".into());
    }
    stats
}

fn index_queue_summary(db: &DbInstance, warnings: &mut Vec<String>) -> WebIndexQueueSummary {
    let summary = WebIndexQueueSummary {
        pending: count_index_status(db, "pending"),
        leased: count_index_status(db, "leased"),
        indexed: count_index_status(db, "indexed"),
        failed: count_index_status(db, "failed"),
    };
    if summary == WebIndexQueueSummary::default() {
        warnings.push("semantic index queue is empty or not initialized yet".into());
    }
    summary
}

fn index_jobs(db: &DbInstance, warnings: &mut Vec<String>) -> Vec<WebIndexJobItem> {
    match db.run_script(
        "?[job_id, scope, provider, status, started_at, leased, indexed, failed, skipped, last_error] :=
            *doc_index_jobs{job_id, scope, provider, status, started_at, leased, indexed, failed, skipped, last_error}
         :order -started_at :limit 8",
        BTreeMap::new(),
        ScriptMutability::Immutable,
    ) {
        Ok(result) => result.rows.iter().map(|row| index_job_from_row(row)).collect(),
        Err(err) => {
            warnings.push(format!("index job listing failed: {err}"));
            Vec::new()
        }
    }
}

fn index_failures(db: &DbInstance, warnings: &mut Vec<String>) -> Vec<WebIndexFailureItem> {
    match db.run_script(
        "?[chunk_id, document_id, attempt_count, last_error, updated_at] :=
            *doc_index_queue{chunk_id, document_id, status, attempt_count, last_error, updated_at},
            status = \"failed\"
         :order -updated_at :limit 8",
        BTreeMap::new(),
        ScriptMutability::Immutable,
    ) {
        Ok(result) => result
            .rows
            .iter()
            .map(|row| index_failure_from_row(row))
            .collect(),
        Err(err) => {
            warnings.push(format!("index failure listing failed: {err}"));
            Vec::new()
        }
    }
}

fn count_index_status(db: &DbInstance, status: &str) -> u64 {
    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status));
    run_count(
        db,
        "?[count(chunk_id)] := *doc_index_queue{chunk_id, status}, status = $status",
        params,
    )
}

fn index_job_from_row(row: &[DataValue]) -> WebIndexJobItem {
    WebIndexJobItem {
        job_id: str_cell(row, 0),
        scope: str_cell(row, 1),
        provider: str_cell(row, 2),
        status: str_cell(row, 3),
        started_at: str_cell(row, 4),
        leased: int_cell(row, 5),
        indexed: int_cell(row, 6),
        failed: int_cell(row, 7),
        skipped: int_cell(row, 8),
        last_error: str_cell(row, 9),
    }
}

fn index_failure_from_row(row: &[DataValue]) -> WebIndexFailureItem {
    WebIndexFailureItem {
        chunk_id: str_cell(row, 0),
        document_id: str_cell(row, 1),
        attempt_count: int_cell(row, 2),
        last_error: str_cell(row, 3),
        updated_at: str_cell(row, 4),
    }
}

fn count_by_doc(db: &DbInstance, relation: &str, key: &str, document_id: &str) -> u64 {
    let script =
        format!("?[count(id)] := *{relation}{{{key}: id, document_id}}, document_id = $did");
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    run_count(db, &script, params)
}

fn count_by_video(db: &DbInstance, relation: &str, key: &str, video_id: &str) -> u64 {
    let script = format!("?[count(id)] := *{relation}{{{key}: id, video_id}}, video_id = $vid");
    let mut params = BTreeMap::new();
    params.insert("vid".into(), DataValue::from(video_id));
    run_count(db, &script, params)
}

fn count_relation(db: &DbInstance, relation: &str, key: &str) -> u64 {
    let script = format!("?[count(id)] := *{relation}{{{key}: id}}");
    run_count(db, &script, BTreeMap::new())
}

fn run_count(db: &DbInstance, script: &str, params: BTreeMap<String, DataValue>) -> u64 {
    db.run_script(script, params, ScriptMutability::Immutable)
        .ok()
        .and_then(|result| {
            result
                .rows
                .first()
                .and_then(|row| row.first())
                .and_then(DataValue::get_int)
        })
        .unwrap_or(0)
        .max(0) as u64
}

fn knowledge_bases(paths: &WebRuntimePaths) -> Vec<WebKnowledgeBaseItem> {
    [
        ("project", paths.cwd.join(".archon/kb")),
        ("home", home_archon().join("kb")),
    ]
    .into_iter()
    .flat_map(|(scope, root)| kb_dirs(scope, &root))
    .collect()
}

fn kb_dirs(scope: &str, root: &Path) -> Vec<WebKnowledgeBaseItem> {
    fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry.metadata().ok()?.is_dir().then(|| {
                let name = entry.file_name().to_string_lossy().to_string();
                kb_item(&name, scope, &entry.path())
            })
        })
        .collect()
}

fn kb_item(name: &str, scope: &str, path: &Path) -> WebKnowledgeBaseItem {
    let (files, bytes) = dir_stats(path, 0);
    WebKnowledgeBaseItem {
        name: name.into(),
        scope: scope.into(),
        path: path.to_string_lossy().to_string(),
        files,
        bytes,
        exists: path.exists(),
    }
}

fn open_docs_db(paths: &WebRuntimePaths) -> Result<DbInstance> {
    let path = evidence_db_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        &path.to_string_lossy(),
        &format!("open web ingest store at {}", path.display()),
    )?;
    Ok(db)
}

fn evidence_db_path(paths: &WebRuntimePaths) -> PathBuf {
    [
        "ARCHON_DOCS_DB_PATH",
        "ARCHON_VIDEO_DB_PATH",
        "ARCHON_KB_DB_PATH",
        "ARCHON_EVIDENCE_DB_PATH",
    ]
    .iter()
    .find_map(|key| std::env::var_os(key).filter(|value| !value.is_empty()))
    .map(PathBuf::from)
    .unwrap_or_else(|| paths.cwd.join(".archon/archon-data.db"))
}

fn probe(label: impl Into<String>, path: PathBuf) -> PathProbe {
    let (files, bytes) = dir_stats(&path, 0);
    PathProbe {
        label: label.into(),
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
        files,
        bytes,
    }
}

fn dir_stats(path: &Path, depth: usize) -> (u64, u64) {
    if depth > 3 {
        return (0, 0);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return (0, 0);
    };
    if metadata.is_file() {
        return (1, metadata.len());
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| dir_stats(&entry.path(), depth + 1))
        .fold((0, 0), |(files, bytes), (child_files, child_bytes)| {
            (files + child_files, bytes + child_bytes)
        })
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '-' | '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

fn str_cell(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn int_cell(row: &[DataValue], index: usize) -> i64 {
    row.get(index).and_then(DataValue::get_int).unwrap_or(0)
}
