// Reading only the records this store itself wrote.
//
// The store's directories live inside the run's own working tree, and the
// agents a run dispatches can write there. One did: a branch left a
// hand-authored JSON note in a directory under `branches/`, and from that
// moment every directory walk below hard-failed deserializing that note as a
// branch outcome. Because every stage loads these records while preparing its
// fan-out, one foreign file failed every later stage before it started, and
// the failures kept arriving instantly until the run was gone.
//
// So a file is treated as this store's own record only when it is a JSON
// object carrying every key the record type always serializes. Anything else
// was written by something that is not this store, and is skipped rather than
// failing the caller. Skipping cannot lose a record: every write goes through
// `write_json`, which renames a fully serialized temporary file into place, so
// a file this store wrote is always a complete JSON object with those keys.
//
// A document that IS shaped like one of ours and still will not parse stays
// fatal. Dropping it would silently change completion accounting, which is the
// one thing this must not do, so it surfaces as corrupt state naming the file
// instead of as a bare deserializer message with no path in it.

/// A record type this store persists, one file per record.
pub(super) trait StoreRecord: DeserializeOwned {
    /// Keys this store always serializes for the record.
    ///
    /// Every key listed is required by the type itself (no serde default), so
    /// a file this store wrote carries all of them and a document missing any
    /// one of them cannot be a record this store wrote.
    const IDENTIFYING_KEYS: &'static [&'static str];
}

impl StoreRecord for WorkflowV2BranchOutcome {
    const IDENTIFYING_KEYS: &'static [&'static str] =
        &["item_id", "role", "status", "result", "error"];
}

impl StoreRecord for WorkflowV2CallRecord {
    const IDENTIFYING_KEYS: &'static [&'static str] =
        &["call", "attempt", "input_hash", "status", "result"];
}

/// Read one record file from inside the store.
///
/// `Ok(None)` means the file is not one of this store's records and the caller
/// should carry on as if it were not there.
pub(super) fn read_store_record<T: StoreRecord>(path: &Path) -> WorkflowResult<Option<T>> {
    let raw = fs::read_to_string(path).map_err(|err| WorkflowError::io(path, err))?;
    parse_store_record(&raw, path)
}

pub(super) fn parse_store_record<T: StoreRecord>(
    raw: &str,
    path: &Path,
) -> WorkflowResult<Option<T>> {
    // Not JSON, or not an object: `write_json` renames a serialized object
    // into place, so this file did not come from here.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if !T::IDENTIFYING_KEYS
        .iter()
        .all(|key| object.contains_key(*key))
    {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|err| corrupt_store_record(path, err))
}

fn corrupt_store_record(path: &Path, err: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::StateCorrupt(format!("{}: {err}", path.display()))
}
