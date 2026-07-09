//! `archon draft` — run the FCDP drafting protocol on a context pack.
//!
//! Surfaces the archon-draft orchestrator (D1 → D1.5 → D2 → gauntlet → R-loop) as a
//! first-class subcommand. Model resolves via `--model` → configured Anthropic Opus →
//! built-in default; auth (subscription OAuth or API key) is resolved by archon-llm.
//!
//! The orchestrator is synchronous and its model client drives its own Tokio runtime, so
//! it is run on a blocking thread (`spawn_blocking`) — a `block_on` inside the async
//! main runtime would panic.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_draft::fable::{self, FableClient};
use archon_draft::{GateConfig, Pack, QuoteBank, orchestrator};

pub(crate) async fn handle_draft_command(
    pack_path: PathBuf,
    workdir: PathBuf,
    model: Option<String>,
    gate_config: Option<PathBuf>,
    config: &ArchonConfig,
) -> Result<()> {
    let pack_dir = pack_path.parent().unwrap_or_else(|| Path::new("."));

    let pack: Pack = serde_json::from_str(
        &std::fs::read_to_string(&pack_path)
            .with_context(|| format!("read pack {}", pack_path.display()))?,
    )
    .with_context(|| "parse pack")?;

    let bank_path = pack_dir.join(&pack.p4b_bank_path);
    let bank: QuoteBank = serde_json::from_str(
        &std::fs::read_to_string(&bank_path)
            .with_context(|| format!("read bank {}", bank_path.display()))?,
    )
    .with_context(|| "parse bank")?;

    let gc_path =
        gate_config.unwrap_or_else(|| pack_dir.join(&pack.p2_style_target.gate_config_path));
    let cfg: GateConfig = serde_json::from_str(
        &std::fs::read_to_string(&gc_path)
            .with_context(|| format!("read gate config {}", gc_path.display()))?,
    )
    .with_context(|| "parse gate config")?;

    // --model → configured Anthropic Opus → built-in default (claude-opus-4-8).
    let model = fable::resolve_model(
        model.as_deref(),
        Some(config.models.anthropic.opus.as_str()),
    );
    eprintln!("archon draft: model={model} work={}", workdir.display());

    let outcome = tokio::task::spawn_blocking(
        move || -> std::result::Result<orchestrator::Outcome, String> {
            let fclient = FableClient::from_env().map_err(|e| e.to_string())?;
            let call = |p: &str, mt: u32| fclient.call(&model, p, mt);
            orchestrator::run(&call, &model, &pack, &bank, &cfg, &workdir)
                .map_err(|e| e.to_string())
        },
    )
    .await
    .map_err(|e| anyhow!("draft task panicked: {e}"))?
    .map_err(|e| anyhow!("draft run failed: {e}"))?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status": outcome.status,
            "cycles": outcome.cycles,
            "surface_to_user_in_skeleton": outcome.surface_to_user_in_skeleton,
            "chain_verified": outcome.chain_verified,
            "surfaced_defects": outcome.surfaced_defects,
            "final_words": outcome.final_draft.split_whitespace().count(),
        }))?
    );
    Ok(())
}
