use std::sync::{Mutex, MutexGuard};

use anyhow::Result;

use super::{
    clear_embedding_migration, drop_embedding_indices, embedding_config, embedding_migration,
    relation_exists, remove_relation_if_exists, restore_embedding_backup,
};

static EMBEDDING_STATE: Mutex<()> = Mutex::new(());

pub(crate) fn lock_embedding_state() -> Result<MutexGuard<'static, ()>> {
    EMBEDDING_STATE
        .lock()
        .map_err(|error| anyhow::anyhow!("KB embedding state lock poisoned: {error}"))
}

pub(crate) fn assert_embedding_space(
    db: &cozo::DbInstance,
    provider: &str,
    dim: usize,
) -> Result<()> {
    let active = super::embedding_config(db)?;
    if active
        .as_ref()
        .is_some_and(|(active_provider, active_dim)| {
            active_provider == provider && *active_dim == dim
        })
    {
        return Ok(());
    }
    anyhow::bail!("KB embedding space changed; recreate the knowledge base handle")
}

pub(crate) fn recover_interrupted_migration(db: &cozo::DbInstance) -> Result<()> {
    let Some((target_provider, target_dim)) = embedding_migration(db)? else {
        return Ok(());
    };
    let config = embedding_config(db)?;
    let committed = config
        .as_ref()
        .is_some_and(|(provider, dim)| provider == &target_provider && *dim == target_dim);
    if relation_exists(db, "kb_embeddings_backup")? {
        if committed {
            drop_embedding_indices(db, "kb_embeddings_backup")?;
            remove_relation_if_exists(db, "kb_embeddings_backup")?;
        } else {
            restore_embedding_backup(db, config.as_ref().map(|(_, dim)| *dim))?;
        }
    }
    clear_embedding_migration(db)
}

pub(crate) fn rollback_embedding_activation(
    db: &cozo::DbInstance,
    had_active: bool,
    previous_dim: Option<usize>,
) -> Result<()> {
    if had_active {
        restore_embedding_backup(db, previous_dim)?;
    } else {
        drop_embedding_indices(db, "kb_embeddings")?;
        remove_relation_if_exists(db, "kb_embeddings")?;
    }
    clear_embedding_migration(db)
}
