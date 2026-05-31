//! Knowledge intelligence CLI handler.

use std::path::PathBuf;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::KbAction;
use crate::command::kb_ingest_output::{print_directory_result, print_file_result};

fn kb_db_path() -> PathBuf {
    crate::command::store_paths::evidence_db_path(&["ARCHON_KB_DB_PATH"])
}

fn open_db() -> Result<DbInstance> {
    let db_path = kb_db_path();
    let db = crate::command::store_paths::open_sqlite_db(&db_path, "knowledge")?;
    archon_docs::schema::ensure_doc_schema(&db)?;
    archon_knowledge::schema::ensure_knowledge_schema(&db)?;
    Ok(db)
}

pub async fn handle_kb_command(action: KbAction) -> Result<()> {
    let db = open_db()?;
    let engine = archon_knowledge::KnowledgeEngine::new(db.clone())?;
    let policy = load_policy();

    match action {
        KbAction::Ingest { source, kb } => {
            let vlm_report = archon_docs::vlm::factory::configure_registered_provider(&policy);
            ingest_source(&db, &source, kb.as_deref(), &policy, &vlm_report).await
        }
        KbAction::List { kb } => list_chunks(&db, kb.as_deref()).await,
        KbAction::Search {
            query,
            limit,
            mode,
            kb,
        } => search(&engine, &query, limit, &mode, kb.as_deref()).await,
        KbAction::Process {
            claims,
            entities,
            relations,
            contradictions,
            kb,
        } => {
            process(
                &engine,
                claims,
                entities,
                relations,
                contradictions,
                kb.as_deref(),
            )
            .await
        }
        KbAction::Reprocess { kb, defer_index } => {
            crate::command::kb_reprocess::handle_reprocess(&kb, defer_index).await
        }
        KbAction::Claims => print_claims(&engine).await,
        KbAction::Entities => print_entities(&engine).await,
        KbAction::Relations => print_relations(&engine).await,
        KbAction::Contradictions => print_contradictions(&engine).await,
        KbAction::Stats => print_stats(&engine).await,
    }
}

fn load_policy() -> archon_policy::EffectivePolicy {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default()
}

async fn ingest_source(
    db: &DbInstance,
    source: &str,
    kb: Option<&str>,
    policy: &archon_policy::EffectivePolicy,
    vlm_report: &archon_docs::vlm::factory::VlmProviderInitReport,
) -> Result<()> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let document_id =
            crate::command::kb_url::ingest_url(db, source, policy, vlm_report).await?;
        attach_to_kb(db, kb, &document_id)?;
        return Ok(());
    }
    let path = PathBuf::from(source);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {source}");
    }
    if path.is_dir() {
        let result = archon_docs::ingest::ingest_directory_with_policy(db, &path, policy).await?;
        print_directory_result(&result, vlm_report);
        if let Some(kb_id) = normalize_kb_id(kb) {
            let assigned = attach_path_documents_to_kb(db, &kb_id, &path)?;
            println!("KB: {kb_id} ({assigned} document(s) attached)");
        }
    } else {
        let result = archon_docs::ingest::ingest_file_with_policy(db, &path, policy).await?;
        print_file_result(db, &result, vlm_report)?;
        attach_to_kb(db, kb, &result.document_id)?;
    }
    Ok(())
}

async fn list_chunks(db: &DbInstance, kb: Option<&str>) -> Result<()> {
    let chunks = if let Some(kb_id) = normalize_kb_id(kb) {
        archon_knowledge::store::list_doc_chunks_for_kb(db, &kb_id)?
    } else {
        archon_knowledge::store::list_doc_chunks(db)?
    };
    for chunk in &chunks {
        println!(
            "{}  {}  {}",
            chunk.chunk_id,
            chunk.document_id,
            preview(&chunk.content)
        );
    }
    println!("{} chunks", chunks.len());
    Ok(())
}

async fn process(
    engine: &archon_knowledge::KnowledgeEngine,
    claims: bool,
    entities: bool,
    relations: bool,
    contradictions: bool,
    kb: Option<&str>,
) -> Result<()> {
    let opts =
        archon_knowledge::ProcessOptions::from_flags(claims, entities, relations, contradictions);
    let report = if let Some(kb_id) = normalize_kb_id(kb) {
        engine.process_kb(&kb_id, opts)?
    } else {
        engine.process_documents(opts)?
    };
    println!("Knowledge process complete");
    println!("Chunks seen: {}", report.chunks_seen);
    println!("Claims: {}", report.claims_created);
    println!("Entities: {}", report.entities_created);
    println!("Relations: {}", report.relations_created);
    println!("Source quality records: {}", report.source_quality_records);
    println!("Contradictions: {}", report.contradictions_created);
    Ok(())
}

