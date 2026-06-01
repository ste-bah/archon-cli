use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use archon_cozo::CozoGuardConfig;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

static CONFIG: OnceLock<Mutex<CozoGuardConfig>> = OnceLock::new();

pub(crate) fn configure_write_lock_for_db(path: impl AsRef<Path>) {
    let lock = CONFIG.get_or_init(|| Mutex::new(CozoGuardConfig::default()));
    let mut config = match lock.lock() {
        Ok(config) => config,
        Err(poisoned) => poisoned.into_inner(),
    };
    config.write_lock_path = Some(archon_cozo::write_lock_path_for_db(path));
}

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    let config = current_config();
    archon_cozo::run_script_guarded(db, script, params, mutability, context, &config)
}

fn current_config() -> CozoGuardConfig {
    let lock = CONFIG.get_or_init(|| Mutex::new(CozoGuardConfig::default()));
    match lock.lock() {
        Ok(config) => config.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn configured_write_lock_uses_db_sidecar_path() {
        configure_write_lock_for_db("/tmp/example.db");
        let config = current_config();
        assert_eq!(
            config.write_lock_path.unwrap(),
            std::path::PathBuf::from("/tmp/example.db.archon-cozo-write.lock")
        );
    }
}
