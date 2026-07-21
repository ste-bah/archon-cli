use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::{Result, anyhow};
use cozo::DbInstance;

use crate::CozoGuardConfig;
use crate::locking::lock_recovering_poison;

static GUARDED_DATABASE_CONFIGS: OnceLock<Mutex<HashMap<usize, GuardedDatabaseConfig>>> =
    OnceLock::new();

struct GuardedDatabaseConfig {
    database: Weak<DbInstance>,
    config: CozoGuardConfig,
}

pub(crate) fn register_guarded_database(db: &Arc<DbInstance>, config: &CozoGuardConfig) {
    let key = Arc::as_ptr(db) as usize;
    let configs = GUARDED_DATABASE_CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut configs = lock_recovering_poison(configs);
    configs.retain(|_, entry| entry.database.strong_count() > 0);
    configs.insert(
        key,
        GuardedDatabaseConfig {
            database: Arc::downgrade(db),
            config: config.clone(),
        },
    );
}

pub(crate) fn guarded_config_for(db: &DbInstance) -> Option<CozoGuardConfig> {
    let key = db as *const DbInstance as usize;
    let configs = GUARDED_DATABASE_CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut configs = lock_recovering_poison(configs);
    configs.retain(|_, entry| entry.database.strong_count() > 0);
    configs.get(&key).map(|entry| entry.config.clone())
}

pub(crate) fn bound_guard_config(db: &DbInstance, context: &str) -> Result<CozoGuardConfig> {
    if let Some(config) = guarded_config_for(db) {
        return Ok(config);
    }
    if matches!(db, DbInstance::Mem(_)) {
        return Ok(CozoGuardConfig::default());
    }
    Err(anyhow!(
        "{context}: database has no bound Cozo guard config"
    ))
}

#[cfg(test)]
pub(crate) fn registered_database_keys() -> Vec<usize> {
    let configs = GUARDED_DATABASE_CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    lock_recovering_poison(configs).keys().copied().collect()
}
