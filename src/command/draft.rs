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

    // Values needed for the post-run provenance import (the run moves pack/model/workdir).
    let section_id = pack.meta.section_id.clone();
    let model_for_import = model.clone();
    let chain_path = workdir.join("provenance.jsonl");

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

    // Promote the JSONL chain into the shared CozoDB provenance store (best-effort: a store
    // failure must not fail a completed draft). Once imported, `archon prov trace|export|
    // verify <artifact-id>` work on the FCDP artifacts.
    match import_provenance_to_store(&chain_path, &section_id, &model_for_import) {
        Ok(Some((n, final_artifact))) => eprintln!(
            "archon draft: imported {n} provenance records; trace with `archon prov trace {final_artifact}`"
        ),
        Ok(None) => {}
        Err(e) => eprintln!("archon draft: provenance store import skipped ({e:#})"),
    }

    Ok(())
}

/// Map an FCDP stage to a coarse artifact type for the provenance store.
fn artifact_type_for(stage: &str) -> &'static str {
    match stage {
        "d1-plan" => "movement-plan",
        "d15-skeleton" => "skeleton",
        "d2-assembled" => "draft",
        "revision" => "draft-revision",
        s if s.starts_with("d2-m") => "movement-draft",
        s if s.starts_with("gauntlet") || s == "regauntlet" || s == "g1-gate" => "gate-report",
        _ => "artifact",
    }
}

/// Import an FCDP JSONL provenance chain into the shared CozoDB store as archon-provenance
/// records + `DerivedFrom` edges. Returns `(record_count, final_artifact_id)`, or `None`
/// if the chain is empty. Store records use archon-provenance's chain-hash rule (so
/// `archon prov verify` works); the JSONL keeps FCDP's own hash for its self-contained verify.
fn import_provenance_to_store(
    chain_path: &Path,
    section_id: &str,
    model: &str,
) -> Result<Option<(usize, String)>> {
    use archon_provenance::record::{ProvenanceEdge, ProvenanceEdgeType, ProvenanceRecord};

    let chain = archon_draft::provenance::read_chain(chain_path)
        .with_context(|| format!("read chain {}", chain_path.display()))?;
    if chain.is_empty() {
        return Ok(None);
    }

    let db_path = crate::command::store_paths::evidence_db_path(&[
        "ARCHON_PROV_DB_PATH",
        "ARCHON_KB_DB_PATH",
    ]);
    let db = crate::command::store_paths::open_sqlite_db(&db_path, "provenance")?;
    archon_provenance::store::ensure_schema(&db)?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut prev_artifact: Option<String> = None;
    let mut prev_record_id: Option<String> = None;
    let mut prev_output_hash: Option<String> = None;
    let mut prev_chain_hash: Option<String> = None;
    let mut final_artifact = String::new();

    for (i, r) in chain.iter().enumerate() {
        // Stage-scoped, index-prefixed id: unique per record, order-revealing, portable.
        let artifact_id = format!("{section_id}#{i:02}-{}", r.stage);
        let parent_hashes: Vec<String> = prev_chain_hash.iter().cloned().collect();
        let input_hashes: Vec<String> = prev_output_hash.iter().cloned().collect();
        let parent_record_ids: Vec<String> = prev_record_id.iter().cloned().collect();

        let ch = archon_provenance::chain::chain_hash(
            &parent_hashes,
            &r.stage,
            &input_hashes,
            &r.content_sha256,
            Some("fcdp"),
            Some(model),
            &r.detail,
        );

        let record = ProvenanceRecord {
            record_id: r.record_id.clone(),
            artifact_id: artifact_id.clone(),
            artifact_type: artifact_type_for(&r.stage).to_string(),
            operation: r.stage.clone(),
            input_hashes,
            output_hash: r.content_sha256.clone(),
            parent_record_ids,
            tool_name: Some("fcdp".to_string()),
            agent_name: Some("archon-draft".to_string()),
            model: Some(model.to_string()),
            parameters_json: r.detail.clone(),
            timestamp: now.clone(),
            chain_hash: ch.clone(),
        };
        archon_provenance::store::insert_record(&db, &record)
            .with_context(|| format!("insert record {}", r.record_id))?;

        if let Some(ref prev) = prev_artifact {
            // DerivedFrom points derived → source; `archon prov trace <final>` walks it back.
            let edge = ProvenanceEdge::new(&artifact_id, prev, ProvenanceEdgeType::DerivedFrom);
            archon_provenance::store::insert_edge(&db, &edge).with_context(|| "insert edge")?;
        }

        prev_artifact = Some(artifact_id.clone());
        prev_record_id = Some(r.record_id.clone());
        prev_output_hash = Some(r.content_sha256.clone());
        prev_chain_hash = Some(ch);
        final_artifact = artifact_id;
    }

    Ok(Some((chain.len(), final_artifact)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_import_roundtrips_through_store() {
        let dir = std::env::temp_dir().join(format!("archon-draft-prov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chain = dir.join("provenance.jsonl");
        let _ = std::fs::remove_file(&chain);

        // Build a small FCDP chain via archon-draft's own recorder (distinct content →
        // distinct content hashes → distinct record ids).
        let art = dir.join("a.md");
        std::fs::write(&art, "movement plan body").unwrap();
        archon_draft::provenance::record(
            &chain,
            &art,
            "d1-plan",
            &serde_json::json!({"gates_run": []}),
        )
        .unwrap();
        std::fs::write(&art, "assembled draft body").unwrap();
        archon_draft::provenance::record(
            &chain,
            &art,
            "d2-assembled",
            &serde_json::json!({"words": 3}),
        )
        .unwrap();
        std::fs::write(&art, "revised draft body").unwrap();
        archon_draft::provenance::record(
            &chain,
            &art,
            "revision",
            &serde_json::json!({"cycle": 1}),
        )
        .unwrap();

        let db_file = dir.join("prov.db");
        // SAFETY: single-threaded test; no other thread reads the env concurrently.
        unsafe {
            std::env::set_var("ARCHON_PROV_DB_PATH", &db_file);
        }

        let (n, final_artifact) =
            import_provenance_to_store(&chain, "test-section", "claude-fable-5")
                .unwrap()
                .expect("non-empty chain");
        assert_eq!(n, 3);
        assert_eq!(final_artifact, "test-section#02-revision");

        let db = crate::command::store_paths::open_sqlite_db(&db_file, "provenance").unwrap();

        // trace the final artifact → export as W3C PROV JSON-LD → must reach the root
        let trace = archon_provenance::traverse::trace_artifact(&db, &final_artifact).unwrap();
        let jsonld = archon_provenance::export_w3c::export_trace_jsonld(&trace).to_string();
        assert!(
            jsonld.contains("test-section#00-d1-plan"),
            "trace/export must reach the root artifact: {jsonld}"
        );

        // verify runs end-to-end against the imported chain
        archon_provenance::verify::verify_artifact(&db, &final_artifact).unwrap();

        unsafe {
            std::env::remove_var("ARCHON_PROV_DB_PATH");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
