use std::collections::BTreeMap;
use std::path::Path;

use archon_cozo::{CozoGuardConfig, GuardedDbInstance};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use crate::CognitiveError;

pub(crate) fn relation_count(
    db: &DbInstance,
    relation: &str,
    field: &str,
) -> Result<usize, CognitiveError> {
    let query = format!("?[{field}] := *{relation}{{{field}}}");
    let rows = run_script_guarded(
        db,
        query.as_str(),
        Default::default(),
        ScriptMutability::Immutable,
        "count cognitive relation",
    )?;
    Ok(rows.rows.len())
}

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows, CognitiveError> {
    archon_cozo::run_bound_script_guarded(db, script, params, mutability, context)
        .map_err(|error| CognitiveError::Store(format!("{error:#}")))
}

pub(crate) fn open_sqlite_guarded(
    path: &Path,
    context: &str,
) -> Result<GuardedDbInstance, CognitiveError> {
    let path_text = path.to_string_lossy();
    archon_cozo::open_sqlite_guarded_instance(
        &path_text,
        context,
        CozoGuardConfig::for_db_path(path),
    )
    .map_err(|error| CognitiveError::Store(format!("{error:#}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_database_retains_its_write_lock_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cognitive.db");
        let database = open_sqlite_guarded(&path, "open test cognitive store").unwrap();

        assert_eq!(
            database.config().write_lock_path.as_deref(),
            Some(archon_cozo::write_lock_path_for_db(&path).as_path())
        );
    }
}
