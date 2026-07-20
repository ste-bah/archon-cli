use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{ImageDescription, PdfIngestMetrics};

pub fn insert_image_description(db: &DbInstance, desc: &ImageDescription) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(desc.artifact_id.as_str()));
    params.insert("did".into(), DataValue::from(desc.document_id.as_str()));
    params.insert("page".into(), DataValue::from(desc.page_number as i64));
    params.insert("provider".into(), DataValue::from(desc.provider.as_str()));
    params.insert("model".into(), DataValue::from(desc.model.as_str()));
    params.insert("desc".into(), DataValue::from(desc.description.as_str()));
    params.insert("created".into(), DataValue::from(desc.created_at.as_str()));
    params.insert("cost".into(), DataValue::from(desc.cost_usd));

    crate::cozo_retry::run_script_guarded(
        db,
        "?[artifact_id, document_id, page_number, provider, model, description, created_at, cost_usd] \
         <- [[$aid, $did, $page, $provider, $model, $desc, $created, $cost]] \
         :put doc_image_descriptions { artifact_id => document_id, page_number, provider, model, description, created_at, cost_usd }",
        params,
        ScriptMutability::Mutable,
        "insert doc_image_descriptions",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_image_descriptions failed: {e}"))?;
    Ok(())
}

pub fn list_image_descriptions_for_doc(
    db: &DbInstance,
    document_id: &str,
) -> Result<Vec<ImageDescription>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = db
        .run_script(
            "?[artifact_id, document_id, page_number, provider, model, description, created_at, cost_usd] \
             := *doc_image_descriptions{artifact_id, document_id, page_number, provider, model, description, created_at, cost_usd}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list doc_image_descriptions failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| ImageDescription {
            artifact_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            page_number: row[2].get_int().unwrap_or(0) as u32,
            provider: row[3].get_str().unwrap_or("").to_string(),
            model: row[4].get_str().unwrap_or("").to_string(),
            description: row[5].get_str().unwrap_or("").to_string(),
            created_at: row[6].get_str().unwrap_or("").to_string(),
            cost_usd: row[7].get_float().unwrap_or(0.0),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// PdfIngestMetrics
// ---------------------------------------------------------------------------

pub fn upsert_pdf_metrics(db: &DbInstance, metrics: &PdfIngestMetrics) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(metrics.document_id.as_str()));
    params.insert(
        "extracted".into(),
        DataValue::from(metrics.embedded_images_extracted as i64),
    );
    params.insert(
        "skipped".into(),
        DataValue::from(metrics.embedded_images_skipped_filter as i64),
    );
    params.insert(
        "ocr_runs".into(),
        DataValue::from(metrics.image_ocr_runs as i64),
    );
    params.insert(
        "ocr_failures".into(),
        DataValue::from(metrics.image_ocr_failures as i64),
    );
    params.insert(
        "vlm_descriptions".into(),
        DataValue::from(metrics.image_vlm_descriptions as i64),
    );
    params.insert(
        "vlm_failures".into(),
        DataValue::from(metrics.image_vlm_failures as i64),
    );
    params.insert(
        "rendered".into(),
        DataValue::from(metrics.pages_rendered as i64),
    );

    crate::cozo_retry::run_script_guarded(
        db,
        "?[document_id, embedded_images_extracted, embedded_images_skipped_filter, image_ocr_runs, image_ocr_failures, image_vlm_descriptions, image_vlm_failures, pages_rendered] \
         <- [[$did, $extracted, $skipped, $ocr_runs, $ocr_failures, $vlm_descriptions, $vlm_failures, $rendered]] \
         :put doc_pdf_metrics { document_id => embedded_images_extracted, embedded_images_skipped_filter, image_ocr_runs, image_ocr_failures, image_vlm_descriptions, image_vlm_failures, pages_rendered }",
        params,
        ScriptMutability::Mutable,
        "upsert doc_pdf_metrics",
    )
    .map_err(|e| anyhow::anyhow!("upsert doc_pdf_metrics failed: {e}"))?;
    Ok(())
}

pub fn get_pdf_metrics(db: &DbInstance, document_id: &str) -> Result<Option<PdfIngestMetrics>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = db
        .run_script(
            "?[document_id, embedded_images_extracted, embedded_images_skipped_filter, image_ocr_runs, image_ocr_failures, image_vlm_descriptions, image_vlm_failures, pages_rendered] \
             := *doc_pdf_metrics{document_id, embedded_images_extracted, embedded_images_skipped_filter, image_ocr_runs, image_ocr_failures, image_vlm_descriptions, image_vlm_failures, pages_rendered}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get doc_pdf_metrics failed: {e}"))?;
    if result.rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(pdf_metrics_from_row(&result.rows[0])))
}

pub fn list_pdf_metrics(db: &DbInstance) -> Result<Vec<PdfIngestMetrics>> {
    let result = db
        .run_script(
            "?[document_id, embedded_images_extracted, embedded_images_skipped_filter, image_ocr_runs, image_ocr_failures, image_vlm_descriptions, image_vlm_failures, pages_rendered] \
             := *doc_pdf_metrics{document_id, embedded_images_extracted, embedded_images_skipped_filter, image_ocr_runs, image_ocr_failures, image_vlm_descriptions, image_vlm_failures, pages_rendered}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list doc_pdf_metrics failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| pdf_metrics_from_row(row))
        .collect())
}

fn pdf_metrics_from_row(row: &[DataValue]) -> PdfIngestMetrics {
    PdfIngestMetrics {
        document_id: row[0].get_str().unwrap_or("").to_string(),
        embedded_images_extracted: row[1].get_int().unwrap_or(0) as u32,
        embedded_images_skipped_filter: row[2].get_int().unwrap_or(0) as u32,
        image_ocr_runs: row[3].get_int().unwrap_or(0) as u32,
        image_ocr_failures: row[4].get_int().unwrap_or(0) as u32,
        image_vlm_descriptions: row[5].get_int().unwrap_or(0) as u32,
        image_vlm_failures: row[6].get_int().unwrap_or(0) as u32,
        pages_rendered: row[7].get_int().unwrap_or(0) as u32,
    }
}