async fn search(
    engine: &archon_knowledge::KnowledgeEngine,
    query: &str,
    limit: usize,
    mode: &str,
    kb: Option<&str>,
) -> Result<()> {
    let options = search_options_for_cli(engine.db(), query, limit, mode, kb)?;
    let results = engine.search(query, &options)?;
    for result in &results {
        println!(
            "{}  score={:.3} exact={:.3} semantic={:.3}  {}",
            result.artifact_id,
            result.combined_score,
            result.exact_score,
            result.semantic_score,
            preview(&result.content)
        );
    }
    println!("{} results", results.len());
    Ok(())
}

fn search_options_for_cli(
    db: &DbInstance,
    query: &str,
    limit: usize,
    mode: &str,
    kb: Option<&str>,
) -> Result<archon_knowledge::hybrid_retriever::SearchOptions> {
    let parsed_mode = archon_knowledge::hybrid_retriever::SearchMode::parse(mode)?;
    let query_embedding = if parsed_mode == archon_knowledge::hybrid_retriever::SearchMode::Exact {
        None
    } else {
        query_embedding_for_search(db, query)
    };
    let mode = effective_search_mode(parsed_mode, query_embedding.is_some());
    if mode != parsed_mode {
        eprintln!("Warning: semantic KB search unavailable; using exact-only results.");
    }
    let document_filter = kb_document_filter(db, kb)?;
    Ok(archon_knowledge::hybrid_retriever::SearchOptions {
        mode,
        top_k: limit,
        query_embedding,
        document_filter,
        ..Default::default()
    })
}

fn kb_document_filter(db: &DbInstance, kb: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(kb_id) = normalize_kb_id(kb) else {
        return Ok(None);
    };
    let document_ids = archon_docs::store::list_kb_document_ids(db, &kb_id)?;
    if document_ids.is_empty() {
        eprintln!("Warning: knowledge base `{kb_id}` has no attached documents.");
    }
    Ok(Some(document_ids))
}

fn attach_to_kb(db: &DbInstance, kb: Option<&str>, document_id: &str) -> Result<()> {
    if let Some(kb_id) = normalize_kb_id(kb) {
        archon_docs::store::assign_document_to_kb(db, &kb_id, document_id)?;
        println!("KB: {kb_id}");
    }
    Ok(())
}

fn attach_path_documents_to_kb(
    db: &DbInstance,
    kb_id: &str,
    path: &std::path::Path,
) -> Result<usize> {
    let prefix = path.to_string_lossy();
    let mut assigned = 0;
    for doc in archon_docs::store::list_doc_sources(db)? {
        if doc.source_path.starts_with(prefix.as_ref()) {
            archon_docs::store::assign_document_to_kb(db, kb_id, &doc.document_id)?;
            assigned += 1;
        }
    }
    Ok(assigned)
}

