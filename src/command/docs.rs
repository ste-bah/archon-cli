use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use cozo::DbInstance;

use archon_docs::answer;
use archon_docs::ingest;
use archon_docs::inspect;
use archon_docs::retrieval;
use archon_docs::retrieval_image;
use archon_docs::store;
use archon_docs::vlm::factory::{self as vlm_factory, VlmProviderInitStatus};

use crate::cli_args::DocsAction;

fn docs_db_path() -> PathBuf {
    crate::command::store_paths::evidence_db_path(&["ARCHON_DOCS_DB_PATH"])
}

pub(crate) fn open_db() -> Result<Arc<DbInstance>> {
    archon_docs::acquire_docs_db(docs_db_path())
}

pub async fn handle_docs_command(action: DocsAction) -> Result<()> {
    match action {
        DocsAction::Ingest { path, yes, jobs } => handle_ingest(&path, yes, jobs.as_deref()).await,
        DocsAction::Reprocess {
            target,
            defer_index,
        } => crate::command::docs_reprocess::handle_reprocess(&target, defer_index).await,
        DocsAction::Delete { target, yes } => {
            crate::command::docs_delete::handle_delete(&target, yes)
        }
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
        DocsAction::VerifyQuote {
            quote,
            doc,
            limit,
            json,
        } => handle_verify_quote(&quote, doc.as_deref(), limit, json).await,
        DocsAction::VerifyIntegrity { doc, json } => {
            handle_verify_integrity(doc.as_deref(), json).await
        }
    }
}

