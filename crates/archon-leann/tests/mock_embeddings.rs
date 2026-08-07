//! The Mock provider must produce usable geometry (issue #145).
//!
//! Zero vectors give NaN cosine distance, which makes every point mutually
//! equidistant and leaves HNSW construction with nothing to prune on. These
//! tests pin the two properties the fix actually depends on -- non-degenerate
//! and deterministic -- rather than the specific numbers, which are an
//! implementation detail nobody should be asserting against.

use cozo::DbInstance;

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};

fn mock_embed(dimension: usize, texts: &[&str]) -> Vec<Vec<f32>> {
    let indexer = Indexer::new(
        DbInstance::new("mem", "", Default::default()).unwrap(),
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension,
        },
        None,
    )
    .expect("indexer creation");
    let owned = texts
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    indexer.embedder().embed(&owned).expect("mock embed")
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    1.0 - dot / (norm(a) * norm(b))
}

const CORPUS: [&str; 6] = [
    "fn main() {}",
    "fn helper() -> i32 { 42 }",
    "pub fn add(a: i32, b: i32) -> i32 { a + b }",
    "def greet(name): return name",
    "",
    "fn main() {} ",
];

/// The bug itself: no NaN, and the corpus is not one equidistant blob.
#[test]
fn mock_vectors_are_not_degenerate() {
    let vectors = mock_embed(8, &CORPUS);

    for (index, vector) in vectors.iter().enumerate() {
        assert_eq!(vector.len(), 8, "dimension not honoured for text {index}");
        let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "text {index} is not on the unit sphere: norm {norm}"
        );
    }

    let mut distances = Vec::new();
    for i in 0..vectors.len() {
        for j in (i + 1)..vectors.len() {
            let distance = cosine_distance(&vectors[i], &vectors[j]);
            assert!(
                distance.is_finite(),
                "cosine distance between {i} and {j} is not finite: {distance}"
            );
            distances.push(distance);
        }
    }
    let min = distances.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = distances.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    assert!(
        max - min > 0.1,
        "distances are effectively uniform ({min}..{max}); HNSW has nothing to prune on"
    );
}

/// Distinct texts get distinct vectors, including near-identical ones.
///
/// The last two entries of `CORPUS` differ by a single trailing space. If the
/// hash collapsed those, whole files of boilerplate would embed identically and
/// the pruning signal would vanish again on exactly the corpora that need it.
#[test]
fn distinct_texts_get_distinct_vectors() {
    let vectors = mock_embed(8, &CORPUS);
    for i in 0..vectors.len() {
        for j in (i + 1)..vectors.len() {
            assert!(
                vectors[i] != vectors[j],
                "texts {i} and {j} collapsed to the same vector"
            );
        }
    }
}

/// Deterministic across calls and across provider instances.
///
/// This is what the provider was named for, and it is what lets a test index a
/// corpus twice and compare the two runs row for row.
#[test]
fn mock_vectors_are_reproducible() {
    let first = mock_embed(16, &CORPUS);
    let second = mock_embed(16, &CORPUS);
    assert_eq!(first, second, "mock embeddings differ between instances");

    let repeated = mock_embed(16, &["fn main() {}", "fn main() {}"]);
    assert_eq!(
        repeated[0], repeated[1],
        "the same text embedded twice in one call must match"
    );
    assert_eq!(
        repeated[0], first[0],
        "the same text must embed the same way in any batch"
    );
}
