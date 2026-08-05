//! Corpus-index relations: the claim/clause-level research index (index-overhaul C1).
//!
//! Seven `corpus_*` relations hold the CLAIM / CLAUSE / EDGE / TENSION / TERM / SOURCE /
//! GROUP record model of the final overhaul plan (spec of record: the draft plan's §3–§6
//! as amended by `phase1-final-plan/01-FINAL-OVERHAUL-PLAN.md` and the three 2026-07-29
//! rulings), plus `corpus_imports` as the import audit trail.
//!
//! Design constraints honored here:
//! - **Cozo has no `ALTER`** — the column set is a redo cost, designed once. Complex
//!   sub-objects are carried as `*_json` String columns (validated JSON) so a vocabulary
//!   evolution is a value change, not a schema change.
//! - **Decision 1 (2026-07-29):** the exact span is ALWAYS stored on CLAUSE rows;
//!   `rights_tier` + `redact_on_render` govern display, never storage.
//! - **C2 byte-vs-char rule:** offset columns are named `span_start`/`span_end` with an
//!   explicit `offset_semantics` column (`"utf8-byte"` or `"codepoint"`) — never a
//!   `char_*` name holding bytes (`doc_chunk_blocks` is the cautionary case). The
//!   [`offsets`] module is the single conversion point.
//! - **Writes** go through the archon-cozo guard (single-writer lock + SQLITE_BUSY
//!   retry) in batched multi-row puts — never one transaction per row.

use std::collections::BTreeMap;
use std::path::Path;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::KnowledgeError;

type Result<T> = std::result::Result<T, KnowledgeError>;

pub const CORPUS_SCHEMA_VERSION: &str = "corpus-v1";

// ─── Relation DDL ───────────────────────────────────────────────────────────────────────

pub const CORPUS_SOURCES_SCHEMA: &str = r#":create corpus_sources {
    source_id: String =>
    doc_id: String,
    archon_document_id: String,
    sha256: String,
    edition_key: String,
    bibliography_json: String,
    verified_bibliography: Bool,
    rights_tier: String,
    text_layer_json: String,
    page_offset_map_json: String,
    text_quality: String,
    entry_profile: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_CLAUSES_SCHEMA: &str = r#":create corpus_clauses {
    clause_id: String =>
    source_id: String,
    text_layer_id: String,
    span_start: Int,
    span_end: Int,
    offset_semantics: String,
    sentence_index: Int,
    line_start: Int,
    line_end: Int,
    page_pdf: Int,
    page_printed: Int,
    citation_address_json: String,
    quote: String,
    quote_sha256: String,
    quote_word_count: Int,
    normalized_quote: String,
    rights_tier: String,
    redact_on_render: Bool,
    bbox_json: String,
    extraction_probe_json: String,
    chunk_id: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_CLAIMS_SCHEMA: &str = r#":create corpus_claims {
    claim_id: String =>
    entry_id: String,
    unit_id: String,
    claim_text: String,
    claim_kind: String,
    scope: String,
    stance: String,
    support_tier: String,
    evidence_type: String,
    strength_rationale: String,
    provenance: String,
    locus_json: String,
    clause_refs_json: String,
    verbatim_ref_json: String,
    premises_json: String,
    supports_json: String,
    supported_by_json: String,
    contests_json: String,
    contested_by_json: String,
    tension_ids_json: String,
    cites_primary_json: String,
    cites_secondary_json: String,
    implicit_citations_json: String,
    interlocutors_json: String,
    coined_terms_json: String,
    flag_present: Bool,
    flag_text: String,
    disposition: String,
    analytic_anchor_json: String,
    volatility_json: String,
    remediation_json: String,
    use_scope: String,
    derivation: String,
    confidence: String,
    needs_human_review: Bool,
    build_provenance_json: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_EDGES_SCHEMA: &str = r#":create corpus_edges {
    edge_id: String =>
    entry_id: String,
    subject_id: String,
    relation: String,
    object_id: String,
    locator_json: String,
    provenance: String,
    support_tier: String,
    deriving_unit: String,
    note: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_TENSIONS_SCHEMA: &str = r#":create corpus_tensions {
    tension_id: String =>
    entry_id: String,
    node_a_id: String,
    node_a_label: String,
    node_b_id: String,
    node_b_label: String,
    relation: String,
    directed: Bool,
    evidence_locus_a: String,
    evidence_locus_b: String,
    description: String,
    unit_ids_json: String,
    claim_ids_json: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_TERMS_SCHEMA: &str = r#":create corpus_terms {
    term_id: String =>
    canonical: String,
    variants_json: String,
    language: String,
    transliteration: String,
    first_locus: String,
    definition_claim_id: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_GROUPS_SCHEMA: &str = r#":create corpus_groups {
    group_id: String =>
    kind: String,
    entry_ids_json: String,
    payload_json: String,
    created_at: String,
    schema_version: String
}"#;

