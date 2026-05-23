use cozo::DbInstance;

pub(crate) fn index_pending_evidence(db: &DbInstance, label: &str) {
    let pending = match archon_docs::store::count_pending_chunks(db) {
        Ok(count) => count,
        Err(e) => {
            println!("Warning: {label} semantic indexing status unavailable: {e}");
            return;
        }
    };
    if pending == 0 {
        return;
    }

    println!("Indexing {pending} pending {label} chunk(s) for semantic search...");
    if archon_docs::embed::get_provider().is_none()
        && let Err(e) = archon_docs::embed::init_default_provider()
    {
        println!("Warning: {label} semantic indexing skipped: {e}");
        return;
    }

    match archon_docs::retrieval::index_pending_chunks(db) {
        Ok(result) => {
            if result.indexed > 0 {
                println!("Indexed {label}: {} chunk(s)", result.indexed);
            }
            if result.failed > 0 {
                println!(
                    "Warning: {label} semantic indexing failed for {} chunk(s)",
                    result.failed
                );
            }
            if result.skipped > 0 {
                println!(
                    "Skipped {label}: {} already-indexed chunk(s)",
                    result.skipped
                );
            }
        }
        Err(e) => {
            println!("Warning: {label} semantic indexing failed: {e}");
        }
    }
}
