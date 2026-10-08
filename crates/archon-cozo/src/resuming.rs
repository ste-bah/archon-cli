//! Batch callers keep their operation while writers progress.
//!
//! These are the same no-progress windows as the plain guarded calls. The
//! wait continues while a holder or writer makes progress, and ends as one
//! typed `StoreBusy` pause that the caller can resume later.
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
    crate::acquire::with_write_lock_resuming(path, context, wait, run)
}

pub fn run_bound_script_resuming(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    let config = crate::bound_guard_config(db, context)?;
    crate::run_guarded(context, mutability, &config, || {
        let rows = db
            .run_script(script, params.clone(), mutability)
            .map_err(|e| anyhow!(crate::render_cozo_error(&e)))?;
        if matches!(mutability, ScriptMutability::Mutable) {
            crate::progress::record(&config);
        }
        Ok(rows)
    })
}
