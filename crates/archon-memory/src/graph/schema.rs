use super::MemoryGraph;
use super::helpers::{empty_rows, run_mutable};
use crate::types::MemoryError;

impl MemoryGraph {
    // -- schema ------------------------------------------------

    pub(super) fn init_schema(&self) -> Result<(), MemoryError> {
        run_mutable(
            &self.db,
            ":create memories {
                    id: String
                    =>
                    content: String,
                    title: String,
                    memory_type: String,
                    importance: Float,
                    tags: String,
                    source_type: String,
                    project_path: String,
                    created_at: String,
                    updated_at: String,
                    access_count: Int,
                    last_accessed: String
                }",
            Default::default(),
            "memory schema: create memories relation",
        )
        .or_else(ignore_already_exists)?;

        run_mutable(
            &self.db,
            ":create relationships {
                    from_id: String,
                    to_id: String,
                    rel_type: String
                    =>
                    context: String,
                    strength: Float,
                    created_at: String
                }",
            Default::default(),
            "memory schema: create relationships relation",
        )
        .or_else(ignore_already_exists)?;

        run_mutable(
            &self.db,
            ":create score_applications {
                    memory_id: String,
                    provenance_id: String
                    =>
                    applied_at: String
                }",
            Default::default(),
            "memory schema: create score_applications relation",
        )
        .or_else(ignore_already_exists)?;

        self.init_memory_fts()?;

        Ok(())
    }

    fn init_memory_fts(&self) -> Result<(), MemoryError> {
        for (script, context) in [
            (
                r#"::fts create memories:content_fts {
                extractor: content,
                extract_filter: content != "",
                tokenizer: NGram(1, 2, false),
                filters: [Lowercase],
            }"#,
                "memory schema: create memories:content_fts index",
            ),
            (
                r#"::fts create memories:title_fts {
                extractor: title,
                extract_filter: title != "",
                tokenizer: NGram(1, 2, false),
                filters: [Lowercase],
            }"#,
                "memory schema: create memories:title_fts index",
            ),
            (
                r#"::fts create memories:tags_fts {
                extractor: tags,
                extract_filter: tags != "[]",
                tokenizer: NGram(1, 2, false),
                filters: [Lowercase],
            }"#,
                "memory schema: create memories:tags_fts index",
            ),
        ] {
            run_mutable(&self.db, script, Default::default(), context)
                .or_else(ignore_already_exists)?;
        }
        Ok(())
    }
}

/// `:create` and `::fts create` are run unconditionally at open; treat an
/// existing relation or index as success and surface everything else.
fn ignore_already_exists(error: MemoryError) -> Result<cozo::NamedRows, MemoryError> {
    let msg = error.to_string();
    if msg.contains("already exists") || msg.contains("conflicts") {
        Ok(empty_rows())
    } else {
        Err(error)
    }
}
