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
    if let Err(e) = crate::command::docs_embedding::init_embedding(db) {
        println!("Warning: {label} semantic indexing skipped: {e}");
        return;
    }
    if archon_docs::embed::get_provider().is_none() {
        let detail = archon_docs::embed::last_init_error()
            .unwrap_or_else(|| "no embedding provider configured".into());
        println!("Warning: {label} semantic indexing skipped: {detail}");
        return;
    }

    match archon_docs::indexing::index_chunks(db, &archon_docs::indexing::IndexOptions::default()) {
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