pub const CORPUS_IMPORTS_SCHEMA: &str = r#":create corpus_imports {
    import_id: String =>
    source_kind: String,
    source_path: String,
    rows_in: Int,
    rows_written: Int,
    rows_quarantined: Int,
    quarantine_path: String,
    started_at: String,
    finished_at: String,
    tool: String
}"#;

/// Create every corpus relation (idempotent: "already exists" is swallowed, everything
/// else errors — the same discipline as `ensure_knowledge_schema`).
pub fn ensure_corpus_schema(db: &DbInstance) -> Result<()> {
    for script in [
        CORPUS_SOURCES_SCHEMA,
        CORPUS_CLAUSES_SCHEMA,
        CORPUS_CLAIMS_SCHEMA,
        CORPUS_EDGES_SCHEMA,
        CORPUS_TENSIONS_SCHEMA,
        CORPUS_TERMS_SCHEMA,
        CORPUS_GROUPS_SCHEMA,
        CORPUS_IMPORTS_SCHEMA,
    ] {
        run_create(db, script)?;
    }
    Ok(())
}

fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts with an existing") {
                Ok(())
            } else {
                Err(KnowledgeError::Schema(msg))
            }
        }
    }
}

// ─── Column specs (single source of truth for import + validation) ─────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColTy {
    Str,
    Int,
    Bool,
    /// A String column that must hold VALID JSON (import validates parseability).
    Json,
}

pub struct RelationSpec {
    pub relation: &'static str,
    pub key: &'static str,
    /// Every column in :put order, key first.
    pub columns: &'static [(&'static str, ColTy)],
    /// Non-key columns that must be non-empty strings on import.
    pub required: &'static [&'static str],
}

