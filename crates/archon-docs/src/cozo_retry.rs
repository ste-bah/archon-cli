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
    archon_cozo::run_bound_script_guarded(db, script, params, mutability, context)
}

#[cfg(test)]
mod tests {
    #[test]
    fn retryable_errors_include_sqlite_lock_and_poison_messages() {
        assert!(archon_cozo::is_retryable_cozo_error(
            "database is locked (code 5)"
        ));
        assert!(archon_cozo::is_retryable_cozo_error(
            "called with PoisonError"
        ));
        assert!(!archon_cozo::is_retryable_cozo_error("relation not found"));
    }
}
