use anyhow::Result;
use cozo::DbInstance;

use archon_docs::vector_store::DocVectorStore;

pub(crate) fn handle_vector_status(db: DbInstance) -> Result<()> {
    let store = DocVectorStore::open_default()?;
    let legacy = archon_docs::vector_migration::legacy_vector_count(&db)?;
    let stats = store.stats(None)?;
    println!("Legacy Cozo vectors: {}", legacy);
    println!(
        "RocksDB vector store: {}",
        archon_docs::vector_store::default_store_dir().display()
    );
    println!("Raw vectors:          {}", stats.raw_vectors);
    println!("Cache entries:        {}", stats.cache_entries);
    if let Some(provider) = current_provider_name() {
        let provider_stats = store.stats(Some(&provider))?;
        println!("Provider:             {}", provider);
        println!("Provider raw vectors: {}", provider_stats.raw_vectors);
        match store.latest_hnsw_manifest(&provider)? {
            Some(manifest) => println!(
                "Rust HNSW snapshot:   {} vectors, dim {}, basename {}",
                manifest.vector_count, manifest.dimension, manifest.dump_basename
            ),
            None => println!("Rust HNSW snapshot:   not built"),
        }
    }
    Ok(())
}

pub(crate) fn handle_vector_migrate(
    db: DbInstance,
    limit: Option<usize>,
    batch_size: usize,
    after: Option<String>,
) -> Result<()> {
    let store = DocVectorStore::open_default()?;
    let report = archon_docs::vector_migration::migrate_legacy_vectors(
        &db,
        &store,
        limit,
        batch_size,
        after.as_deref(),
    )?;
    println!("Scanned legacy vectors: {}", report.scanned);
    println!("Migrated to RocksDB:    {}", report.migrated);
    println!("Skipped existing:       {}", report.skipped_existing);
    if let Some(last) = report.last_chunk_id {
        println!("Last chunk id:          {}", last);
        println!("Resume with:            archon docs vector-migrate --after {last}");
    }
    Ok(())
}

pub(crate) fn handle_vector_compact(
    db: DbInstance,
    provider: Option<String>,
    dimension: Option<usize>,
    limit: Option<usize>,
) -> Result<()> {
    crate::command::docs_embedding::init_embedding(&db)?;
    let provider_name = provider
        .or_else(current_provider_name)
        .ok_or_else(|| anyhow::anyhow!("no embedding provider available for vector compaction"))?;
    let dimension = dimension
        .or_else(current_provider_dimension)
        .ok_or_else(|| anyhow::anyhow!("no embedding dimension available for vector compaction"))?;
    let store = DocVectorStore::open_default()?;
    let manifest = store.build_hnsw(&provider_name, dimension, limit)?;
    println!("Rust HNSW snapshot built.");
    println!("Provider:   {}", manifest.provider);
    println!("Dimension:  {}", manifest.dimension);
    println!("Vectors:    {}", manifest.vector_count);
    println!("Basename:   {}", manifest.dump_basename);
    Ok(())
}

fn current_provider_name() -> Option<String> {
    archon_docs::embed::get_provider().map(|provider| provider.backend_name().to_string())
}

fn current_provider_dimension() -> Option<usize> {
    archon_docs::embed::get_provider().map(|provider| provider.dimension())
}
