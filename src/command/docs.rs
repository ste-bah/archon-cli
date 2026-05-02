//! Document management CLI handler.
//!
//! Wires `archon docs` subcommands to the archon-docs crate.

use std::path::PathBuf;
use std::fs;

use anyhow::Result;
use cozo::DbInstance;

use archon_docs::ingest;
use archon_docs::inspect;
use archon_docs::schema::ensure_doc_schema;
use archon_docs::status;

use crate::cli_args::DocsAction;

fn docs_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
        .join("docs.db")
}

fn open_db() -> Result<DbInstance> {
    let db_path = docs_db_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let path_str = db_path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("Failed to open document store at {path_str}: {e}"))?;
    ensure_doc_schema(&db)?;
    Ok(db)
}

pub async fn handle_docs_command(action: DocsAction) -> Result<()> {
    match action {
        DocsAction::Ingest { path } => handle_ingest(&path).await,
        DocsAction::List => handle_list().await,
        DocsAction::Show { document_id } => handle_show(&document_id).await,
        DocsAction::Status => handle_status().await,
        DocsAction::Chunks { document_id } => handle_chunks(&document_id).await,
        DocsAction::Inspect { document_id } => handle_inspect(&document_id).await,
    }
}

async fn handle_ingest(path_str: &str) -> Result<()> {
    let db = open_db()?;
    let path = PathBuf::from(path_str);

    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path_str);
    }

    if path.is_dir() {
        let result = ingest::ingest_directory(&db, &path).await?;
        println!("Ingested: {} sources", result.sources_registered);
        if result.sources_skipped_duplicate > 0 {
            println!("Skipped: {} duplicates", result.sources_skipped_duplicate);
        }
        if result.images_skipped > 0 {
            println!(
                "Note: {} image file(s) registered without OCR (not yet implemented in Phase 1)",
                result.images_skipped
            );
        }
        if result.sources_failed > 0 {
            println!("Failed: {} sources", result.sources_failed);
            for e in &result.errors {
                eprintln!("  Error: {e}");
            }
        }
    } else {
        match ingest::ingest_file(&db, &path).await {
            Ok(r) if r.was_new && r.ocr_skipped => println!(
                "Ingested: {}  (Note: image OCR not yet implemented in Phase 1; registered without text extraction)",
                r.document_id
            ),
            Ok(r) if r.was_new => println!("Ingested: {}", r.document_id),
            Ok(_) => println!("Skipped: duplicate (same content hash)"),
            Err(e) => anyhow::bail!("Ingest failed: {e}"),
        }
    }

    Ok(())
}

async fn handle_list() -> Result<()> {
    let db = open_db()?;
    let sources = archon_docs::store::list_doc_sources(&db)?;
    println!("{}", inspect::format_list_output(&sources));
    Ok(())
}

async fn handle_show(document_id: &str) -> Result<()> {
    let db = open_db()?;
    let output = inspect::inspect_document(&db, document_id)?;
    println!("{}", inspect::format_inspect_output(&output));
    Ok(())
}

async fn handle_status() -> Result<()> {
    let db = open_db()?;
    let summary = status::get_status_summary(&db)?;
    println!("Total sources:   {}", summary.total_sources);
    println!("  Discovered:    {}", summary.discovered);
    println!("  Ingesting:     {}", summary.ingesting);
    println!("  Ingested:      {}", summary.ingested);
    println!("  Processing:    {}", summary.processing);
    println!("  Processed:     {}", summary.processed);
    println!("  Failed:        {}", summary.failed);
    println!("Total chunks:    {}", summary.total_chunks);
    println!("Total pages:     {}", summary.total_pages);
    Ok(())
}

async fn handle_chunks(document_id: &str) -> Result<()> {
    let db = open_db()?;
    let chunks = archon_docs::store::list_chunks_for_doc(&db, document_id)?;
    if chunks.is_empty() {
        println!("No chunks for document {document_id}");
        return Ok(());
    }
    println!("{} chunk(s) for document {document_id}:", chunks.len());
    for chunk in &chunks {
        println!(
            "  {}  pages {}-{}  hash={}  embed={}",
            chunk.chunk_id,
            chunk.page_start,
            chunk.page_end,
            &chunk.content_hash[..16.min(chunk.content_hash.len())],
            chunk.embedding_status
        );
    }
    Ok(())
}

async fn handle_inspect(document_id: &str) -> Result<()> {
    let db = open_db()?;
    let output = inspect::inspect_document(&db, document_id)?;
    println!("{}", inspect::format_inspect_output(&output));
    Ok(())
}
