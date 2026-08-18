use std::sync::atomic::{AtomicU64, Ordering};

use cozo::{DataValue, DbInstance};

static NEXT_IDENTITY: AtomicU64 = AtomicU64::new(1);

/// Returns the storage-resident identity for an in-memory Cozo database.
///
/// A write transaction serializes relation creation, identity lookup, and the
/// first insert. Concurrent callers therefore either observe the committed
/// row or create it; no caller can overwrite a previously stored identity.
pub(super) fn database_identity(db: &DbInstance) -> Option<String> {
    if !matches!(db, DbInstance::Mem(_)) {
        return None;
    }

    const CREATE: &str = ":create archon_cozo_store_identity { key: String => value: String }";
    const INSERT: &str =
        "?[key, value] <- [['identity', $value]] :insert archon_cozo_store_identity {key => value}";
    const SELECT: &str = "?[value] := *archon_cozo_store_identity{key, value}, key = 'identity'";

    let transaction = db.multi_transaction(true);
    let identity = (|| {
        let _ = transaction.run_script(CREATE, Default::default());
        if let Some(identity) = transaction
            .run_script(SELECT, Default::default())
            .ok()
            .and_then(first_string)
        {
            return Some(identity);
        }
        let value = format!("mem-{}", NEXT_IDENTITY.fetch_add(1, Ordering::Relaxed));
        let params = [("value".to_owned(), DataValue::from(value.as_str()))]
            .into_iter()
            .collect();
        transaction.run_script(INSERT, params).ok()?;
        Some(value)
    })();
    if identity.is_some() {
        transaction.commit().ok()?;
        identity
    } else {
        let _ = transaction.abort();
        None
    }
}

fn first_string(rows: cozo::NamedRows) -> Option<String> {
    rows.rows
        .first()
        .and_then(|row| row.first())
        .and_then(DataValue::get_str)
        .map(str::to_owned)
}
