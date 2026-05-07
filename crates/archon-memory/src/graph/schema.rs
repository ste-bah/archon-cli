use super::MemoryGraph;
use super::helpers::{db_err, empty_rows};
use crate::types::MemoryError;
use cozo::ScriptMutability;

impl MemoryGraph {
    // -- schema ------------------------------------------------

    pub(super) fn init_schema(&self) -> Result<(), MemoryError> {
        self.db
            .run_script(
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
                ScriptMutability::Mutable,
            )
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(e))
                }
            })?;

        self.db
            .run_script(
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
                ScriptMutability::Mutable,
            )
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(e))
                }
            })?;

        Ok(())
    }
}
