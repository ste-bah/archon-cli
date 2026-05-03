//! CLI handler for `archon gametheory` commands.

use anyhow::Result;
use cozo::DbInstance;

use archon_pipeline::gametheory;

/// Handle the `archon gametheory <situation>` command.
///
/// Opens the Cozo database, runs Tier 1 classification, persists the
/// fingerprint, and prints a summary.
pub fn handle_gametheory(situation: &str) -> Result<()> {
    let db = open_db()?;

    match gametheory::classify(&db, situation) {
        Ok(fp) => {
            println!("Game-Theory Fingerprint");
            println!("=======================");
            println!("Run ID:         {}", fp.run_id);
            println!("Primary Family: {}", fp.primary_family);
            if let Some(ref classic) = fp.nearest_classic {
                println!("Nearest Classic: {}", classic);
            }
            println!();
            println!("Axes:");
            println!("  Cooperation:    {:20} ({})", fp.cooperation.value, fp.cooperation.confidence);
            println!("  Payoff Sum:     {:20} ({})", fp.payoff_sum.value, fp.payoff_sum.confidence);
            println!("  Symmetry:       {:20} ({})", fp.symmetry.value, fp.symmetry.confidence);
            println!("  Timing:         {:20} ({})", fp.timing.value, fp.timing.confidence);
            println!("  Perfect Info:   {:20} ({})", fp.perfect_info.value, fp.perfect_info.confidence);
            println!("  Complete Info:  {:20} ({})", fp.complete_info.value, fp.complete_info.confidence);
            println!("  Cardinality:    {:20} ({})", fp.cardinality.value, fp.cardinality.confidence);
            println!("  Strategy Space: {:20} ({})", fp.strategy_space.value, fp.strategy_space.confidence);
            println!("  Horizon:        {:20} ({})", fp.horizon.value, fp.horizon.confidence);

            if !fp.shadow_games.is_empty() {
                println!();
                println!("Shadow Games:");
                for sg in &fp.shadow_games {
                    println!("  - {}", sg);
                }
            }

            if !fp.ambiguities.is_empty() {
                println!();
                println!("Ambiguities:");
                for a in &fp.ambiguities {
                    println!("  - [{}] {}", a.axis, a.note);
                }
            }

            if let Some(ref hg) = fp.hidden_game_scan {
                println!();
                println!("Hidden Game Scan: {} ({})", hg.game_name, hg.confidence);
                println!("  {}", hg.description);
            }

            println!();
            println!("Fingerprint persisted to Cozo (gt_runs, gt_fingerprints).");
            Ok(())
        }
        Err(archon_pipeline::gametheory::GameTheoryError::EmptySituation) => {
            println!("Error: an empty situation is not valid.");
            println!("Usage: archon gametheory \"<situation description>\"");
            println!("Example: archon gametheory \"Two firms simultaneously set prices.\"");
            Ok(())
        }
        Err(e) => {
            anyhow::bail!("gametheory classification failed: {e}");
        }
    }
}

fn open_db() -> Result<DbInstance> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".local/share"))
        .join("archon");
    std::fs::create_dir_all(&data_dir)?;
    let path = data_dir.join("archon-data.db");
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("Failed to open gametheory store at {path_str}: {e}"))
}
