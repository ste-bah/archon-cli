use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};

use super::MemoryGraph;
use super::helpers::db_err;
use crate::types::{MemoryError, RelType};

impl MemoryGraph {
    // -- relationships -----------------------------------------

    /// Create a directed relationship between two memories.
    pub fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        let now = Utc::now().to_rfc3339();
        let ctx = context.unwrap_or("");

        let mut params = BTreeMap::new();
        params.insert("from_id".to_string(), DataValue::from(from_id));
        params.insert("to_id".to_string(), DataValue::from(to_id));
        params.insert(
            "rel_type".to_string(),
            DataValue::from(rel_type.to_string()),
        );
        params.insert("context".to_string(), DataValue::from(ctx));
        params.insert("strength".to_string(), DataValue::from(strength));
        params.insert("created_at".to_string(), DataValue::from(now));

        self.db
            .run_script(
                "?[from_id, to_id, rel_type, context, strength, created_at] <- [[
                    $from_id, $to_id, $rel_type, $context, $strength, $created_at
                ]]
                :put relationships {from_id, to_id, rel_type => context, strength, created_at}",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        Ok(())
    }
}
