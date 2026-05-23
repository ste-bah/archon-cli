//! Document management CLI handler.
//!
//! Wires `archon docs` subcommands to the archon-docs crate.

use std::path::PathBuf;

use anyhow::Result;
use cozo::DbInstance;

use archon_docs::answer;
use archon_docs::embed;
use archon_docs::ingest;
use archon_docs::inspect;
use archon_docs::retrieval;
use archon_docs::schema::ensure_doc_schema;
use archon_docs::status;
use archon_docs::store;
use archon_docs::vlm::factory::{self as vlm_factory, VlmProviderInitStatus};

use crate::cli_args::DocsAction;

fn docs_db_path() -> PathBuf {
    crate::command::store_paths::evidence_db_path(&["ARCHON_DOCS_DB_PATH"])
}

fn open_db() -> Result<DbInstance> {
    let db_path = docs_db_path();
    let db = crate::command::store_paths::open_sqlite_db(&db_path, "document")?;
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
        DocsAction::Search { query, mode, debug } => handle_search(&query, &mode, debug).await,
        DocsAction::Answer { query } => handle_answer(&query).await,
        DocsAction::Provenance { chunk_or_answer_id } => {
            handle_provenance(&chunk_or_answer_id).await
        }
        DocsAction::Index { all } => handle_index(all).await,
        DocsAction::ModelStatus => handle_model_status().await,
    }
}

async fn handle_ingest(path_str: &str) -> Result<()> {
    let db = open_db()?;
    let _ = init_embedding(&db); // Eager indexing if model is available
    let policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();
    let vlm_report = vlm_factory::configure_registered_provider(&policy);
    let path = PathBuf::from(path_str);

    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path_str);
    }

    if path.is_dir() {
        let result = ingest::ingest_directory_with_policy(&db, &path, &policy).await?;
        println!("Ingested: {} sources", result.sources_registered);
        if result.sources_skipped_duplicate > 0 {
            println!("Skipped: {} duplicates", result.sources_skipped_duplicate);
        }
        if result.images_skipped > 0 {
            println!("Skipped OCR: {} image file(s)", result.images_skipped);
        }
        if result.image_ocr_completed > 0 {
            println!("Image OCR: {} image file(s)", result.image_ocr_completed);
        }
        if result.vlm_descriptions > 0 {
            println!(
                "VLM described: {} image file(s) via {}/{}",
                result.vlm_descriptions, vlm_report.provider, vlm_report.model
            );
        }
        if result.pdf_embedded_images_extracted > 0 || result.pdf_pages_rendered > 0 {
            println!(
                "PDF images: {} embedded extracted, {} skipped by filter, {} rendered page(s)",
                result.pdf_embedded_images_extracted,
                result.pdf_embedded_images_skipped_filter,
                result.pdf_pages_rendered
            );
            println!(
                "PDF image OCR: {} run(s), {} failure(s); VLM failures: {}",
                result.pdf_image_ocr_runs,
                result.pdf_image_ocr_failures,
                result.pdf_image_vlm_failures
            );
        }
        print_vlm_init_warning_if_needed(&vlm_report);
        for warning in &result.warnings {
            println!("Warning: {warning}");
        }
        if result.sources_failed > 0 {
            println!("Failed: {} sources", result.sources_failed);
            for e in &result.errors {
                eprintln!("  Error: {e}");
            }
        }
    } else {
        if is_video_path(&path) {
            let result = archon_video::ingest::ingest_video(
                archon_video::ingest::IngestOpts {
                    source: path.display().to_string(),
                    transcript_path: None,
                    metadata_only: false,
                    frames_mode: None,
                    asr_provider: None,
                    vlm: false,
                    yes: false,
                },
                &policy,
                &db,
            )
            .await?;
            println!(
                "Ingested video: {} ({} chunk(s))",
                result.video_id, result.chunk_count
            );
            return Ok(());
        }
        match ingest::ingest_file_with_policy(&db, &path, &policy).await {
            Ok(r) if r.pipeline_failed => {
                println!(
                    "Registered: {}  (processing failed; document status is Failed)",
                    r.document_id
                );
                print_vlm_init_warning_if_needed(&vlm_report);
                for warning in &r.warnings {
                    println!("Warning: {warning}");
                }
            }
            Ok(r) if r.was_new && r.ocr_skipped => {
                println!("Ingested: {}  (OCR skipped)", r.document_id);
                print_vlm_init_warning_if_needed(&vlm_report);
                for warning in &r.warnings {
                    println!("Warning: {warning}");
                }
            }
            Ok(r) if r.was_new => {
                println!("Ingested: {}", r.document_id);
                if r.vlm_descriptions > 0 {
                    println!(
                        "VLM descriptions: {} via {}/{}",
                        r.vlm_descriptions, vlm_report.provider, vlm_report.model
                    );
                }
                if r.image_embeddings_stored > 0 {
                    println!("Image embeddings: {}", r.image_embeddings_stored);
                }
                if r.pdf_embedded_images_extracted > 0 || r.pdf_pages_rendered > 0 {
                    println!(
                        "PDF images: {} embedded extracted, {} skipped by filter, {} rendered page(s)",
                        r.pdf_embedded_images_extracted,
                        r.pdf_embedded_images_skipped_filter,
                        r.pdf_pages_rendered
                    );
                    println!(
                        "PDF image OCR: {} run(s), {} failure(s); VLM failures: {}",
                        r.pdf_image_ocr_runs, r.pdf_image_ocr_failures, r.pdf_image_vlm_failures
                    );
                }
                print_vlm_init_warning_if_needed(&vlm_report);
                for warning in &r.warnings {
                    println!("Warning: {warning}");
                }
            }
            Ok(_) => println!("Skipped: duplicate (same content hash)"),
            Err(e) => anyhow::bail!("Ingest failed: {e}"),
        }
    }

    Ok(())
}

