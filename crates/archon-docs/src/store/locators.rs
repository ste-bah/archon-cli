use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::models::{Locator, LocatorKind};

/// Insert (upsert) a citation locator into `doc_locators`.
pub fn insert_locator(db: &DbInstance, l: &Locator) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("lid".into(), DataValue::from(l.locator_id.as_str()));
    params.insert("did".into(), DataValue::from(l.document_id.as_str()));
    params.insert("pn".into(), DataValue::from(l.page_num as i64));
    params.insert("kind".into(), DataValue::from(l.kind.as_str()));
    params.insert("val".into(), DataValue::from(l.value.as_str()));
    params.insert("bbox".into(), DataValue::from(l.bbox.as_str()));
    crate::cozo_retry::run_script_guarded(
        db,
        "?[locator_id, document_id, page_num, kind, value, bbox] \
         <- [[$lid, $did, $pn, $kind, $val, $bbox]] \
         :put doc_locators { locator_id => document_id, page_num, kind, value, bbox }",
        params,
        ScriptMutability::Mutable,
        "insert doc_locators",
    )
    .map_err(|e| anyhow::anyhow!("insert doc_locators failed: {e}"))?;
    Ok(())
}

/// List a document's citation locators, ordered by page then value.
pub fn list_locators_for_doc(db: &DbInstance, document_id: &str) -> Result<Vec<Locator>> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(document_id));
    let result = crate::cozo_retry::run_script_guarded(
        db,
        "?[locator_id, document_id, page_num, kind, value, bbox] \
         := *doc_locators{locator_id, document_id, page_num, kind, value, bbox}, document_id = $did",
        params,
        ScriptMutability::Immutable,
        "list doc_locators",
    )
    .map_err(|e| anyhow::anyhow!("list doc_locators failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| Locator {
            locator_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            page_num: row[2].get_int().unwrap_or(0) as u32,
            kind: LocatorKind::parse(row[3].get_str().unwrap_or("PageNumber")),
            value: row[4].get_str().unwrap_or("").to_string(),
            bbox: row[5].get_str().unwrap_or("").to_string(),
        })
        .collect())
}
