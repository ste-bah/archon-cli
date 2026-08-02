pub mod cozo_store;
pub mod jsonl;

use std::path::{Path, PathBuf};

use anyhow::Result;
use archon_cozo::{CozoGuardConfig, GuardedDbInstance};

use crate::types::ReasoningQualityEvent;

pub struct ReasoningQualityStore {
    root: PathBuf,
    /// Guarded handle. Registering the instance is what lets the free
    /// functions in [`cozo_store`] resolve the write-lock config by pointer
    /// identity, so every `Mutable` script serialises against other writers
    /// of the same `reasoning-quality.db` file.
    db: GuardedDbInstance,
}

impl ReasoningQualityStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        let db_path = root.join("reasoning-quality.db");
        let db = archon_cozo::open_sqlite_guarded_instance(
            db_path.to_string_lossy().as_ref(),
            "open reasoning-quality store",
            CozoGuardConfig::for_db_path(&db_path),
        )
        .map_err(|e| anyhow::anyhow!("reasoning-quality db open failed: {e}"))?;
        cozo_store::ensure_schema(&db)?;
        Ok(Self { root, db })
    }

    pub fn append_events(&self, events: &[ReasoningQualityEvent]) -> Result<usize> {
        jsonl::append_events(&self.root, events)?;
        cozo_store::put_events(&self.db, events)
    }

    pub fn count_events(&self) -> Result<usize> {
        cozo_store::count_events(&self.db)
    }

    pub fn events_for_session(&self, session_id: &str) -> Result<Vec<ReasoningQualityEvent>> {
        cozo_store::events_for_session(&self.db, session_id)
    }

    pub fn recent_events(&self, limit: usize) -> Result<Vec<ReasoningQualityEvent>> {
        cozo_store::recent_events(&self.db, limit)
    }

    pub fn record_schema_migration(&self, to_version: u32, dry_run: bool) -> Result<()> {
        cozo_store::put_schema_migration(&self.db, to_version, dry_run)
    }
}

#[cfg(test)]
mod tests {
    use crate::extractor::{DeterministicExtractor, ExtractorConfig};
    use crate::types::ReasoningTurnInput;

    use super::*;

    #[test]
    fn store_appends_jsonl_and_upserts_cozo() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReasoningQualityStore::open(temp.path()).unwrap();
        let input = ReasoningTurnInput {
            session_id: "s1".to_string(),
            turn_number: 1,
            assistant_text: "The module src/lib.rs exists.".to_string(),
            ..ReasoningTurnInput::default()
        };
        let events = DeterministicExtractor::new(ExtractorConfig::default()).extract_turn(&input);
        store.append_events(&events).unwrap();
        store.append_events(&events).unwrap();

        assert_eq!(store.count_events().unwrap(), 1);
        assert_eq!(store.events_for_session("s1").unwrap().len(), 1);
    }
}