pub const SPEC_SOURCES: RelationSpec = RelationSpec {
    relation: "corpus_sources",
    key: "source_id",
    columns: &[
        ("source_id", ColTy::Str),
        ("doc_id", ColTy::Str),
        ("archon_document_id", ColTy::Str),
        ("sha256", ColTy::Str),
        ("edition_key", ColTy::Str),
        ("bibliography_json", ColTy::Json),
        ("verified_bibliography", ColTy::Bool),
        ("rights_tier", ColTy::Str),
        ("text_layer_json", ColTy::Json),
        ("page_offset_map_json", ColTy::Json),
        ("text_quality", ColTy::Str),
        ("entry_profile", ColTy::Str),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    required: &["doc_id", "rights_tier"],
};

pub const SPEC_CLAUSES: RelationSpec = RelationSpec {
    relation: "corpus_clauses",
    key: "clause_id",
    columns: &[
        ("clause_id", ColTy::Str),
        ("source_id", ColTy::Str),
        ("text_layer_id", ColTy::Str),
        ("span_start", ColTy::Int),
        ("span_end", ColTy::Int),
        ("offset_semantics", ColTy::Str),
        ("sentence_index", ColTy::Int),
        ("line_start", ColTy::Int),
        ("line_end", ColTy::Int),
        ("page_pdf", ColTy::Int),
        ("page_printed", ColTy::Int),
        ("citation_address_json", ColTy::Json),
        ("quote", ColTy::Str),
        ("quote_sha256", ColTy::Str),
        ("quote_word_count", ColTy::Int),
        ("normalized_quote", ColTy::Str),
        ("rights_tier", ColTy::Str),
        ("redact_on_render", ColTy::Bool),
        ("bbox_json", ColTy::Json),
        ("extraction_probe_json", ColTy::Json),
        ("chunk_id", ColTy::Str),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    required: &["source_id", "offset_semantics", "rights_tier"],
};

pub const SPEC_CLAIMS: RelationSpec = RelationSpec {
    relation: "corpus_claims",
    key: "claim_id",
    columns: &[
        ("claim_id", ColTy::Str),
        ("entry_id", ColTy::Str),
        ("unit_id", ColTy::Str),
        ("claim_text", ColTy::Str),
        ("claim_kind", ColTy::Str),
        ("scope", ColTy::Str),
        ("stance", ColTy::Str),
        ("support_tier", ColTy::Str),
        ("evidence_type", ColTy::Str),
        ("strength_rationale", ColTy::Str),
        ("provenance", ColTy::Str),
        ("locus_json", ColTy::Json),
        ("clause_refs_json", ColTy::Json),
        ("verbatim_ref_json", ColTy::Json),
        ("premises_json", ColTy::Json),
        ("supports_json", ColTy::Json),
        ("supported_by_json", ColTy::Json),
        ("contests_json", ColTy::Json),
        ("contested_by_json", ColTy::Json),
        ("tension_ids_json", ColTy::Json),
        ("cites_primary_json", ColTy::Json),
        ("cites_secondary_json", ColTy::Json),
        ("implicit_citations_json", ColTy::Json),
        ("interlocutors_json", ColTy::Json),
        ("coined_terms_json", ColTy::Json),
        ("flag_present", ColTy::Bool),
        ("flag_text", ColTy::Str),
        ("disposition", ColTy::Str),
        ("analytic_anchor_json", ColTy::Json),
        ("volatility_json", ColTy::Json),
        ("remediation_json", ColTy::Json),
        ("use_scope", ColTy::Str),
        ("derivation", ColTy::Str),
        ("confidence", ColTy::Str),
        ("needs_human_review", ColTy::Bool),
        ("build_provenance_json", ColTy::Json),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    required: &["entry_id", "claim_text", "provenance", "derivation"],
};

pub const SPEC_EDGES: RelationSpec = RelationSpec {
    relation: "corpus_edges",
    key: "edge_id",
    columns: &[
        ("edge_id", ColTy::Str),
        ("entry_id", ColTy::Str),
        ("subject_id", ColTy::Str),
        ("relation", ColTy::Str),
        ("object_id", ColTy::Str),
        ("locator_json", ColTy::Json),
        ("provenance", ColTy::Str),
        ("support_tier", ColTy::Str),
        ("deriving_unit", ColTy::Str),
        ("note", ColTy::Str),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    required: &["entry_id", "subject_id", "relation", "object_id"],
};

pub const SPEC_TENSIONS: RelationSpec = RelationSpec {
    relation: "corpus_tensions",
    key: "tension_id",
    columns: &[
        ("tension_id", ColTy::Str),
        ("entry_id", ColTy::Str),
        ("node_a_id", ColTy::Str),
        ("node_a_label", ColTy::Str),
        ("node_b_id", ColTy::Str),
        ("node_b_label", ColTy::Str),
        ("relation", ColTy::Str),
        ("directed", ColTy::Bool),
        ("evidence_locus_a", ColTy::Str),
        ("evidence_locus_b", ColTy::Str),
        ("description", ColTy::Str),
        ("unit_ids_json", ColTy::Json),
        ("claim_ids_json", ColTy::Json),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    // node_b_label is NOT required: two source dialects (In-Game's labeled tensions,
    // Grammar's single-pentad-term rows) are legitimately one-labeled.
    required: &["entry_id", "node_a_label"],
};

pub const SPEC_TERMS: RelationSpec = RelationSpec {
    relation: "corpus_terms",
    key: "term_id",
    columns: &[
        ("term_id", ColTy::Str),
        ("canonical", ColTy::Str),
        ("variants_json", ColTy::Json),
        ("language", ColTy::Str),
        ("transliteration", ColTy::Str),
        ("first_locus", ColTy::Str),
        ("definition_claim_id", ColTy::Str),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    required: &["canonical"],
};

pub const SPEC_GROUPS: RelationSpec = RelationSpec {
    relation: "corpus_groups",
    key: "group_id",
    columns: &[
        ("group_id", ColTy::Str),
        ("kind", ColTy::Str),
        ("entry_ids_json", ColTy::Json),
        ("payload_json", ColTy::Json),
        ("created_at", ColTy::Str),
        ("schema_version", ColTy::Str),
    ],
    required: &["kind"],
};

pub fn spec_for_kind(kind: &str) -> Option<&'static RelationSpec> {
    match kind {
        "sources" => Some(&SPEC_SOURCES),
        "clauses" => Some(&SPEC_CLAUSES),
        "claims" => Some(&SPEC_CLAIMS),
        "edges" => Some(&SPEC_EDGES),
        "tensions" => Some(&SPEC_TENSIONS),
        "terms" => Some(&SPEC_TERMS),
        "groups" => Some(&SPEC_GROUPS),
        _ => None,
    }
}

pub const ALL_KINDS: &[&str] = &[
    "sources", "clauses", "claims", "edges", "tensions", "terms", "groups",
];

// ─── Row conversion + batched writer ────────────────────────────────────────────────────

/// Convert one JSON object into a DataValue row in the spec's column order.
/// Missing optional columns default (Str→"", Int→-1, Bool→false, Json→"null");
/// a missing KEY or `required` column, or a type mismatch, is an error (→ quarantine).
pub fn row_from_json(
    spec: &RelationSpec,
    obj: &serde_json::Value,
) -> std::result::Result<DataValue, String> {
    let map = obj
        .as_object()
        .ok_or_else(|| "record is not a JSON object".to_string())?;
    // Unknown fields are an error, not a silent drop (the audit's `extra`-slot lesson —
    // P17 — inverted: the intermediate must MATCH the contract, not lose data into it).
    for k in map.keys() {
        if !spec.columns.iter().any(|(c, _)| c == k) {
            return Err(format!("unknown field '{k}' for {}", spec.relation));
        }
    }
    let mut out: Vec<DataValue> = Vec::with_capacity(spec.columns.len());
    for (col, ty) in spec.columns {
        let v = map.get(*col);
        let is_key = *col == spec.key;
        let is_required = is_key || spec.required.contains(col);
        match v {
            None | Some(serde_json::Value::Null) => {
                if is_required {
                    return Err(format!("missing required field '{col}'"));
                }
                out.push(match ty {
                    ColTy::Str => DataValue::from(""),
                    ColTy::Int => DataValue::from(-1i64),
                    ColTy::Bool => DataValue::from(false),
                    ColTy::Json => DataValue::from("null"),
                });
            }
            Some(val) => match ty {
                ColTy::Str => {
                    let s = val
                        .as_str()
                        .ok_or_else(|| format!("field '{col}' must be a string"))?;
                    if is_required && s.trim().is_empty() {
                        return Err(format!("required field '{col}' is empty"));
                    }
                    out.push(DataValue::from(s));
                }
                ColTy::Int => {
                    let n = val
                        .as_i64()
                        .ok_or_else(|| format!("field '{col}' must be an integer"))?;
                    out.push(DataValue::from(n));
                }
                ColTy::Bool => {
                    let b = val
                        .as_bool()
                        .ok_or_else(|| format!("field '{col}' must be a boolean"))?;
                    out.push(DataValue::from(b));
                }
                ColTy::Json => {
                    // Accept either a JSON value (stored serialized) or a pre-serialized
                    // string that must itself parse as JSON.
                    let s = match val {
                        serde_json::Value::String(s) => {
                            serde_json::from_str::<serde_json::Value>(s)
                                .map_err(|e| format!("field '{col}' is not valid JSON: {e}"))?;
                            s.clone()
                        }
                        other => other.to_string(),
                    };
                    out.push(DataValue::from(s.as_str()));
                }
            },
        }
    }
    Ok(DataValue::List(out))
}

/// Batched multi-row :put through the archon-cozo guard. `db_path` (when known) scopes
/// the cross-process write lock; in-memory test DBs pass `None`.
pub fn put_rows(
    db: &DbInstance,
    db_path: Option<&Path>,
    spec: &RelationSpec,
    rows: Vec<DataValue>,
) -> Result<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    let names: Vec<&str> = spec.columns.iter().map(|(c, _)| *c).collect();
    let head = names.join(", ");
    let non_key: Vec<&str> = names.iter().skip(1).copied().collect();
    let script = format!(
        "?[{head}] <- $rows :put {} {{ {} => {} }}",
        spec.relation,
        spec.key,
        non_key.join(", ")
    );
    let mut config = archon_cozo::CozoGuardConfig::default();
    if let Some(p) = db_path {
        config.write_lock_path = Some(archon_cozo::write_lock_path_for_db(p));
    }
    let n = rows.len();
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    archon_cozo::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Mutable,
        &format!("put {}", spec.relation),
        &config,
    )
    .map_err(|e| KnowledgeError::Schema(format!("put {} failed: {e}", spec.relation)))?;
    Ok(n)
}

/// Guarded batched :rm by key. Cozo's :rm on a missing key is a silent
/// no-op, so callers that need existence semantics must check first (the CLI
/// handler verifies the records before removing). Returns the number of keys
/// submitted.
pub fn remove_rows(
    db: &DbInstance,
    db_path: Option<&Path>,
    spec: &RelationSpec,
    key_values: &[String],
) -> Result<usize> {
    if key_values.is_empty() {
        return Ok(0);
    }
    let script = format!(
        "?[{k}] <- $rows :rm {} {{ {k} }}",
        spec.relation,
        k = spec.key
    );
    let mut config = archon_cozo::CozoGuardConfig::default();
    if let Some(p) = db_path {
        config.write_lock_path = Some(archon_cozo::write_lock_path_for_db(p));
    }
    let rows: Vec<DataValue> = key_values
        .iter()
        .map(|k| DataValue::List(vec![DataValue::from(k.as_str())]))
        .collect();
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    archon_cozo::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Mutable,
        &format!("rm {}", spec.relation),
        &config,
    )
    .map_err(|e| KnowledgeError::Schema(format!("rm {} failed: {e}", spec.relation)))?;
    Ok(key_values.len())
}

/// Single-row convenience wrapper over [`remove_rows`].
pub fn remove_row(
    db: &DbInstance,
    db_path: Option<&Path>,
    spec: &RelationSpec,
    key_value: &str,
) -> Result<()> {
    remove_rows(db, db_path, spec, &[key_value.to_string()]).map(|_| ())
}

/// Row count of one corpus relation.
pub fn count_rows(db: &DbInstance, relation: &str) -> Result<i64> {
    let script = format!(
        "?[count(k)] := *{relation}{{ {k}: k }}",
        k = match relation {
            "corpus_sources" => "source_id",
            "corpus_clauses" => "clause_id",
            "corpus_claims" => "claim_id",
            "corpus_edges" => "edge_id",
            "corpus_tensions" => "tension_id",
            "corpus_terms" => "term_id",
            "corpus_groups" => "group_id",
            "corpus_imports" => "import_id",
            other => return Err(KnowledgeError::Schema(format!("unknown relation {other}"))),
        }
    );
    let out = db
        .run_script(&script, Default::default(), ScriptMutability::Immutable)
        .map_err(|e| KnowledgeError::Schema(format!("count {relation}: {e}")))?;
    Ok(out
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| v.get_int())
        .unwrap_or(0))
}

/// Record an import in the audit trail.
#[allow(clippy::too_many_arguments)]
pub fn record_import(
    db: &DbInstance,
    db_path: Option<&Path>,
    import_id: &str,
    source_kind: &str,
    source_path: &str,
    rows_in: i64,
    rows_written: i64,
    rows_quarantined: i64,
    quarantine_path: &str,
    started_at: &str,
    finished_at: &str,
) -> Result<()> {
    let row = DataValue::List(vec![
        DataValue::from(import_id),
        DataValue::from(source_kind),
        DataValue::from(source_path),
        DataValue::from(rows_in),
        DataValue::from(rows_written),
        DataValue::from(rows_quarantined),
        DataValue::from(quarantine_path),
        DataValue::from(started_at),
        DataValue::from(finished_at),
        DataValue::from(format!("archon corpus-index ({CORPUS_SCHEMA_VERSION})").as_str()),
    ]);
    let mut config = archon_cozo::CozoGuardConfig::default();
    if let Some(p) = db_path {
        config.write_lock_path = Some(archon_cozo::write_lock_path_for_db(p));
    }
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(vec![row]));
    archon_cozo::run_script_guarded(
        db,
        "?[import_id, source_kind, source_path, rows_in, rows_written, rows_quarantined, \
          quarantine_path, started_at, finished_at, tool] <- $rows \
          :put corpus_imports { import_id => source_kind, source_path, rows_in, rows_written, \
          rows_quarantined, quarantine_path, started_at, finished_at, tool }",
        params,
        ScriptMutability::Mutable,
        "put corpus_imports",
        &config,
    )
    .map_err(|e| KnowledgeError::Schema(format!("record_import failed: {e}")))?;
    Ok(())
}

