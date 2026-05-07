use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::helpers::db_err;
use crate::types::{Memory, MemoryError, MemoryType};

// -- helpers --------------------------------------------------

fn extract_str(val: &DataValue) -> String {
    val.get_str().unwrap_or("").to_string()
}

fn extract_f64(val: &DataValue) -> f64 {
    val.get_float().unwrap_or(0.0)
}

fn extract_i64(val: &DataValue) -> i64 {
    val.get_int().unwrap_or(0)
}

/// Read a single memory row by id.
pub(crate) fn row_to_memory(db: &DbInstance, id: &str) -> Result<Memory, MemoryError> {
    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(id));

    let result = db
        .run_script(
            "?[id, content, title, memory_type, importance, tags,
              source_type, project_path, created_at, updated_at,
              access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags,
                          source_type, project_path, created_at, updated_at,
                          access_count, last_accessed},
                id = $id",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    if result.rows.is_empty() {
        return Err(MemoryError::NotFound(id.to_string()));
    }

    let row = &result.rows[0];
    let raw = RawRow {
        id: extract_str(&row[0]),
        content: extract_str(&row[1]),
        title: extract_str(&row[2]),
        memory_type: extract_str(&row[3]),
        importance: extract_f64(&row[4]),
        tags: extract_str(&row[5]),
        source_type: extract_str(&row[6]),
        project_path: extract_str(&row[7]),
        created_at: extract_str(&row[8]),
        updated_at: extract_str(&row[9]),
        access_count: extract_i64(&row[10]),
        last_accessed: extract_str(&row[11]),
    };

    raw_to_memory(raw)
}

pub(crate) struct RawRow {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) title: String,
    pub(crate) memory_type: String,
    pub(crate) importance: f64,
    pub(crate) tags: String,
    pub(crate) source_type: String,
    pub(crate) project_path: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) access_count: i64,
    pub(crate) last_accessed: String,
}

pub(crate) fn raw_to_memory(raw: RawRow) -> Result<Memory, MemoryError> {
    let memory_type = MemoryType::from_str_opt(&raw.memory_type)
        .ok_or_else(|| MemoryError::InvalidType(raw.memory_type.clone()))?;
    let tags: Vec<String> = serde_json::from_str(&raw.tags)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&raw.created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = chrono::DateTime::parse_from_rfc3339(&raw.updated_at)
        .map(|dt| dt.with_timezone(&Utc))
        .ok();
    let last_accessed = chrono::DateTime::parse_from_rfc3339(&raw.last_accessed)
        .map(|dt| dt.with_timezone(&Utc))
        .ok();

    Ok(Memory {
        id: raw.id,
        content: raw.content,
        title: raw.title,
        memory_type,
        importance: raw.importance,
        tags,
        source_type: raw.source_type,
        project_path: raw.project_path,
        created_at,
        updated_at,
        access_count: raw.access_count as u64,
        last_accessed,
    })
}

/// Read all memory rows from the database.
pub(crate) fn read_all_memories(db: &DbInstance) -> Result<Vec<RawRow>, MemoryError> {
    let result = db
        .run_script(
            "?[id, content, title, memory_type, importance, tags,
              source_type, project_path, created_at, updated_at,
              access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags,
                          source_type, project_path, created_at, updated_at,
                          access_count, last_accessed}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(db_err)?;

    let mut rows = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        rows.push(RawRow {
            id: extract_str(&row[0]),
            content: extract_str(&row[1]),
            title: extract_str(&row[2]),
            memory_type: extract_str(&row[3]),
            importance: extract_f64(&row[4]),
            tags: extract_str(&row[5]),
            source_type: extract_str(&row[6]),
            project_path: extract_str(&row[7]),
            created_at: extract_str(&row[8]),
            updated_at: extract_str(&row[9]),
            access_count: extract_i64(&row[10]),
            last_accessed: extract_str(&row[11]),
        });
    }
    Ok(rows)
}
