//! Meaning compiler CLI handler.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::MeaningAction;

fn meaning_db_path() -> PathBuf {
    std::env::var_os("ARCHON_MEANING_DB_PATH")
        .or_else(|| std::env::var_os("ARCHON_KB_DB_PATH"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from(".local/share"))
                .join("archon")
                .join("docs.db")
        })
}

fn open_db() -> Result<DbInstance> {
    let db_path = meaning_db_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let path_str = db_path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("Failed to open meaning store at {path_str}: {e}"))?;
    archon_meaning::ensure_schema(&db)?;
    Ok(db)
}

pub async fn handle_meaning_command(action: MeaningAction) -> Result<()> {
    let db = open_db()?;
    match action {
        MeaningAction::Build { from } => build(&db, &from),
        MeaningAction::Samples => print_samples(&db),
        MeaningAction::Contrastive => print_pairs(&db),
        MeaningAction::Triplets => print_triplets(&db),
        MeaningAction::Export { kind } => export(&db, &kind),
    }
}

fn build(db: &DbInstance, source: &str) -> Result<()> {
    let report = match source {
        "gametheory-runs" => archon_meaning::build_from_gametheory_runs(db)?,
        "learning-events" => archon_meaning::build_from_learning_events(db)?,
        other => anyhow::bail!("unknown meaning source '{other}'"),
    };
    println!("Meaning build complete");
    println!("Source: {source}");
    println!("Events seen: {}", report.events_seen);
    println!("Samples: {}", report.samples_created);
    println!("Contrastive pairs: {}", report.pairs_created);
    println!("Triplets: {}", report.triplets_created);
    println!("Eval datasets: {}", report.datasets_created);
    Ok(())
}

fn print_samples(db: &DbInstance) -> Result<()> {
    let samples = archon_meaning::list_samples(db)?;
    for sample in &samples {
        println!(
            "{}  {}  {}  {}",
            sample.sample_id,
            sample.workspace_id,
            sample.label.as_str(),
            sample.text
        );
    }
    println!("{} samples", samples.len());
    Ok(())
}

fn print_pairs(db: &DbInstance) -> Result<()> {
    let pairs = archon_meaning::list_pairs(db)?;
    for pair in &pairs {
        println!(
            "{}  {} -> {}  anchor={}",
            pair.pair_id, pair.positive_sample_id, pair.negative_sample_id, pair.anchor_artifact_id
        );
    }
    println!("{} contrastive pairs", pairs.len());
    Ok(())
}

fn print_triplets(db: &DbInstance) -> Result<()> {
    let triplets = archon_meaning::list_triplets(db)?;
    for triplet in &triplets {
        println!(
            "{}  anchor={}  positive={}  negative={}",
            triplet.triplet_id,
            triplet.anchor_artifact_id,
            triplet.positive_sample_id,
            triplet.negative_sample_id
        );
    }
    println!("{} triplets", triplets.len());
    Ok(())
}

fn export(db: &DbInstance, kind: &str) -> Result<()> {
    match kind {
        "samples" => println!(
            "{}",
            archon_meaning::export::samples_jsonl(&archon_meaning::list_samples(db)?)?
        ),
        "triplets" => println!(
            "{}",
            archon_meaning::export::triplets_jsonl(&archon_meaning::list_triplets(db)?)?
        ),
        other => anyhow::bail!("unknown export kind '{other}' (expected samples or triplets)"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meaning_db_path_prefers_explicit_override() {
        unsafe {
            std::env::set_var("ARCHON_MEANING_DB_PATH", "/tmp/archon-meaning-test.db");
        }
        assert_eq!(
            meaning_db_path(),
            PathBuf::from("/tmp/archon-meaning-test.db")
        );
        unsafe {
            std::env::remove_var("ARCHON_MEANING_DB_PATH");
        }
    }
}