/// V-4: locate a quote in the corpus and report its source document, page(s), and bbox(es) so the
/// citation can be verified against the source (and a PDF highlight drawn).
async fn handle_verify_quote(
    quote: &str,
    doc: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<()> {
    use archon_docs::quote_verify::{self, MatchKind};

    let db = open_db()?;
    let locations = match doc {
        Some(d) => quote_verify::find_fragment_bboxes(&db, d, quote)?
            .into_iter()
            .collect::<Vec<_>>(),
        None => quote_verify::locate_quote(&db, quote, limit.max(1))?,
    };

    if json {
        let out = serde_json::json!({
            "quote": quote,
            "found": !locations.is_empty(),
            "locations": locations.iter().map(|l| serde_json::json!({
                "document_id": l.document_id,
                "source_path": l.source_path,
                "page_start": l.page_start,
                "page_end": l.page_end,
                "match_kind": if l.match_kind == MatchKind::Exact { "exact" } else { "fuzzy" },
                "similarity": l.similarity,
                "source_span": l.source_span,
                "fragments": l.fragments.iter().map(|f| serde_json::json!({
                    "chunk_id": f.chunk_id,
                    "page": f.page,
                    "bbox": f.bbox,
                    "coord_space": f.coord_space,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let shown: String = quote.chars().take(70).collect();
    println!(
        "\nQUOTE VERIFICATION — \"{}{}\"",
        shown,
        if quote.chars().count() > 70 {
            "…"
        } else {
            ""
        }
    );
    if locations.is_empty() {
        println!("  ✗ NOT FOUND — no corpus document contains this quote (above the match floor).");
        println!(
            "    (A real quote not found here may be from a source not in the corpus, or misquoted.)"
        );
        return Ok(());
    }
    for (i, l) in locations.iter().enumerate() {
        let (mark, label) = match l.match_kind {
            MatchKind::Exact => ("✓", "EXACT MATCH".to_string()),
            MatchKind::Fuzzy => (
                "~",
                format!(
                    "FUZZY {:.0}% — {}",
                    l.similarity * 100.0,
                    if l.similarity >= 0.90 {
                        "near-verbatim; minor differences"
                    } else {
                        "REVIEW: source differs from the quote"
                    }
                ),
            ),
        };
        let name = std::path::Path::new(&l.source_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| l.document_id.clone());
        println!("\n  [{}] {mark} {label}", i + 1);
        println!("      source: {name}");
        let pages = if l.page_start == l.page_end {
            format!("p.{}", l.page_start)
        } else {
            format!("pp.{}–{}", l.page_start, l.page_end)
        };
        let boxes: Vec<String> = l
            .fragments
            .iter()
            .map(|f| match f.bbox {
                Some(b) => format!(
                    "p{} [{:.0},{:.0},{:.0},{:.0}]",
                    f.page, b[0], b[1], b[2], b[3]
                ),
                None => format!("p{} (no bbox)", f.page),
            })
            .collect();
        println!("      {pages}   bbox: {}", boxes.join(" · "));
        // Exact spans are short; a fuzzy span is the whole matched chunk — truncate for display.
        let span = l.source_span.trim();
        let shown_span: String = if span.chars().count() > 320 {
            format!("{}…", span.chars().take(320).collect::<String>())
        } else {
            span.to_string()
        };
        println!("      source text: \"{shown_span}\"");
    }
    Ok(())
}

/// Verify chunk-integrity (`chunks_root`) for one or all documents. Recomputes the Merkle-style
/// root over the document's per-chunk commit hashes and compares it to the sealed
/// `extract_text_spatial` provenance record — any drift (a chunk added, removed, or edited after
/// ingestion) flips the root and fails the check. A document with no sealed record (ingested before
/// integrity sealing existed) is reported separately, not as a failure.
async fn handle_verify_integrity(doc: Option<&str>, json: bool) -> Result<()> {
    use archon_docs::{provenance_chunks, store};

    let db = open_db()?;
    let sources = store::list_doc_sources(&db)?;
    let targets: Vec<_> = match doc {
        Some(d) => sources.into_iter().filter(|s| s.document_id == d).collect(),
        None => sources,
    };
    if targets.is_empty() {
        match doc {
            Some(d) => println!("No such document: {d}"),
            None => println!("No documents ingested."),
        }
        return Ok(());
    }

    struct Report {
        document_id: String,
        source: String,
        status: &'static str, // "pass" | "fail" | "no-record"
        chunks: usize,
        record_id: Option<String>,
    }

    let mut reports = Vec::new();
    for s in &targets {
        // The chunks_root record hangs off the ocr_text artifact (see persist_chunk_integrity):
        // record_id = prov-extract-<ocr_artifact_id>. A doc may have more than one ocr_text
        // artifact (text + image union under C3); the authoritative root is the one sealed over
        // the current full chunk set, so PASS if ANY sealed record verifies.
        let sealed: Vec<_> = store::list_artifacts_for_doc(&db, &s.document_id)?
            .into_iter()
            .filter(|a| a.artifact_type == "ocr_text" && !a.provenance_record_id.is_empty())
            .collect();
        let n_chunks = store::get_doc_commit_hashes(&db, &s.document_id)?.len();
        let name = std::path::Path::new(&s.source_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| s.document_id.clone());

        if sealed.is_empty() {
            reports.push(Report {
                document_id: s.document_id.clone(),
                source: name,
                status: "no-record",
                chunks: n_chunks,
                record_id: None,
            });
            continue;
        }
        let mut matched: Option<String> = None;
        for a in &sealed {
            if provenance_chunks::verify_chunks_root(&db, &s.document_id, &a.provenance_record_id)?
            {
                matched = Some(a.provenance_record_id.clone());
                break;
            }
        }
        let status = if matched.is_some() { "pass" } else { "fail" };
        let record_id = matched.or_else(|| sealed.first().map(|a| a.provenance_record_id.clone()));
        reports.push(Report {
            document_id: s.document_id.clone(),
            source: name,
            status,
            chunks: n_chunks,
            record_id,
        });
    }

    if json {
        let out = serde_json::json!({
            "all_pass": reports.iter().all(|r| r.status == "pass"),
            "documents": reports.iter().map(|r| serde_json::json!({
                "document_id": r.document_id,
                "source": r.source,
                "status": r.status,
                "chunks": r.chunks,
                "record_id": r.record_id,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("\nCHUNK-INTEGRITY VERIFICATION (chunks_root)");
    let (mut pass, mut fail, mut none) = (0usize, 0usize, 0usize);
    for r in &reports {
        let (mark, label) = match r.status {
            "pass" => {
                pass += 1;
                (
                    "✓",
                    "INTACT — recomputed root matches the sealed record".to_string(),
                )
            }
            "fail" => {
                fail += 1;
                (
                    "✗",
                    "MISMATCH — chunks changed since sealing (integrity violation)".to_string(),
                )
            }
            _ => {
                none += 1;
                (
                    "!",
                    "NO INTEGRITY RECORD — ingested before chunks_root sealing".to_string(),
                )
            }
        };
        println!("\n  {mark} {label}");
        println!("      source: {}", r.source);
        println!("      doc:    {}", r.document_id);
        println!("      chunks: {}", r.chunks);
        if let Some(rec) = &r.record_id {
            println!("      record: {rec}");
        }
    }
    println!(
        "\n  Summary: {pass} intact · {fail} mismatch · {none} no-record  ({} document(s))",
        reports.len()
    );
    Ok(())
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
        println!(
            "     active verdict; the '{}' detector is currently in force.",
            plan.detector
        );
    }
    if plan.is_scanned_book && plan.has_text_layer {
        println!("  Classification: SCANNED BOOK (text layer present)");
        println!(
            "    -> image enrichment SKIPPED: {} full-page scan(s) will NOT be OCR'd/VLM'd",
            plan.embedded_images
        );
        println!("       (Marker / the text layer already owns the pages).");
        println!("  !! If this is actually a born-digital doc with REAL figures, abort and review");
        println!("     -- the scan detector may have misfired.");
    } else if plan.is_scanned_book {
        // Scanned but NO text layer → the page scans are the ONLY content, so they get OCR'd.
        println!("  Classification: IMAGE-ONLY SCAN (no text layer)");
        println!(
            "    -> {} full-page scan(s) WILL be OCR'd for content (Marker if configured, else",
            plan.embedded_images
        );
        println!("       image OCR); VLM skipped -- these are page reproductions, not figures.");
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

/// Resolve `--jobs` to a concrete image-enrichment worker count.
///
/// - An explicit integer wins outright (clamped to the enrichment engine's 1..=16), no prompt.
/// - `auto` probes the accelerators and derives a recommendation from FREE VRAM (free, not
///   card size — co-tenancy can starve a big card). Interactive sessions get to confirm or
///   override the recommendation; `--yes` (or a non-tty stdin) takes it unattended.
fn resolve_jobs(jobs: &str, yes: bool) -> Result<u32> {
    if !jobs.eq_ignore_ascii_case("auto") {
        // Explicit numeric value: reject out-of-range rather than silently coercing, so `--jobs 0`
        // or `--jobs 99` surfaces the user's mistake instead of quietly becoming 1 or 16. (The
        // 1..=16 clamp is kept only for the auto-derived value, which is machine-generated.)
        let n: u32 = jobs.parse().map_err(|_| {
            anyhow::anyhow!("--jobs must be \"auto\" or an integer 1..=16 (got: {jobs})")
        })?;
        if !(1..=16).contains(&n) {
            anyhow::bail!("--jobs must be \"auto\" or an integer 1..=16 (got: {n})");
        }
        return Ok(n);
    }
    let report = archon_accel::detect();
    let recommended = archon_docs::auto_image_workers(&report);
    match report.best_gpu() {
        Some(gpu) => println!(
            "GPU: {} — {} MB free → recommended {} parallel VLM workers (1 = serial).{}",
            gpu.name,
            gpu.free_mb,
            recommended,
            if report.unified_memory {
                " [unified memory: capped at 2]"
            } else {
                ""
            }
        ),
        None => println!("No GPU detected → recommended 1 VLM worker (serial)."),
    }
    if yes || !std::io::stdin().is_terminal() {
        // Unattended (--yes or piped stdin): take the probe's answer, but say so — the run
        // log should show why N workers were chosen.
        println!("Using {recommended} image-enrichment worker(s) (--jobs auto).");
        return Ok(recommended);
    }
    use std::io::Write;
    eprint!("Image-enrichment workers? [{recommended}]: ");
    std::io::stderr().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(recommended);
    }
    let n: u32 = trimmed.parse().map_err(|_| {
        anyhow::anyhow!(
            "expected an empty line (accept {recommended}) or a number 1..=16 (got: {trimmed})"
        )
    })?;
    Ok(n.clamp(1, 16))
}

async fn handle_ingest(path_str: &str, yes: bool, jobs: Option<&str>) -> Result<()> {
    let result = handle_ingest_inner(path_str, yes, jobs).await;
    archon_docs::vlm::clear_provider_blocking_safe().await;
    result
}

async fn handle_ingest_inner(path_str: &str, yes: bool, jobs: Option<&str>) -> Result<()> {
    // Validate the path FIRST — before the (possibly interactive) `--jobs auto` probe/prompt —
    // so a typo'd path errors immediately instead of after the user answers a worker-count prompt.
    let path = PathBuf::from(path_str);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path_str);
    }

    let db = open_db()?;
    let _ = crate::command::docs_embedding::init_embedding(&db);
    let mut policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();
    // `--jobs` overrides the policy's image-enrichment worker count at runtime, resolved
    // BEFORE any ingest work so the probe/prompt happens once up front. When the flag is
    // absent the policy value stands untouched — the zero-regression default (a policy.toml
    // that sets its own worker count keeps working exactly as before this flag existed).
    if let Some(jobs) = jobs {
        let workers = resolve_jobs(jobs, yes)?;
        policy.docs.pdf.image_enrichment_workers = workers;
        if workers > 1 {
            // The workers fan out over one ollama server; unless it accepts parallel
            // requests they just queue there and the run is serial anyway.
            println!(
                "Note: {workers} parallel VLM workers need the ollama server to accept \
                 parallel requests (OLLAMA_NUM_PARALLEL >= {workers}); otherwise they queue serially."
            );
        }
    }
    // Preflight the persistent Marker server BEFORE any ingest work: a set `marker_url` means the
    // run expects real bboxes, so a wrong/forgotten URL or a still-loading/dead server must hard-
    // stop here rather than silently degrade the whole corpus to bbox-less text. Tolerates a just-
    // started server by polling /health (it doesn't bind its port until models finish loading).
    if let Some(marker_url) = policy.docs.pdf.marker_url.clone() {
        println!("Marker server: preflighting {marker_url}/health (waiting for warm models)…");
        archon_docs::marker_source::preflight_health(
            &marker_url,
            std::time::Duration::from_secs(archon_docs::marker_source::HEALTH_MAX_WAIT_SECS),
            std::time::Duration::from_secs(archon_docs::marker_source::HEALTH_POLL_INTERVAL_SECS),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("Marker server: ready (models resident).");
    }

    let vlm_report = vlm_factory::configure_registered_provider_blocking_safe(&policy).await;

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
        // COORD integrity summary: for the re-ingest we must see at a glance that no PDF silently
        // fell back to bbox-less text. Printed whenever any PDF carried a coordinate verdict.
        if result.pdf_coord_marker > 0 || result.pdf_coord_native > 0 || result.pdf_coord_none > 0 {
            println!(
                "PDF coord: {} doc(s) COORD_MARKER, {} COORD_PDF_NATIVE (real bboxes), \
                 {} COORD_NONE (text fallback)",
                result.pdf_coord_marker, result.pdf_coord_native, result.pdf_coord_none
            );
            if result.pdf_coord_none > 0 {
                println!(
                    "  WARNING: {} PDF(s) landed in COORD_NONE — those chunks carry NO bboxes.",
                    result.pdf_coord_none
                );
            }
        }
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
                if let Some(coord) = r.pdf_coord {
                    println!("Marker coord: {coord}");
                }
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
