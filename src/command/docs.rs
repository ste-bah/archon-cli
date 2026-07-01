use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::Result;
use cozo::DbInstance;

use archon_docs::answer;
use archon_docs::ingest;
use archon_docs::inspect;
use archon_docs::retrieval;
use archon_docs::retrieval_image;
use archon_docs::schema::ensure_doc_schema;
use archon_docs::store;
use archon_docs::vlm::factory::{self as vlm_factory, VlmProviderInitStatus};

use crate::cli_args::DocsAction;

fn docs_db_path() -> PathBuf {
    crate::command::store_paths::evidence_db_path(&["ARCHON_DOCS_DB_PATH"])
}

pub(crate) fn open_db() -> Result<DbInstance> {
    let db_path = docs_db_path();
    archon_docs::configure_cozo_write_lock_for_db(&db_path);
    let db = crate::command::store_paths::open_sqlite_db(&db_path, "document")?;
    ensure_doc_schema(&db)?;
    Ok(db)
}

pub async fn handle_docs_command(action: DocsAction) -> Result<()> {
    match action {
        DocsAction::Ingest { path, yes } => handle_ingest(&path, yes).await,
        DocsAction::Reprocess {
            target,
            defer_index,
        } => crate::command::docs_reprocess::handle_reprocess(&target, defer_index).await,
        DocsAction::List => handle_list().await,
        DocsAction::Show { document_id } => handle_show(&document_id).await,
        DocsAction::Status => crate::command::docs_status::handle_status(open_db()?).await,
        DocsAction::Chunks { document_id } => handle_chunks(&document_id).await,
        DocsAction::Inspect { document_id } => handle_inspect(&document_id).await,
        DocsAction::Search { query, mode, debug } => handle_search(&query, &mode, debug).await,
        DocsAction::SearchImages { query, limit } => handle_search_images(&query, limit).await,
        DocsAction::Answer { query } => handle_answer(&query).await,
        DocsAction::Provenance { chunk_or_answer_id } => {
            handle_provenance(&chunk_or_answer_id).await
        }
        DocsAction::Index {
            all,
            document,
            batch_size,
            limit,
        } => {
            crate::command::docs_index::handle_index(all, document, batch_size, limit, open_db()?)
                .await
        }
        DocsAction::IndexStatus => crate::command::docs_index::handle_index_status(open_db()?),
        DocsAction::IndexRetryFailed { limit } => {
            crate::command::docs_index::handle_index_retry_failed(open_db()?, limit)
        }
        DocsAction::IndexPause { job_id } => {
            crate::command::docs_index::handle_index_pause(open_db()?, &job_id)
        }
        DocsAction::IndexResume { job_id } => {
            crate::command::docs_index::handle_index_resume(open_db()?, &job_id)
        }
        DocsAction::IndexCancel { job_id } => {
            crate::command::docs_index::handle_index_cancel(open_db()?, &job_id)
        }
        DocsAction::IndexDaemon { action } => {
            crate::command::docs_index_daemon::handle_index_daemon(action).await
        }
        DocsAction::VectorStatus => crate::command::docs_vector::handle_vector_status(open_db()?),
        DocsAction::VectorMigrate {
            limit,
            batch_size,
            after,
        } => {
            crate::command::docs_vector::handle_vector_migrate(open_db()?, limit, batch_size, after)
        }
        DocsAction::VectorCompact {
            provider,
            dimension,
            limit,
        } => crate::command::docs_vector::handle_vector_compact(
            open_db()?,
            provider,
            dimension,
            limit,
        ),
        DocsAction::ModelStatus => {
            crate::command::docs_embedding::handle_model_status(open_db()?).await
        }
    }
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

/// LOUD pre-ingest report of how the image-enrichment classifier will treat this PDF, so a
/// misclassification is visible (and abortable) before any OCR/VLM runs.
fn print_enrichment_plan(
    path: &Path,
    plan: &archon_docs::pdf::EnrichmentClassification,
    policy: &archon_policy::EffectivePolicy,
) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let vlm_on = policy.docs.vlm.enabled && policy.docs.vlm.provider != "disabled";
    println!("\n========================================================================");
    println!("  ENRICHMENT PLAN — {name}");
    println!("========================================================================");
    println!(
        "  Pages: {}   Embedded images: {}   Page-scans detected: {} (active detector: {})",
        plan.page_count, plan.embedded_images, plan.page_scans, plan.detector
    );
    // A/B: show both detector verdicts so a divergence is visible before committing.
    let verdict = |scanned: bool| if scanned { "SCANNED" } else { "born-digital" };
    let coverage_line = match (plan.coverage_scanned, plan.coverage_max) {
        (Some(scanned), Some(max)) => {
            format!(
                "{} ({} page-scan(s), peak coverage {:.0}%){}",
                verdict(scanned),
                plan.coverage_page_scans.unwrap_or(0),
                max * 100.0,
                if plan.coverage_low_confidence {
                    " — LOW CONFIDENCE (some images deferred to aspect; review)"
                } else {
                    ""
                }
            )
        }
        _ => "unavailable (no page dimensions readable)".to_string(),
    };
    println!(
        "  Detectors:  aspect = {} ({} page-scan(s))    coverage = {}",
        verdict(plan.aspect_scanned),
        plan.aspect_page_scans,
        coverage_line
    );
    if plan.divergent {
        println!(
            "  !! DETECTORS DISAGREE — aspect says {}, coverage says {}. Review before trusting the",
            verdict(plan.aspect_scanned),
            plan.coverage_scanned.map(verdict).unwrap_or("?")
        );
        println!("     active verdict; the '{}' detector is currently in force.", plan.detector);
    }
    if plan.is_scanned_book {
        println!("  Classification: SCANNED BOOK");
        println!(
            "    -> image enrichment SKIPPED: {} full-page scan(s) will NOT be OCR'd/VLM'd",
            plan.embedded_images
        );
        println!("       (Marker OCRs the pages via the text layer).");
        println!("  !! If this is actually a born-digital doc with REAL figures, abort and review");
        println!("     -- the scan detector may have misfired.");
    } else if plan.embedded_images == 0 {
        println!("  Classification: no embedded images -- nothing to enrich.");
    } else {
        println!("  Classification: BORN-DIGITAL");
        println!(
            "    -> {} figure(s) WILL be ENRICHED: image OCR{}",
            plan.will_enrich,
            if vlm_on {
                " + VLM description"
            } else {
                " (VLM off)"
            }
        );
        println!(
            "  !! If this is actually a SCANNED book, abort -- enriching page-scans wastes the"
        );
        println!("     VLM and duplicates the page text.");
    }
    println!("========================================================================");
}

