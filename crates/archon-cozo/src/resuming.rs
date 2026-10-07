//! Batch ingestion keeps its stack and registered document across acquisition pauses.
use anyhow::{Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub fn with_write_lock_resuming<T>(
    path: &Path,
    context: &str,
    wait: Duration,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    crate::locking::with_write_lock_resuming(path, context, wait, run)
}

pub fn run_bound_script_resuming(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    let config = crate::bound_guard_config(db, context)?;
    crate::guarded_run::run_guarded_mode(context, mutability, &config, true, || {
        let rows = db
            .run_script(script, params.clone(), mutability)
            .map_err(|e| anyhow!(crate::render_cozo_error(&e)))?;
        if matches!(mutability, ScriptMutability::Mutable) {
            crate::progress::record(&config);
        }
        Ok(rows)
    })
}
