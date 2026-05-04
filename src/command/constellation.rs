//! Constellation CLI handler.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::ConstellationAction;

fn constellation_db_path() -> PathBuf {
    std::env::var_os("ARCHON_CONSTELLATION_DB_PATH")
        .or_else(|| std::env::var_os("ARCHON_MEANING_DB_PATH"))
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
    let db_path = constellation_db_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let path_str = db_path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("Failed to open constellation store at {path_str}: {e}"))?;
    archon_constellation::ensure_schema(&db)?;
    Ok(db)
}

pub async fn handle_constellation_command(action: ConstellationAction) -> Result<()> {
    let db = open_db()?;
    match action {
        ConstellationAction::Build { target } => build(&db, &target),
        ConstellationAction::Score {
            target,
            answer,
            text,
        } => score(&db, &target, answer, text),
        ConstellationAction::Drift {
            target,
            answer,
            text,
            threshold,
        } => drift(&db, &target, answer, text, threshold),
        ConstellationAction::List => list(&db),
    }
}

fn build(db: &DbInstance, target: &str) -> Result<()> {
    let report = archon_constellation::build_constellation(db, target)?;
    println!("Constellation build complete");
    println!("Target: {}", report.target);
    println!("Samples seen: {}", report.samples_seen);
    println!("Samples used: {}", report.sample_count);
    match (&report.centroid_id, report.version) {
        (Some(id), Some(version)) => {
            println!("Centroid: {id}");
            println!("Version: {version}");
        }
        _ => println!("Centroid: none"),
    }
    println!("Vector rows: {}", report.vector_rows);
    Ok(())
}

fn score(
    db: &DbInstance,
    target: &str,
    answer: Option<PathBuf>,
    text: Option<String>,
) -> Result<()> {
    let input = read_input(answer, text)?;
    let score = archon_constellation::score_text(db, target, &input)?;
    println!("Target: {}", score.target);
    println!("Centroid: {}", score.centroid_id);
    println!("Version: {}", score.version);
    println!("Similarity: {:.4}", score.similarity);
    println!("Distance: {:.4}", score.distance);
    println!("Sample count: {}", score.sample_count);
    Ok(())
}

fn drift(
    db: &DbInstance,
    target: &str,
    answer: Option<PathBuf>,
    text: Option<String>,
    threshold: f64,
) -> Result<()> {
    let input = read_input(answer, text)?;
    let report = archon_constellation::detect_drift(db, target, &input, threshold)?;
    println!("Target: {}", report.target);
    println!("Centroid: {}", report.centroid_id);
    println!("Version: {}", report.version);
    println!("Similarity: {:.4}", report.similarity);
    println!("Threshold: {:.4}", report.threshold);
    println!("Drifted: {}", report.drifted);
    println!("Reason: {}", report.reason);
    Ok(())
}

fn list(db: &DbInstance) -> Result<()> {
    let centroids = archon_constellation::list_centroids(db)?;
    for centroid in &centroids {
        println!(
            "{}  target={}  v{}  samples={}  source={}",
            centroid.centroid_id,
            centroid.target,
            centroid.version,
            centroid.sample_count,
            centroid.source_relation
        );
    }
    println!("{} centroids", centroids.len());
    Ok(())
}

fn read_input(answer: Option<PathBuf>, text: Option<String>) -> Result<String> {
    if let Some(path) = answer {
        return Ok(fs::read_to_string(path)?);
    }
    if let Some(value) = text
        && !value.trim().is_empty()
    {
        return Ok(value);
    }
    anyhow::bail!("provide --answer <file> or --text <text>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_prefers_constellation_override() {
        unsafe {
            std::env::set_var(
                "ARCHON_CONSTELLATION_DB_PATH",
                "/tmp/archon-constellation.db",
            );
        }
        assert_eq!(
            constellation_db_path(),
            PathBuf::from("/tmp/archon-constellation.db")
        );
        unsafe {
            std::env::remove_var("ARCHON_CONSTELLATION_DB_PATH");
        }
    }

    #[test]
    fn read_input_rejects_missing_sources() {
        assert!(read_input(None, None).is_err());
    }
}
