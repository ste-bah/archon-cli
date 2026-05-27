use std::path::{Path, PathBuf};

use anyhow::Result;
use archon_docs::models::SourceDocument;
use archon_docs::schema::ensure_doc_schema;
use archon_docs::vlm::factory::{
    self as vlm_factory, VlmProviderInitReport, VlmProviderInitStatus,
};
use cozo::DbInstance;

pub(crate) fn open_docs_db() -> Result<DbInstance> {
    let db_path = crate::command::store_paths::evidence_db_path(&["ARCHON_DOCS_DB_PATH"]);
    let db = crate::command::store_paths::open_sqlite_db(&db_path, "document")?;
    ensure_doc_schema(&db)?;
    Ok(db)
}

pub(crate) fn load_policy() -> archon_policy::EffectivePolicy {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default()
}

pub(crate) async fn handle_reprocess(target: &str, defer_index: bool) -> Result<()> {
    let db = open_docs_db()?;
    if !defer_index {
        let _ = archon_docs::embed::init_default_provider();
    }
    let policy = load_policy();
    let vlm_report = vlm_factory::configure_registered_provider(&policy);
    let docs = resolve_target_documents(&db, target)?;
    reprocess_documents(
        &db,
        &policy,
        &vlm_report,
        &docs,
        "document evidence",
        !defer_index,
    )
    .await
}

pub(crate) async fn reprocess_documents(
    db: &DbInstance,
    policy: &archon_policy::EffectivePolicy,
    vlm_report: &VlmProviderInitReport,
    docs: &[SourceDocument],
    label: &str,
    index_after: bool,
) -> Result<()> {
    if docs.is_empty() {
        anyhow::bail!("no documents matched for reprocess");
    }
    println!("Reprocessing {} {label} document(s)", docs.len());
    print_vlm_init_status(vlm_report);

    let mut failed = 0usize;
    for doc in docs {
        println!("Reprocessing: {}  {}", doc.document_id, doc.source_path);
        match archon_docs::reprocess::reprocess_document_with_policy(db, &doc.document_id, policy)
            .await
        {
            Ok(result) => {
                if result.ingest.pipeline_failed {
                    failed += 1;
                }
                print_result(db, &result, vlm_report)?;
            }
            Err(err) => {
                failed += 1;
                println!("Failed: {}  {}", doc.document_id, err);
            }
        }
    }

    if index_after {
        crate::command::evidence_index::index_pending_evidence(db, label);
    } else {
        println!(
            "Deferred semantic indexing; run `archon docs index` when reprocessing is complete."
        );
    }
    if failed > 0 {
        anyhow::bail!("reprocess completed with {failed} failed document(s)");
    }
    Ok(())
}

fn resolve_target_documents(db: &DbInstance, target: &str) -> Result<Vec<SourceDocument>> {
    if let Some(doc) = archon_docs::store::get_doc_source(db, target)? {
        return Ok(vec![doc]);
    }

    let docs = archon_docs::store::list_doc_sources(db)?
        .into_iter()
        .filter(|doc| source_matches(target, &doc.source_path))
        .collect::<Vec<_>>();
    if docs.is_empty() {
        anyhow::bail!("no documents matched target `{target}`");
    }
    Ok(docs)
}

fn source_matches(target: &str, source: &str) -> bool {
    let target = target.trim();
    source == target
        || source.starts_with(path_prefix(target).as_str())
        || canonical_prefix_match(Path::new(target), Path::new(source))
}

fn path_prefix(path: &str) -> String {
    if path.ends_with(std::path::MAIN_SEPARATOR) {
        path.to_string()
    } else {
        format!("{path}{}", std::path::MAIN_SEPARATOR)
    }
}

fn canonical_prefix_match(target: &Path, source: &Path) -> bool {
    let Ok(target) = canonicalize_maybe_relative(target) else {
        return false;
    };
    let Ok(source) = canonicalize_maybe_relative(source) else {
        return false;
    };
    source.starts_with(target)
}

fn canonicalize_maybe_relative(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        path.canonicalize()
    } else {
        std::env::current_dir()?.join(path).canonicalize()
    }
}

fn print_result(
    db: &DbInstance,
    result: &archon_docs::reprocess::ReprocessDocumentResult,
    vlm_report: &VlmProviderInitReport,
) -> Result<()> {
    let chunks = archon_docs::store::list_chunks_for_doc(db, &result.ingest.document_id)?.len();
    println!("Reprocessed: {}", result.ingest.document_id);
    println!("Source: {}", result.source_path);
    println!(
        "Cleared: {} chunk(s), {} page(s), {} artifact(s), {} image description(s)",
        result.cleared.chunks,
        result.cleared.pages,
        result.cleared.artifacts,
        result.cleared.image_descriptions
    );
    println!("Chunks: {chunks}");
    if result.ingest.vlm_descriptions > 0 {
        println!(
            "VLM descriptions: {} via {}/{}",
            result.ingest.vlm_descriptions, vlm_report.provider, vlm_report.model
        );
    }
    print_pdf_metrics(&result.ingest);
    for warning in &result.ingest.warnings {
        println!("Warning: {warning}");
    }
    Ok(())
}

fn print_pdf_metrics(result: &archon_docs::ingest::IngestFileResult) {
    if result.pdf_embedded_images_extracted == 0 && result.pdf_pages_rendered == 0 {
        return;
    }
    println!(
        "PDF images: {} embedded extracted, {} skipped by filter, {} rendered page(s)",
        result.pdf_embedded_images_extracted,
        result.pdf_embedded_images_skipped_filter,
        result.pdf_pages_rendered
    );
    println!(
        "PDF image OCR: {} run(s), {} failure(s); VLM failures: {}",
        result.pdf_image_ocr_runs, result.pdf_image_ocr_failures, result.pdf_image_vlm_failures
    );
}

fn print_vlm_init_status(report: &VlmProviderInitReport) {
    match report.status {
        VlmProviderInitStatus::Registered => {
            println!("VLM provider: {}/{}", report.provider, report.model);
        }
        VlmProviderInitStatus::Skipped => {
            println!("Warning: VLM provider unavailable: {}", report.message);
        }
        VlmProviderInitStatus::Disabled => {}
    }
}