fn confirm_proceed() -> Result<bool> {
    use std::io::Write;
    eprint!("Proceed with this enrichment plan? [y/N] ");
    std::io::stderr().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

async fn handle_ingest(path_str: &str, yes: bool) -> Result<()> {
    let result = handle_ingest_inner(path_str, yes).await;
    archon_docs::vlm::clear_provider_blocking_safe().await;
    result
}

async fn handle_ingest_inner(path_str: &str, yes: bool) -> Result<()> {
    let db = open_db()?;
    let _ = crate::command::docs_embedding::init_embedding(&db);
    let policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();
    let vlm_report = vlm_factory::configure_registered_provider_blocking_safe(&policy).await;
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
            crate::command::evidence_index::index_pending_evidence(&db, "video evidence");
            return Ok(());
        }
        // Pre-ingest enrichment classification: LOUD report + confirm, so a mis-detected doc (a
        // born-digital paper wrongly flagged as scanned, or a scanned book wrongly enriched) is
        // caught BEFORE any OCR/VLM. Skipped for non-PDFs, with --yes, or when non-interactive.
        if is_pdf_path(&path) {
            let plan = archon_docs::pdf::classify_pdf_enrichment(&path, &policy.docs.pdf);
            print_enrichment_plan(&path, &plan, &policy);
            if !yes && std::io::stdin().is_terminal() && !confirm_proceed()? {
                println!("Aborted — no changes made.");
                return Ok(());
            }
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

// ── Phase 2: retrieval, answer, provenance ──────────────

async fn handle_search_images(query: &str, limit: usize) -> Result<()> {
    let db = open_db()?;
    match retrieval_image::search_images(&db, query, limit) {
        Ok(results) => {
            if results.is_empty() {
                println!(
                    "No image results. Ingest standalone images (.jpg/.png) first, and ensure a \
                     multimodal (CLIP) embedding provider is configured."
                );
                return Ok(());
            }
            println!(
                "Found {} image result(s) for \"{}\":\n",
                results.len(),
                query
            );
            for (i, r) in results.iter().enumerate() {
                println!("  {}. score={:.3}  {}", i + 1, r.score, r.source_path);
                println!(
                    "     page {} · doc {} · distance {:.4}",
                    r.page_number, r.document_id, r.distance
                );
            }
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

async fn handle_search(query: &str, mode: &str, debug: bool) -> Result<()> {
    let db = open_db()?;
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
