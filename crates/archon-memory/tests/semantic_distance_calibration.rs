//! Measures what cosine distance real paraphrases actually get.
//!
//! `semantic_dedup_max_distance` decides whether two memories are merged, and
//! merging deletes. Picking it from intuition is how the first attempt shipped a
//! threshold of 0.08 that merged nothing on a real store: the unit tests used
//! hand-built vectors at 0.996 similarity, which is far tighter than anything a
//! sentence model produces for a genuine restatement.
//!
//! Ignored by default because it loads the local embedding model. Run with:
//!
//! ```text
//! cargo test -p archon-memory --test semantic_distance_calibration -- --ignored --nocapture
//! ```

use archon_memory::embedding::EmbeddingProvider;
use archon_memory::embedding::local::LocalEmbedding;

fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    1.0 - f64::from(dot / (na * nb))
}

#[test]
#[ignore = "loads the local embedding model"]
fn report_distances_for_real_restatements_and_distinct_claims() {
    let provider = LocalEmbedding::new().expect("local embedding provider");

    // Verbatim from a real store: one instruction, recorded seven ways by
    // different writers across turns.
    let restatements = [
        "Deploy region: eu-west-2 only, never us-east-1",
        "The eu-west-2-only rule is a standing rule that applies to deploys, infrastructure",
        "The user wants to use the AWS region eu-west-2, not us-east-1.",
        "The eu-west-2-only rule applies to deploys, infra provisioning, region config",
        "User requires all deploys to target eu-west-2 only, never us-east-1",
        "The user requires all deployments to target eu-west-2 only, and never us-east-1.",
    ];

    // Same subject, different claims. These must NOT merge.
    let distinct = [
        "Deploy to eu-west-2",
        "Never deploy to us-east-1",
        "Python is good for data science",
        "Run the linter before committing",
    ];

    let all: Vec<String> = restatements
        .iter()
        .chain(distinct.iter())
        .map(|s| s.to_string())
        .collect();
    let vectors = provider.embed(&all).expect("embed");

    println!("\n=== RESTATEMENTS (should merge) ===");
    let mut worst_restatement = 0.0_f64;
    for i in 0..restatements.len() {
        for j in (i + 1)..restatements.len() {
            let d = cosine_distance(&vectors[i], &vectors[j]);
            worst_restatement = worst_restatement.max(d);
            println!("{d:.4}  [{i}] x [{j}]");
        }
    }

    println!("\n=== DISTINCT (must NOT merge) ===");
    let offset = restatements.len();
    let mut best_distinct = 1.0_f64;
    for i in 0..distinct.len() {
        for j in (i + 1)..distinct.len() {
            let d = cosine_distance(&vectors[offset + i], &vectors[offset + j]);
            best_distinct = best_distinct.min(d);
            println!("{d:.4}  \"{}\" x \"{}\"", distinct[i], distinct[j]);
        }
    }

    println!("\n=== CROSS (restatement vs distinct) ===");
    let mut best_cross = 1.0_f64;
    for i in 0..restatements.len() {
        for j in 0..distinct.len() {
            let d = cosine_distance(&vectors[i], &vectors[offset + j]);
            best_cross = best_cross.min(d);
        }
    }
    println!("closest cross-group pair: {best_cross:.4}");

    println!("\n=== SUMMARY ===");
    println!(
        "worst restatement pair : {worst_restatement:.4}  (threshold must be ABOVE this to merge them)"
    );
    println!(
        "closest distinct pair  : {best_distinct:.4}  (threshold must be BELOW this to spare them)"
    );
    println!("closest cross pair     : {best_cross:.4}");
    println!(
        "=> any threshold in ({:.4}, {:.4}) separates them",
        worst_restatement,
        best_distinct.min(best_cross)
    );
}
