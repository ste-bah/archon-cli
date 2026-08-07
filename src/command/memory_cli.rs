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
use archon_core::config::ArchonConfig;

use crate::cli_args::MemoryAction;

pub async fn handle_memory_command(action: MemoryAction, config: &ArchonConfig) -> Result<()> {
    match action {
        MemoryAction::Reindex { all } => handle_reindex(all, config).await,
    }
}

/// `config` is the fully layered bootstrap config (user → project → local →
/// settings, plus env overrides) — the same resolution the interactive
/// session uses. Do not re-load config here with `config::load_config()`:
/// that reads only the user layer and can resolve a different
/// `[memory] embedding_provider` than the session in the same workspace.
async fn handle_reindex(all: bool, config: &ArchonConfig) -> Result<()> {
    if !all {
        eprintln!(
            "archon memory reindex requires --all to confirm. \
             This re-embeds every memory in the graph and may take a while."
        );
        std::process::exit(1);
    }

    let record = crate::command::world_model::record_runtime_advisory(
        config,
        archon_world_model::integration::WorldAdvisorSurface::MemorySurfacing,
        "memory-cli",
        "memory_reindex",
        "reindex memory embeddings",
    );
    tracing::debug!(
        continue_foreground_flow = record.continue_foreground_flow,
        "world_model.memory_advisory"
    );

    // Resolve, open through the election, attach the embedding provider — the
    // shared body every entry point uses since #146. Direct mode is still
    // required afterwards: `reindex_all_embeddings` is on the concrete
    // MemoryGraph, not on the trait-object access wrapper.
    let spec = config.memory.open_spec();
    let opened = archon_memory::open_configured_memory(&spec)
        .await
        .context("failed to open memory graph")?;

    // The one caller that must refuse a store with no embedder. Everywhere else
    // keyword-only search is a degraded but honest service; here it would mean
    // re-embedding nothing and printing a completion for it.
    if let archon_memory::EmbeddingSetup::Unavailable(reason) = &opened.embedding {
        anyhow::bail!("{reason}");
    }
    let graph = opened
        .access
        .graph()
        .context("memory graph not in Direct mode (cannot reindex)")?;

    let total = graph.memory_count().context("failed to count memories")?;
    println!(
        "Reindexing {total} memories under provider '{}'...",
        spec.embedding.provider
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