fn normalize_kb_id(kb: Option<&str>) -> Option<String> {
    kb.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn effective_search_mode(
    requested: archon_knowledge::hybrid_retriever::SearchMode,
    has_query_embedding: bool,
) -> archon_knowledge::hybrid_retriever::SearchMode {
    if requested != archon_knowledge::hybrid_retriever::SearchMode::Exact && !has_query_embedding {
        archon_knowledge::hybrid_retriever::SearchMode::Exact
    } else {
        requested
    }
}

fn query_embedding_for_search(db: &DbInstance, query: &str) -> Option<Vec<f32>> {
    if let Err(e) = crate::command::docs_embedding::init_embedding(db) {
        eprintln!("Warning: semantic embedding provider unavailable: {e}");
        return None;
    }
    if archon_docs::embed::get_provider().is_none() {
        let detail = archon_docs::embed::last_init_error()
            .unwrap_or_else(|| "no embedding provider configured".into());
        eprintln!("Warning: semantic embedding provider unavailable: {detail}");
        return None;
    }
    let provider = archon_docs::embed::get_provider()?;
    match archon_docs::indexing::index_chunks(db, &archon_docs::indexing::IndexOptions::default()) {
        Ok(indexed) => {
            if indexed.failed > 0 {
                eprintln!(
                    "Warning: {} pending chunk(s) failed semantic indexing before search.",
                    indexed.failed
                );
            }
        }
        Err(e) => {
            eprintln!("Warning: semantic indexing unavailable: {e}");
            return None;
        }
    }
    match provider.embed_query(query) {
        Ok(embedding) => Some(embedding),
        Err(e) => {
            eprintln!("Warning: query embedding failed: {e}");
            None
        }
    }
}

async fn print_claims(engine: &archon_knowledge::KnowledgeEngine) -> Result<()> {
    let rows = engine.claims()?;
    for row in &rows {
        println!(
            "{}  {}  {:?}  {}",
            row.claim_id, row.document_id, row.polarity, row.text
        );
    }
    println!("{} claims", rows.len());
    Ok(())
}

async fn print_entities(engine: &archon_knowledge::KnowledgeEngine) -> Result<()> {
    let rows = engine.entities()?;
    for row in &rows {
        println!(
            "{}  {}  {}  mentions={}",
            row.entity_id, row.name, row.entity_type, row.mentions
        );
    }
    println!("{} entities", rows.len());
    Ok(())
}

async fn print_relations(engine: &archon_knowledge::KnowledgeEngine) -> Result<()> {
    let rows = engine.relations()?;
    for row in &rows {
        println!(
            "{}  {} -> {}  {}",
            row.relation_id, row.source_entity_id, row.target_entity_id, row.relation_type
        );
    }
    println!("{} relations", rows.len());
    Ok(())
}

async fn print_contradictions(engine: &archon_knowledge::KnowledgeEngine) -> Result<()> {
    let rows = engine.contradictions()?;
    for row in &rows {
        println!(
            "{}  {} <-> {}  {}",
            row.contradiction_id, row.left_claim_id, row.right_claim_id, row.explanation
        );
    }
    println!("{} contradictions", rows.len());
    Ok(())
}

async fn print_stats(engine: &archon_knowledge::KnowledgeEngine) -> Result<()> {
    let stats = engine.stats()?;
    println!("Claims: {}", stats.claims);
    println!("Entities: {}", stats.entities);
    println!("Relations: {}", stats.relations);
    println!("Source quality records: {}", stats.source_quality_records);
    println!("Contradictions: {}", stats.contradictions);
    Ok(())
}

fn preview(content: &str) -> String {
    const MAX: usize = 96;
    if content.len() <= MAX {
        content.to_string()
    } else {
        let prefix: String = content.chars().take(MAX).collect();
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        archon_docs::schema::ensure_doc_schema(&db).unwrap();
        archon_knowledge::schema::ensure_knowledge_schema(&db).unwrap();
        db
    }

    #[test]
    fn persist_text_document_writes_doc_and_chunk_rows() {
        let db = test_db();
        let result = archon_docs::ingest_text::ingest_text_source(
            &db,
            "https://example.test/policy.txt",
            "text/plain",
            "Archon uses CozoDB.",
        )
        .unwrap();
        let doc = archon_docs::store::get_doc_source(&db, &result.document_id)
            .unwrap()
            .unwrap();
        let chunks = archon_docs::store::list_chunks_for_doc(&db, &result.document_id).unwrap();
        assert_eq!(doc.source_path, "https://example.test/policy.txt");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("Archon uses CozoDB"));
    }

    #[test]
    fn persist_text_document_deduplicates_by_content_hash() {
        let db = test_db();
        let first = archon_docs::ingest_text::ingest_text_source(
            &db,
            "https://example.test/a.txt",
            "text/plain",
            "Same text.",
        )
        .unwrap();
        let second = archon_docs::ingest_text::ingest_text_source(
            &db,
            "https://example.test/b.txt",
            "text/plain",
            "Same text.",
        )
        .unwrap();
        assert_eq!(first.document_id, second.document_id);
        assert!(!second.was_new);
    }

    #[test]
    fn exact_search_options_do_not_require_embedding_provider() {
        let db = test_db();
        let options = search_options_for_cli(&db, "plugin", 3, "exact", None).unwrap();
        assert_eq!(
            options.mode,
            archon_knowledge::hybrid_retriever::SearchMode::Exact
        );
        assert_eq!(options.top_k, 3);
        assert!(options.query_embedding.is_none());
    }

    #[test]
    fn semantic_modes_downgrade_when_query_embedding_is_missing() {
        use archon_knowledge::hybrid_retriever::SearchMode;

        assert_eq!(
            effective_search_mode(SearchMode::Semantic, false),
            SearchMode::Exact
        );
        assert_eq!(
            effective_search_mode(SearchMode::Hybrid, false),
            SearchMode::Exact
        );
        assert_eq!(
            effective_search_mode(SearchMode::Hybrid, true),
            SearchMode::Hybrid
        );
    }
}