// ─── C2: the single byte↔char conversion point ─────────────────────────────────────────

/// Offset-semantics conversion (index-overhaul C2). This module is the ONE place where
/// byte offsets and codepoint offsets convert; every record stores which semantics it
/// uses in `offset_semantics` ("utf8-byte" | "codepoint").
pub mod offsets {
    /// Convert a UTF-8 BYTE offset into a codepoint (char) offset for `text`.
    /// Errors if the byte offset is out of range or not on a char boundary.
    pub fn byte_to_char(text: &str, byte: usize) -> Result<usize, String> {
        if byte > text.len() {
            return Err(format!("byte offset {byte} > text length {}", text.len()));
        }
        if !text.is_char_boundary(byte) {
            return Err(format!("byte offset {byte} is not a char boundary"));
        }
        Ok(text[..byte].chars().count())
    }

    /// Convert a codepoint (char) offset into a UTF-8 BYTE offset for `text`.
    pub fn char_to_byte(text: &str, ch: usize) -> Result<usize, String> {
        if ch == 0 {
            return Ok(0);
        }
        match text.char_indices().nth(ch) {
            Some((b, _)) => Ok(b),
            None => {
                if text.chars().count() == ch {
                    Ok(text.len())
                } else {
                    Err(format!(
                        "char offset {ch} > text length {} chars",
                        text.chars().count()
                    ))
                }
            }
        }
    }

