use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    archon_cozo::run_script_guarded(
        db,
        script,
        params,
        mutability,
        context,
        &archon_cozo::CozoGuardConfig::default(),
    )
}

pub fn open_sqlite_guarded(path: &str, context: &str) -> Result<DbInstance> {
    let config = archon_cozo::CozoGuardConfig::for_db_path(path);
    archon_cozo::open_sqlite_guarded(path, context, &config)
}

pub fn ensure_learning_schema_guarded(db: &DbInstance, path: &std::path::Path) -> Result<()> {
    let config = archon_cozo::CozoGuardConfig::for_db_path(path);
    archon_cozo::run_guarded(
        "ensure governed learning schema",
        ScriptMutability::Mutable,
        &config,
        || crate::schema::ensure_learning_schema(db),
    )
}

pub async fn open_sqlite_guarded_async(path: &str, context: &str) -> Result<DbInstance> {
    let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(path);
    archon_cozo::open_sqlite_guarded_async(path, context, &config).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn retryable_errors_include_sqlite_busy_messages() {
        assert!(archon_cozo::is_retryable_cozo_error(
            "database is locked (code 5)"
        ));
        assert!(archon_cozo::is_retryable_cozo_error(
            "Error { code: Some(5) }"
        ));
        assert!(!archon_cozo::is_retryable_cozo_error("relation not found"));
    }
}
