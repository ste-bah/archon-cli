//! `archon memory ...` CLI subcommand handler.
//!
//! Distinct from `src/command/memory.rs`, which is the in-session
//! `/memory` slash handler. This file backs the top-level `archon
//! memory` subcommand surface.
//!
//! Currently exposes one subcommand: `archon memory reindex --all`,
//! which re-embeds every memory in the persistent graph using the
//! currently-configured embedding provider. Used after swapping
//! embedding models or recovering from a corrupted prior model.

use anyhow::{Context, Result};

use crate::cli_args::MemoryAction;

pub async fn handle_memory_command(action: MemoryAction) -> Result<()> {
    match action {
        MemoryAction::Reindex { all } => handle_reindex(all).await,
    }
}

async fn handle_reindex(all: bool) -> Result<()> {
    if !all {
        eprintln!(
            "archon memory reindex requires --all to confirm. \
             This re-embeds every memory in the graph and may take a while."
        );
        std::process::exit(1);
    }

    // Resolve config (so we use the configured embedding provider).
    let config = archon_core::config::load_config().context("failed to load archon config")?;
    let record = crate::command::world_model::record_runtime_advisory(
        &config,
        archon_world_model::integration::WorldAdvisorSurface::MemorySurfacing,
        "memory-cli",
        "memory_reindex",
        "reindex memory embeddings",
    );
    tracing::debug!(
        continue_foreground_flow = record.continue_foreground_flow,
        "world_model.memory_advisory"
    );

    let (memory_data_dir, memory_db_path) =
        archon_memory::resolve_memory_paths(config.memory.db_path.as_deref());

    // Open the memory graph in Direct mode (we need set_embedding_provider
    // + reindex_all_embeddings, both of which require the concrete
    // MemoryGraph rather than the trait-object access wrapper).
    let access = archon_memory::open_memory_with_db_path(&memory_data_dir, &memory_db_path)
        .await
        .context("failed to open memory graph")?;
    let graph = access
        .graph()
        .context("memory graph not in Direct mode (cannot reindex)")?;

    // Wire up an embedding provider — same code path as session bootstrap.
    let embed_cfg = archon_memory::embedding::EmbeddingConfig {
        provider: config.memory.embedding_provider.clone(),
        hybrid_alpha: config.memory.hybrid_alpha,
    };
    let provider = archon_memory::embedding::create_provider(&embed_cfg)
        .context("failed to create embedding provider")?;
    graph
        .set_embedding_provider(provider)
        .context("failed to attach embedding provider to graph")?;

    let total = graph.memory_count().context("failed to count memories")?;
    println!(
        "Reindexing {total} memories under provider '{}'...",
        embed_cfg.provider
    );

    let started = std::time::Instant::now();
    let (reindexed, skipped, failed) = graph
        .reindex_all_embeddings()
        .context("reindex_all_embeddings failed")?;
    let elapsed = started.elapsed();

    println!("Reindexed: {reindexed}");
    if skipped > 0 {
        println!("Skipped:   {skipped} (content shorter than min-embed threshold)");
    }
    if failed > 0 {
        println!("Failed:    {failed} (see logs)");
    }
    println!("Elapsed:   {:.1}s", elapsed.as_secs_f64());
    Ok(())
}
