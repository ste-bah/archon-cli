//! Knowledge intelligence CLI handler.

use std::path::PathBuf;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::KbAction;

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

    match action {
        KbAction::Ingest { source, domain: _ } => ingest_source(&db, &source).await,
        KbAction::List => list_chunks(&db).await,
        KbAction::Search { query, limit, mode } => search(&engine, &query, limit, &mode).await,
        KbAction::Process {
            claims,
            entities,
            relations,
            contradictions,
        } => process(&engine, claims, entities, relations, contradictions).await,
        KbAction::Claims => print_claims(&engine).await,
        KbAction::Entities => print_entities(&engine).await,
        KbAction::Relations => print_relations(&engine).await,
        KbAction::Contradictions => print_contradictions(&engine).await,
        KbAction::Stats => print_stats(&engine).await,
    }
}

async fn ingest_source(db: &DbInstance, source: &str) -> Result<()> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let response = reqwest::get(source).await?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("URL ingest failed for {source}: HTTP {status}");
        }
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("text/plain")
            .split(';')
            .next()
            .unwrap_or("text/plain")
            .to_string();
        if !is_text_url_media_type(&media_type) {
            anyhow::bail!(
                "KB URL ingest only supports text-like URLs; got `{media_type}` from {source}. \
                 Download binary/media sources and run `archon docs ingest <path>` or `archon kb ingest <path>`."
            );
        }
        let body = response.text().await?;
        let result = archon_docs::ingest_text::ingest_text_source(db, source, &media_type, &body)?;
        println!("Ingested: {}", result.document_id);
        if !result.was_new {
            println!("Skipped duplicate: true");
        }
        println!("Chunks: {}", result.chunks_registered);
        return Ok(());
    }
    let path = PathBuf::from(source);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {source}");
    }
    if path.is_dir() {
        let result = archon_docs::ingest::ingest_directory(db, &path).await?;
        println!("Ingested: {} sources", result.sources_registered);
        println!("Skipped duplicates: {}", result.sources_skipped_duplicate);
        println!("Failed: {}", result.sources_failed);
    } else {
        let result = archon_docs::ingest::ingest_file(db, &path).await?;
        let chunks = archon_docs::store::list_chunks_for_doc(db, &result.document_id)?;
        println!("Ingested: {}", result.document_id);
        println!("Chunks: {}", chunks.len());
    }
    Ok(())
}

fn is_text_url_media_type(media_type: &str) -> bool {
    let normalized = media_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    normalized.starts_with("text/")
        || normalized.ends_with("+json")
        || normalized.ends_with("+xml")
        || matches!(
            normalized.as_str(),
            "application/json"
                | "application/ld+json"
                | "application/x-ndjson"
                | "application/xml"
                | "application/xhtml+xml"
                | "application/rss+xml"
                | "application/atom+xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
        )
}

async fn list_chunks(db: &DbInstance) -> Result<()> {
    let chunks = archon_knowledge::store::list_doc_chunks(db)?;
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
) -> Result<()> {
    let opts =
        archon_knowledge::ProcessOptions::from_flags(claims, entities, relations, contradictions);
    let report = engine.process_documents(opts)?;
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
) -> Result<()> {
    let options = search_options_for_cli(engine.db(), query, limit, mode)?;
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
    Ok(archon_knowledge::hybrid_retriever::SearchOptions {
        mode,
        top_k: limit,
        query_embedding,
        ..Default::default()
    })
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
    if archon_docs::embed::get_provider().is_none()
        && let Err(e) = archon_docs::embed::init_default_provider()
    {
        eprintln!("Warning: semantic embedding provider unavailable: {e}");
        return None;
    }
    let provider = archon_docs::embed::get_provider()?;
    match archon_docs::retrieval::index_pending_chunks(db) {
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
    fn url_media_type_gate_accepts_text_like_content() {
        for media_type in [
            "text/plain",
            "text/html; charset=utf-8",
            "application/json",
            "application/activity+json",
            "application/rss+xml",
            "application/x-yaml",
        ] {
            assert!(
                is_text_url_media_type(media_type),
                "{media_type} should be accepted"
            );
        }
    }

    #[test]
    fn url_media_type_gate_rejects_binary_content() {
        for media_type in ["application/pdf", "image/png", "audio/mpeg"] {
            assert!(
                !is_text_url_media_type(media_type),
                "{media_type} should be rejected"
            );
        }
    }

    #[test]
    fn exact_search_options_do_not_require_embedding_provider() {
        let db = test_db();
        let options = search_options_for_cli(&db, "plugin", 3, "exact").unwrap();
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
