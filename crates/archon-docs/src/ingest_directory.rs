use std::path::Path;

use anyhow::Result;
use cozo::DbInstance;

use crate::errors::DocsError;
use crate::ingest::{
    IngestFileResult, detect_media_type, ingest_file_with_policy, is_image_media_type,
    is_supported_media_type,
};
use crate::schema::ensure_doc_schema;

/// Result of a directory ingest operation.
#[derive(Clone, Debug, Default)]
pub struct IngestResult {
    pub sources_registered: usize,
    pub sources_skipped_duplicate: usize,
    pub sources_failed: usize,
    pub images_skipped: usize,
    pub image_ocr_completed: usize,
    pub vlm_descriptions: usize,
    pub pdf_embedded_images_extracted: usize,
    pub pdf_embedded_images_skipped_filter: usize,
    pub pdf_image_ocr_runs: usize,
    pub pdf_image_vlm_failures: usize,
    pub pdf_image_ocr_failures: usize,
    pub pdf_pages_rendered: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// Ingest a directory: walk all files, ingest supported types, skip duplicates.
pub async fn ingest_directory(db: &DbInstance, dir: &Path) -> Result<IngestResult> {
    ingest_directory_with_policy(db, dir, &archon_policy::EffectivePolicy::default()).await
}

/// Ingest a directory with an explicit policy for optional multimodal steps.
pub async fn ingest_directory_with_policy(
    db: &DbInstance,
    dir: &Path,
    policy: &archon_policy::EffectivePolicy,
) -> Result<IngestResult> {
    ensure_doc_schema(db).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let mut result = IngestResult::default();

    for entry in walkdir::WalkDir::new(dir).into_iter().filter_entry(|e| {
        // Never skip the root directory; skip hidden subdirectories.
        e.depth() == 0
            || !e
                .file_name()
                .to_str()
                .map(|s| s.starts_with('.'))
                .unwrap_or(false)
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(e.to_string());
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let media_type = detect_media_type(path);

        if !is_supported_media_type(media_type) {
            continue;
        }

        match ingest_file_with_policy(db, path, policy).await {
            Ok(r) if r.pipeline_failed => {
                result.sources_failed += 1;
                result.vlm_descriptions += r.vlm_descriptions;
                add_pdf_counts(&mut result, &r);
                result.warnings.extend(r.warnings);
                result.errors.push(format!(
                    "{}: pipeline failed for {}",
                    path.display(),
                    r.document_id
                ));
            }
            Ok(r) if r.was_new && r.ocr_skipped => {
                result.sources_registered += 1;
                result.images_skipped += 1;
                result.vlm_descriptions += r.vlm_descriptions;
                add_pdf_counts(&mut result, &r);
                result.warnings.extend(r.warnings);
            }
            Ok(r) if r.was_new => {
                result.sources_registered += 1;
                if is_image_media_type(media_type) {
                    result.image_ocr_completed += 1;
                }
                result.vlm_descriptions += r.vlm_descriptions;
                add_pdf_counts(&mut result, &r);
                result.warnings.extend(r.warnings);
            }
            Ok(_) => {
                result.sources_skipped_duplicate += 1;
            }
            Err(e) => {
                result.sources_failed += 1;
                result.errors.push(format!("{}: {}", path.display(), e));
            }
        }
    }

    Ok(result)
}

fn add_pdf_counts(result: &mut IngestResult, file: &IngestFileResult) {
    result.pdf_embedded_images_extracted += file.pdf_embedded_images_extracted;
    result.pdf_embedded_images_skipped_filter += file.pdf_embedded_images_skipped_filter;
    result.pdf_image_ocr_runs += file.pdf_image_ocr_runs;
    result.pdf_image_vlm_failures += file.pdf_image_vlm_failures;
    result.pdf_image_ocr_failures += file.pdf_image_ocr_failures;
    result.pdf_pages_rendered += file.pdf_pages_rendered;
}