    /// Slice `text` by a span under the named semantics.
    pub fn slice<'a>(
        text: &'a str,
        start: usize,
        end: usize,
        semantics: &str,
    ) -> Result<&'a str, String> {
        let (b0, b1) = match semantics {
            "utf8-byte" => (start, end),
            "codepoint" => (char_to_byte(text, start)?, char_to_byte(text, end)?),
            other => return Err(format!("unknown offset_semantics '{other}'")),
        };
        if b1 < b0 || b1 > text.len() || !text.is_char_boundary(b0) || !text.is_char_boundary(b1) {
            return Err(format!(
                "invalid span {b0}..{b1} for text of {} bytes",
                text.len()
            ));
        }
        Ok(&text[b0..b1])
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Greek and German — the corpus this programme exists for (C2 conformance).
        const GREEK: &str = "ἡ ψυχὴ τὰ ὄντα πώς ἐστι πάντα";
        const GERMAN: &str = "die Möglichkeit des Übergangs";

        #[test]
        fn greek_roundtrip() {
            let chars = GREEK.chars().count();
            let bytes = GREEK.len();
            assert!(bytes > chars, "multibyte fixture must differ");
            for ch in [0usize, 1, 5, chars] {
                let b = char_to_byte(GREEK, ch).unwrap();
                assert_eq!(byte_to_char(GREEK, b).unwrap(), ch);
            }
        }

        #[test]
        fn german_slice_both_semantics() {
            // "Möglichkeit" starts at char 4 (byte 4) and is 11 chars / 12 bytes (ö = 2B).
            let by_char = slice(GERMAN, 4, 15, "codepoint").unwrap();
            assert_eq!(by_char, "Möglichkeit");
            let b0 = char_to_byte(GERMAN, 4).unwrap();
            let b1 = char_to_byte(GERMAN, 15).unwrap();
            let by_byte = slice(GERMAN, b0, b1, "utf8-byte").unwrap();
            assert_eq!(by_byte, "Möglichkeit");
            assert_eq!(b1 - b0, 12);
        }

        #[test]
        fn non_boundary_byte_rejected() {
            // byte 1 lands inside the two-byte ἡ.
            assert!(byte_to_char(GREEK, 1).is_err());
            assert!(slice(GREEK, 1, 4, "utf8-byte").is_err());
        }

        #[test]
        fn unknown_semantics_rejected() {
            assert!(slice(GREEK, 0, 2, "chars??").is_err());
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", "").unwrap()
    }

    #[test]
    fn ensure_schema_creates_and_is_idempotent() {
        let db = test_db();
        ensure_corpus_schema(&db).unwrap();
        ensure_corpus_schema(&db).unwrap();
        for rel in [
            "corpus_sources",
            "corpus_clauses",
            "corpus_claims",
            "corpus_edges",
            "corpus_tensions",
            "corpus_terms",
            "corpus_groups",
            "corpus_imports",
        ] {
            assert_eq!(count_rows(&db, rel).unwrap(), 0, "{rel} should exist+empty");
        }
    }

    #[test]
    fn claim_row_roundtrip() {
        let db = test_db();
        ensure_corpus_schema(&db).unwrap();
        let obj = serde_json::json!({
            "claim_id": "DISS-01-C0001",
            "entry_id": "dissertation",
            "unit_id": "sec-1-1",
            "claim_text": "Phantasia mediates between perception and thought.",
            "claim_kind": "interpretation",
            "stance": "asserts",
            "support_tier": "T1",
            "provenance": "FAITHFUL:aristotle",
            "locus_json": [{"locator_kind": "bekker", "work": "DA", "bekker_start": "431b1"}],
            "derivation": "agent-read",
            "schema_version": CORPUS_SCHEMA_VERSION,
        });
        let row = row_from_json(&SPEC_CLAIMS, &obj).unwrap();
        assert_eq!(put_rows(&db, None, &SPEC_CLAIMS, vec![row]).unwrap(), 1);
        assert_eq!(count_rows(&db, "corpus_claims").unwrap(), 1);
    }

    #[test]
    fn validator_can_fail() {
        // Missing required field.
        let bad = serde_json::json!({"claim_id": "X", "claim_text": "y"});
        assert!(row_from_json(&SPEC_CLAIMS, &bad).is_err());
        // Unknown field.
        let unk = serde_json::json!({
            "claim_id": "X", "entry_id": "e", "claim_text": "t",
            "provenance": "FE", "derivation": "agent-read", "bogus_field": 1
        });
        assert!(row_from_json(&SPEC_CLAIMS, &unk).is_err());
        // Type mismatch.
        let ty = serde_json::json!({
            "clause_id": "c", "source_id": "s", "offset_semantics": "utf8-byte",
            "rights_tier": "own-work", "span_start": "not-an-int"
        });
        assert!(row_from_json(&SPEC_CLAUSES, &ty).is_err());
        // Invalid embedded JSON string.
        let ij = serde_json::json!({
            "claim_id": "X", "entry_id": "e", "claim_text": "t",
            "provenance": "FE", "derivation": "agent-read",
            "locus_json": "{broken"
        });
        assert!(row_from_json(&SPEC_CLAIMS, &ij).is_err());
    }

    #[test]
    fn clause_stores_exact_span_with_rights_tier() {
        // Decision 1: the span is stored regardless of tier; redaction is display-side.
        let db = test_db();
        ensure_corpus_schema(&db).unwrap();
        let obj = serde_json::json!({
            "clause_id": "EQ-CL00001",
            "source_id": "equiano-1789",
            "span_start": 100, "span_end": 160,
            "offset_semantics": "utf8-byte",
            "quote": "a sixty-byte exact passage stored under third-party copyright",
            "rights_tier": "third-party-copyright",
            "redact_on_render": true,
        });
        let row = row_from_json(&SPEC_CLAUSES, &obj).unwrap();
        assert_eq!(put_rows(&db, None, &SPEC_CLAUSES, vec![row]).unwrap(), 1);
    }
}
