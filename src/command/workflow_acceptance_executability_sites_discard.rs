use super::*;

/// Defensively clear a mutation result whose disposable site tree could not be restored.
/// The save hook is the guarantee that an unrestored result is never persisted.
pub fn discard_mutation_evidence(
    probe: &HostProbe,
    tree: &Baseline,
    contract: &AcceptanceContract,
    id: &str,
) {
    let data = if probe.memo {
        data_state(probe, tree).ok()
    } else {
        Some(Vec::new())
    };
    if let Some(key) = data.and_then(|data| reuse::memo_key(probe, tree, &data, contract, id)) {
        if let Ok(mut entries) = memo().lock() {
            entries.remove(&key);
        }
        if let Some(store) = probe.result_store()
            && let Some(path) = store.path(&key)
        {
            let _ = std::fs::remove_file(path.with_extension("provisional"));
            let _ = std::fs::remove_file(path);
        }
    }
}
