//! Knowledge-base enumeration.
//!
//! Every other knowledge-base verb takes a name as an *input filter*, which
//! assumes the caller already knows it. Nothing projected the set of names that
//! actually exist, so a name you did not write down was unrecoverable through
//! any interface (#170). These functions are that projection.
//!
//! Two relations hold names, and a knowledge base may be in either or both:
//!
//! - `doc_kb_memberships` — written when a document is attached to a name.
//! - `kb_registry` — written when a name is declared before it has documents.
//!
//! `list_kbs` returns their union so neither origin can hide a knowledge base
//! from the other.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use super::relation_missing;
use crate::errors::{KnowledgeError, Result};

/// One knowledge base as the store knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeBaseRow {
    /// The exact string to pass to `--kb`.
    pub kb_id: String,
    /// Documents attached to this knowledge base.
    pub documents: u64,
    /// Scope recorded at declaration time (`project` / `home`). Empty for a
    /// knowledge base that exists only as membership rows, which carry no
    /// scope of their own.
    pub scope: String,
    /// Whether a declaration row exists, as opposed to membership rows alone.
    pub registered: bool,
}

/// Every knowledge base in the store, sorted by name.
///
/// A missing relation yields no rows: that is a store which has never held a
/// knowledge base, not a store that could not be read. Anything else is
/// returned as an error so callers can say so rather than render an empty
/// list.
pub fn list_kbs(db: &DbInstance) -> Result<Vec<KnowledgeBaseRow>> {
    let mut merged: BTreeMap<String, KnowledgeBaseRow> = BTreeMap::new();
    for (kb_id, documents) in list_kb_membership_counts(db)? {
        merged.insert(
            kb_id.clone(),
            KnowledgeBaseRow {
                kb_id,
                documents,
                scope: String::new(),
                registered: false,
            },
        );
    }
    for declared in list_registered_kbs(db)? {
        let row = merged
            .entry(declared.kb_id.clone())
            .or_insert_with(|| KnowledgeBaseRow {
                kb_id: declared.kb_id.clone(),
                documents: 0,
                scope: String::new(),
                registered: false,
            });
        row.scope = declared.scope;
        row.registered = true;
    }
    Ok(merged.into_values().collect())
}

/// The distinct `kb_id` values in `doc_kb_memberships` with their document
/// counts.
pub fn list_kb_membership_counts(db: &DbInstance) -> Result<Vec<(String, u64)>> {
    let script = r#"
        ?[kb_id, count(document_id)] := *doc_kb_memberships{kb_id, document_id}
    "#;
    match db.run_script(script, Default::default(), ScriptMutability::Immutable) {
        Ok(result) => Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                let kb_id = row.first()?.get_str()?.to_string();
                let count = row.get(1).and_then(DataValue::get_int).unwrap_or(0).max(0);
                Some((kb_id, count as u64))
            })
            .collect()),
        Err(e) if relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(KnowledgeError::Store(format!("list kb ids failed: {e}"))),
    }
}

/// A knowledge base declared through a surface that names it before attaching
/// anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredKb {
    pub kb_id: String,
    pub scope: String,
    pub description: String,
    pub created_at: String,
}

/// Read every declared knowledge base.
pub fn list_registered_kbs(db: &DbInstance) -> Result<Vec<RegisteredKb>> {
    let script = r#"
        ?[kb_id, scope, description, created_at] :=
            *kb_registry{kb_id, scope, description, created_at}
    "#;
    match db.run_script(script, Default::default(), ScriptMutability::Immutable) {
        Ok(result) => Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                Some(RegisteredKb {
                    kb_id: row.first()?.get_str()?.to_string(),
                    scope: cell(row, 1),
                    description: cell(row, 2),
                    created_at: cell(row, 3),
                })
            })
            .collect()),
        Err(e) if relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(KnowledgeError::Store(format!(
            "list registered kbs failed: {e}"
        ))),
    }
}

/// Declare a knowledge base by name so it is enumerable before any document is
/// attached to it. Idempotent — re-declaring refreshes scope and description.
pub fn register_kb(db: &DbInstance, kb_id: &str, scope: &str, description: &str) -> Result<()> {
    let kb_id = kb_id.trim();
    if kb_id.is_empty() {
        return Err(KnowledgeError::Store(
            "knowledge base name is required".into(),
        ));
    }
    let mut params = BTreeMap::new();
    params.insert("kid".into(), DataValue::from(kb_id));
    params.insert("scope".into(), DataValue::from(scope));
    params.insert("desc".into(), DataValue::from(description));
    let created_at = chrono::Utc::now().to_rfc3339();
    params.insert("ts".into(), DataValue::from(created_at.as_str()));
    archon_cozo::run_bound_script_guarded(
        db,
        r#"
        ?[kb_id, scope, description, created_at] <- [[$kid, $scope, $desc, $ts]]
        :put kb_registry { kb_id => scope, description, created_at }
        "#,
        params,
        ScriptMutability::Mutable,
        "knowledge store: register knowledge base",
    )
    .map_err(|e| KnowledgeError::Store(format!("register knowledge base failed: {e}")))?;
    Ok(())
}

fn cell(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
#[path = "kbs_tests.rs"]
mod tests;
