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

/// Drop the entry a [`crate::GuardedDbInstance`] registered for itself.
///
/// Pruning used to happen only as a side effect of the next `register` or
/// lookup, so a dead entry survived until unrelated traffic arrived — and each
/// one holds a `Weak`, which keeps the whole `ArcInner<DbInstance>` allocation
/// alive for as long as it lingers. A process that opens databases and then
/// goes quiet kept every one of them. Deregistering at the point the last owner
/// drops makes the removal an event rather than a side effect.
pub(crate) fn deregister_guarded_database(key: usize) {
    let configs = GUARDED_DATABASE_CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut configs = lock_recovering_poison(configs);
    configs.remove(&key);
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