fn is_video_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp4" | "mkv" | "mov" | "webm" | "m4v"
            )
        })
        .unwrap_or(false)
}

fn print_vlm_init_warning_if_needed(report: &vlm_factory::VlmProviderInitReport) {
    if matches!(report.status, VlmProviderInitStatus::Skipped) {
        println!("Warning: {}", report.message);
    }
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
    println!(
        "PDF images:      {} extracted",
        summary.pdf_embedded_images_extracted
    );
    println!(
        "PDF image skips: {} filtered",
        summary.pdf_embedded_images_skipped_filter
    );
    println!(
        "PDF image OCR:   {} run(s), {} failed",
        summary.pdf_image_ocr_runs, summary.pdf_image_ocr_failures
    );
    println!(
        "PDF image VLM:   {} description(s), {} failed",
        summary.pdf_image_vlm_descriptions, summary.pdf_image_vlm_failures
    );
    println!("PDF rendered:    {} page(s)", summary.pdf_pages_rendered);
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

// ── Phase 2: retrieval, answer, provenance, model-status ──────────────

fn init_embedding(_db: &cozo::DbInstance) -> Result<()> {
    if embed::get_provider().is_none() {
        match embed::init_default_provider() {
            Ok(()) => {
                let provider = embed::get_provider().expect("just set");
                tracing::info!(
                    "embedding provider initialised: {}",
                    provider.backend_name()
                );
            }
            Err(e) => {
                tracing::warn!("embedding provider not available: {e}");
            }
        }
    }
    Ok(())
}

async fn ensure_search_ready(db: &cozo::DbInstance) -> Result<()> {
    init_embedding(db)?;
    Ok(())
}

async fn handle_search(query: &str, mode: &str, debug: bool) -> Result<()> {
    let db = open_db()?;
    ensure_search_ready(&db).await?;
    let mode = retrieval::SearchMode::parse(mode).map_err(|e| anyhow::anyhow!("{e}"))?;
    let policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();

    match retrieval::search_with_policy(&db, query, 10, mode, &policy) {
        Ok(results) => {
            if results.results.is_empty() && results.total_chunks == 0 {
                println!("No documents indexed. Use 'archon docs ingest <path>' first.");
                return Ok(());
            }
            if results.results.is_empty() {
                println!(
                    "No results found. {} chunks stored, {} chunks indexed, but none matched your query.",
                    results.total_chunks, results.total_indexed_chunks
                );
                return Ok(());
            }
            println!(
                "Found {} result(s) ({} chunks indexed, mode={}):\n",
                results.results.len(),
                results.total_indexed_chunks,
                results.mode.as_str()
            );
            if debug {
                match results.query_embedding_norm {
                    Some(norm) => println!("Query embedding norm: {:.6}", norm),
                    None => println!("Query embedding norm: n/a"),
                }
                println!("Top-k raw scores and citation chains:");
            }
            for (i, r) in results.results.iter().enumerate() {
                println!(
                    "  {}. {}  pages {}-{}  score={:.3}",
                    i + 1,
                    r.chunk_id,
                    r.page_start,
                    r.page_end,
                    r.score
                );
                if debug {
                    println!("     document: {}", r.document_id);
                    println!("     raw distance:        {:.4}", r.distance);
                    println!("     raw exact score:     {:.4}", r.exact_score);
                    println!("     raw semantic score:  {:.4}", r.semantic_score);
                    println!("     post-rerank score:   n/a");
                    println!("     final score:         {:.4}", r.score);
                    print_citation_chain(&db, &r.chunk_id)?;
                    println!(
                        "     content:  {}",
                        if r.content.len() > 120 {
                            format!("{}...", &r.content[..120])
                        } else {
                            r.content.clone()
                        }
                    );
                }
            }
            for warning in &results.warnings {
                println!("Warning: {warning}");
            }
            if !debug {
                println!("\nUse --debug for full content and provenance details.");
            }
        }
        Err(archon_docs::errors::DocsError::Embedding { message }) => {
            println!("{message}");
        }
        Err(archon_docs::errors::DocsError::ModelNotConfigured { message }) => {
            let mut msg = format!("Error: {message}");
            if let Some(init_err) = archon_docs::embed::last_init_error() {
                msg.push_str(&format!(
                    "\nLast init failure: {init_err}\nRun 'archon docs model-status' for details."
                ));
            }
            println!("{msg}");
        }
        Err(e) => {
            anyhow::bail!("search failed: {e}");
        }
    }

    Ok(())
}

async fn handle_answer(query: &str) -> Result<()> {
    let db = open_db()?;
    ensure_search_ready(&db).await?;

    match answer::answer(&db, query, 5) {
        Ok(ans) => {
            let edge_count = answer::persist_answer_provenance(&db, &ans)?;
            println!("Answer ID: {}\n", ans.answer_id);
            println!("{}\n", ans.text);
            if !ans.citations.is_empty() {
                println!("Citations ({edge_count} provenance edge(s)):");
                for (i, c) in ans.citations.iter().enumerate() {
                    println!(
                        "  [{}] {}  pages {}-{}  doc={}",
                        i + 1,
                        c.chunk_id,
                        c.page_start,
                        c.page_end,
                        c.document_id
                    );
                }
            }
        }
        Err(archon_docs::errors::DocsError::Embedding { message }) => {
            println!("{message}");
        }
        Err(archon_docs::errors::DocsError::ModelNotConfigured { message }) => {
            let mut msg = format!("Error: {message}");
            if let Some(init_err) = archon_docs::embed::last_init_error() {
                msg.push_str(&format!(
                    "\nLast init failure: {init_err}\nRun 'archon docs model-status' for details."
                ));
            }
            println!("{msg}");
        }
        Err(e) => {
            anyhow::bail!("answer failed: {e}");
        }
    }

    Ok(())
}

async fn handle_provenance(chunk_or_answer_id: &str) -> Result<()> {
    let db = open_db()?;

    // Try to look up as chunk ID directly
    match archon_docs::store::get_chunk_by_id(&db, chunk_or_answer_id) {
        Ok(Some(chunk)) => {
            println!("Chunk: {}", chunk.chunk_id);
            println!("  Document:  {}", chunk.document_id);
            println!("  Pages:     {}-{}", chunk.page_start, chunk.page_end);
            println!(
                "  Content:   {}",
                &chunk.content[..chunk.content.len().min(200)]
            );
            println!("  Hash:      {}", chunk.content_hash);
            println!("  Embedding: {}", chunk.embedding_status);
        }
        Ok(None) => {} // Not a chunk ID; will still print provenance edges below
        Err(e) => {
            tracing::warn!(chunk_or_answer_id = %chunk_or_answer_id, error = %e, "chunk lookup failed");
        }
    }

    // Always try to trace provenance edges
    let edges =
        archon_docs::store::list_provenance_from(&db, chunk_or_answer_id).unwrap_or_default();
    if !edges.is_empty() {
        println!("\nProvenance edges (outgoing):");
        for e in &edges {
            println!(
                "  {}  {:?}  -> {}",
                e.edge_id, e.edge_type, e.to_artifact_id
            );
        }
    }

    let edges_to =
        archon_docs::store::list_provenance_to(&db, chunk_or_answer_id).unwrap_or_default();
    if !edges_to.is_empty() {
        println!("\nProvenance edges (incoming):");
        for e in &edges_to {
            println!(
                "  {}  {:?}  <- {}",
                e.edge_id, e.edge_type, e.from_artifact_id
            );
        }
    }

    if edges.is_empty() && edges_to.is_empty() {
        println!(
            "No results found for '{}'. Provide a chunk_id or artifact_id.",
            chunk_or_answer_id
        );
    }

    Ok(())
}

async fn handle_model_status() -> Result<()> {
    let db = open_db()?;

    // Attempt provider init
    let init_start = std::time::Instant::now();
    if embed::get_provider().is_none() {
        let _ = embed::init_default_provider();
    }
    let init_elapsed = init_start.elapsed();

    match embed::get_provider() {
        Some(provider) => {
            println!("Backend:       {}", provider.backend_name());
            println!("Model name:    BGE-base-en-v1.5");
            println!("Dimension:     {}", provider.dimension());

            // Report init result
            println!("Init result:   ok (took {}ms)", init_elapsed.as_millis());

            // Smoke embed test
            let smoke_start = std::time::Instant::now();
            match provider.embed_query("hello") {
                Ok(v) => {
                    println!(
                        "Smoke embed:   ok (dim={}, took {}ms)",
                        v.len(),
                        smoke_start.elapsed().as_millis()
                    );
                }
                Err(e) => {
                    println!("Smoke embed:   failed: {}", e);
                    let fastembed_dir = dirs::data_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join("archon")
                        .join("fastembed");
                    println!(
                        "Hint: Cache may be corrupt. Try: rm -rf {}",
                        fastembed_dir.display()
                    );
                }
            }

            // Vectors stored (read-only)
            match archon_docs::store::count_embeddings(&db) {
                Ok(count) => println!("Vectors:       {} indexed", count),
                Err(e) => println!("Vectors:       unable to query — {}", e),
            }

            // Pending chunks (read-only)
            match archon_docs::store::count_pending_chunks(&db) {
                Ok(count) => println!("Pending:       {} chunks", count),
                Err(e) => println!("Pending:       unable to query — {}", e),
            }

            // HNSW index check (read-only — uses Cozo ::relations system query)
            match check_hnsw_index(&db, provider.dimension()) {
                Ok(true) => println!("HNSW index:    present"),
                Ok(false) => println!(
                    "HNSW index:    not yet created (will be created on first ingest with provider)"
                ),
                Err(e) => println!("HNSW index:    unable to check — {}", e),
            }
            println!("pdfimages:     {}", pdfimages_status());
        }
        None => {
            println!("Backend:       not-configured");
            println!("Dimension:     n/a");
            println!(
                "Init result:   failed (took {}ms)",
                init_elapsed.as_millis()
            );
            println!();
            println!("No embedding model is configured. To enable local embeddings:");
            println!("  1. Ensure the fastembed model is available (BGE-base-en-v1.5 quantized).");
            println!("  2. On first run, the model will be downloaded automatically.");
            println!("  3. Set ARCHON_EMBEDDING_MODEL_PATH to override the model location.");

            let fastembed_dir = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("archon")
                .join("fastembed");
            println!("  4. Cache dir: {}", fastembed_dir.display());
            println!("pdfimages:     {}", pdfimages_status());
            println!();
            println!("Search and answer commands will return structured errors until");
            println!("a model is configured.");
        }
    }

    match std::env::current_dir()
        .map_err(anyhow::Error::from)
        .and_then(|cwd| archon_policy::load_policy_for_workspace(&cwd).map_err(anyhow::Error::from))
    {
        Ok(load) => {
            let decision = load.policy.docs_vlm_decision();
            println!();
            println!(
                "VLM policy:    {} ({})",
                if decision.allowed {
                    "allowed"
                } else {
                    "denied"
                },
                decision.reason
            );
            let (configured_provider, configured_model) =
                vlm_factory::default_provider_summary(&load.policy);
            println!(
                "VLM config:    provider={} model={}",
                configured_provider,
                if configured_model.is_empty() {
                    "n/a"
                } else {
                    &configured_model
                }
            );
            let report = vlm_factory::diagnostic_report(&load.policy);
            println!(
                "VLM provider:  {}",
                match report.status {
                    VlmProviderInitStatus::Registered =>
                        format!("ok — {}/{} reachable", report.provider, report.model),
                    VlmProviderInitStatus::Disabled => format!("disabled — {}", report.message),
                    VlmProviderInitStatus::Skipped => format!(
                        "skipped — {}/{}: {}",
                        report.provider, report.model, report.message
                    ),
                }
            );
        }
        Err(e) => println!("VLM policy:    unable to load policy — {e}"),
    }

    Ok(())
}

fn pdfimages_status() -> String {
    let bin = std::env::var_os("ARCHON_PDFIMAGES_BIN").unwrap_or_else(|| "pdfimages".into());
    let display = std::path::PathBuf::from(&bin).display().to_string();
    match std::process::Command::new(&bin).arg("-v").output() {
        Ok(output) if output.status.success() || !output.stderr.is_empty() => {
            format!("available ({display})")
        }
        Ok(output) => format!(
            "missing or unhealthy ({display}) status={:?}",
            output.status.code()
        ),
        Err(e) => format!("missing ({display}) — {e}"),
    }
}

async fn handle_index(force_all: bool) -> Result<()> {
    let db = open_db()?;
    init_embedding(&db)?;

    let result = if force_all {
        retrieval::reindex_all(&db).map_err(|e| anyhow::anyhow!("reindex failed: {e}"))?
    } else {
        retrieval::index_pending_chunks(&db).map_err(|e| anyhow::anyhow!("index failed: {e}"))?
    };

    println!("Indexed: {} chunks", result.indexed);
    if result.failed > 0 {
        println!(
            "Failed:  {} chunks (use 'archon docs model-status' for diagnostics)",
            result.failed
        );
    }
    if result.skipped > 0 {
        println!("Skipped: {} chunks (already indexed)", result.skipped);
    }
    Ok(())
}

/// Check if vec_text_chunks relation exists without creating it.
/// Uses a lightweight query against the relation and catches "not found" errors.
fn check_hnsw_index(db: &cozo::DbInstance, _dim: usize) -> Result<bool> {
    match db.run_script(
        "?[count(chunk_id)] := *vec_text_chunks{chunk_id}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    ) {
        Ok(_) => Ok(true),
        Err(e) => {
            if e.to_string()
                .contains(archon_docs::errors::COZO_RELATION_NOT_FOUND)
            {
                Ok(false)
            } else {
                Err(anyhow::anyhow!("failed to query vec_text_chunks: {e}"))
            }
        }
    }
}

fn print_citation_chain(db: &DbInstance, chunk_id: &str) -> Result<()> {
    let outgoing = store::list_provenance_from(db, chunk_id)?;
    let incoming = store::list_provenance_to(db, chunk_id)?;
    if outgoing.is_empty() && incoming.is_empty() {
        println!("     citation chain: none recorded");
        return Ok(());
    }
    for edge in outgoing.iter().chain(incoming.iter()) {
        println!(
            "     citation chain: {} --{:?}--> {}",
            edge.from_artifact_id, edge.edge_type, edge.to_artifact_id
        );
    }
    Ok(())
}
