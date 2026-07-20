use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::PageArtifact;

use super::common::optional_str;

pub fn insert_page(db: &DbInstance, page: &PageArtifact) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(page.page_id.as_str()));
    params.insert("did".into(), DataValue::from(page.document_id.as_str()));
    params.insert("pnum".into(), DataValue::from(page.page_number as i64));
    params.insert(
        "thash".into(),
        DataValue::from(page.text_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "ihash".into(),
        DataValue::from(page.image_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "w".into(),
        DataValue::from(page.width.unwrap_or(0.0) as f64),
    );
    params.insert(
        "h".into(),
        DataValue::from(page.height.unwrap_or(0.0) as f64),
    );
    params.insert(
        "prov".into(),
        DataValue::from(page.provenance_record_id.as_str()),
    );

    crate::cozo_retry::run_script_guarded(
        db,
        "?[page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id] \
         <- [[$pid, $did, $pnum, $thash, $ihash, $w, $h, $prov]] \
         :put doc_pages { page_id => document_id, page_number, text_hash, image_hash, width, height, provenance_record_id }",
        params,
        ScriptMutability::Mutable,
        "insert doc_pages",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_pages failed: {e}"))?;
    Ok(())
}

pub fn list_pages_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<PageArtifact>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));

    let result = db
        .run_script(
            "?[page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id] \
             := *doc_pages{page_id, document_id, page_number, text_hash, image_hash, width, height, provenance_record_id}, \
             document_id = $did",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list pages failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| PageArtifact {
            page_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            page_number: row[2].get_int().unwrap_or(0) as u32,
            text_hash: optional_str(row[3].get_str().unwrap_or("")),
            image_hash: optional_str(row[4].get_str().unwrap_or("")),
            width: {
                let v = row[5].get_float().unwrap_or(0.0);
                if v == 0.0 { None } else { Some(v as f32) }
            },
            height: {
                let v = row[6].get_float().unwrap_or(0.0);
                if v == 0.0 { None } else { Some(v as f32) }
            },
            provenance_record_id: row[7].get_str().unwrap_or("").to_string(),
        })
        .collect())
}
