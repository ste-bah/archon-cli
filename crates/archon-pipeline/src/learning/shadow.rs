//! ShadowVectorSearch — adversarial contradiction detection via semantic inversion.
//! Implements REQ-LEARN-010.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DocumentClassification {
    Supporting,
    Contradicting,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Verdict {
    Supported,
    Refuted,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowSearchOptions {
    pub similarity_threshold: f64,
    pub max_results: usize,
    pub min_credibility: f64,
}

impl Default for ShadowSearchOptions {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.7,
            max_results: 10,
            min_credibility: 0.3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowDocument {
    pub id: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub credibility: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub document_id: String,
    pub content: String,
    pub similarity_to_shadow: f64,
    pub refutation_strength: f64,
    pub classification: DocumentClassification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportingEvidence {
    pub document_id: String,
    pub content: String,
    pub similarity: f64,
    pub credibility: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub claim: String,
    pub verdict: Verdict,
    pub confidence: f64,
    pub contradictions: Vec<Contradiction>,
    pub supporting: Vec<SupportingEvidence>,
}

// ---------------------------------------------------------------------------
// ShadowVectorSearch
// ---------------------------------------------------------------------------

pub struct ShadowVectorSearch {
    documents: Vec<ShadowDocument>,
}

impl ShadowVectorSearch {
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
        }
    }

    pub fn add_document(&mut self, doc: ShadowDocument) {
        self.documents.push(doc);
    }

    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Find documents that contradict the claim by searching for documents
    /// similar to the *shadow* (negated) embedding of the claim.
    pub fn find_contradictions(
        &self,
        claim_embedding: &[f32],
        opts: &ShadowSearchOptions,
    ) -> Vec<Contradiction> {
        let shadow = create_shadow_vector(claim_embedding);

        let mut results: Vec<Contradiction> = self
            .documents
            .iter()
            .filter(|doc| doc.credibility >= opts.min_credibility)
            .filter_map(|doc| {
                let sim_to_shadow = cosine_similarity(&doc.embedding, &shadow);
                if sim_to_shadow >= opts.similarity_threshold {
                    let refutation = calculate_refutation_strength(sim_to_shadow, doc.credibility);
                    let sim_to_original = cosine_similarity(&doc.embedding, claim_embedding);
                    let classification = classify_document(sim_to_original, sim_to_shadow);
                    Some(Contradiction {
                        document_id: doc.id.clone(),
                        content: doc.content.clone(),
                        similarity_to_shadow: sim_to_shadow,
                        refutation_strength: refutation,
                        classification,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by refutation strength descending.
        results.sort_by(|a, b| {
            b.refutation_strength
                .partial_cmp(&a.refutation_strength)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(opts.max_results);
        results
    }

    /// Find documents that support the claim (similar to the original embedding).
    pub fn find_supporting_evidence(
        &self,
        claim_embedding: &[f32],
        opts: &ShadowSearchOptions,
    ) -> Vec<SupportingEvidence> {
        let mut results: Vec<SupportingEvidence> = self
            .documents
            .iter()
            .filter(|doc| doc.credibility >= opts.min_credibility)
            .filter_map(|doc| {
                let sim = cosine_similarity(&doc.embedding, claim_embedding);
                if sim >= opts.similarity_threshold {
                    Some(SupportingEvidence {
                        document_id: doc.id.clone(),
                        content: doc.content.clone(),
                        similarity: sim,
                        credibility: doc.credibility,
                    })
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(opts.max_results);
        results
    }

    /// Generate a full validation report for a claim.
    pub fn generate_validation_report(
        &self,
        claim: &str,
        claim_embedding: &[f32],
        opts: &ShadowSearchOptions,
    ) -> ValidationReport {
        let contradictions = self.find_contradictions(claim_embedding, opts);
        let supporting = self.find_supporting_evidence(claim_embedding, opts);
        let verdict = determine_verdict(&contradictions, &supporting);
        let confidence = calculate_verdict_confidence(&contradictions, &supporting);

        ValidationReport {
            claim: claim.to_string(),
            verdict,
            confidence,
            contradictions,
            supporting,
        }
    }
}

impl Default for ShadowVectorSearch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Create a shadow vector by negating all components.
pub fn create_shadow_vector(embedding: &[f32]) -> Vec<f32> {
    embedding.iter().map(|v| -v).collect()
}

/// Cosine similarity between two vectors.  Returns 0.0 on degenerate inputs.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as f64) * (*y as f64))
        .sum();
    let mag_a: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Check whether a vector is L2-normalized (unit length within tolerance).
pub fn is_l2_normalized(v: &[f32]) -> bool {
    let norm: f64 = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    (norm - 1.0).abs() < 1e-5
}

/// Normalize a vector to unit length (L2).
pub fn normalize_l2(v: &[f32]) -> Vec<f32> {
    let norm: f64 = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| (*x as f64 / norm) as f32).collect()
}

/// Classify a document as supporting, contradicting, or neutral.
pub fn classify_document(
    similarity_to_original: f64,
    similarity_to_shadow: f64,
) -> DocumentClassification {
    if similarity_to_shadow > 0.7 {
        DocumentClassification::Contradicting
    } else if similarity_to_original > 0.7 {
        DocumentClassification::Supporting
    } else {
        DocumentClassification::Neutral
    }
}

fn determine_verdict(
    contradictions: &[Contradiction],
    supporting: &[SupportingEvidence],
) -> Verdict {
    let contra_strength: f64 = contradictions.iter().map(|c| c.refutation_strength).sum();
    let support_strength: f64 = supporting
        .iter()
        .map(|s| s.similarity * s.credibility)
        .sum();

    if contra_strength > support_strength * 1.5 {
        Verdict::Refuted
    } else if support_strength > contra_strength * 1.5 {
        Verdict::Supported
    } else {
        Verdict::Inconclusive
    }
}

fn calculate_verdict_confidence(
    contradictions: &[Contradiction],
    supporting: &[SupportingEvidence],
) -> f64 {
    let total_evidence = contradictions.len() + supporting.len();
    if total_evidence == 0 {
        return 0.0;
    }
    let contra_strength: f64 = contradictions.iter().map(|c| c.refutation_strength).sum();
    let support_strength: f64 = supporting
        .iter()
        .map(|s| s.similarity * s.credibility)
        .sum();
    let max_strength = contra_strength.max(support_strength);
    let total_strength = contra_strength + support_strength;
    if total_strength == 0.0 {
        return 0.0;
    }
    (max_strength / total_strength).clamp(0.0, 1.0)
}

fn calculate_refutation_strength(shadow_similarity: f64, credibility: f64) -> f64 {
    (shadow_similarity * credibility).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_shadow_vector_negates() {
        let v = vec![1.0f32, -2.0, 3.0, 0.0];
        let shadow = create_shadow_vector(&v);
        assert_eq!(shadow, vec![-1.0, 2.0, -3.0, -0.0]);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        // [1,0] and [0,1] are orthogonal -> similarity = 0
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-9, "sim={sim}");
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0f32, 0.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-9, "sim={sim}");
    }

    #[test]
    fn test_cosine_similarity_vector_and_shadow() {
        let v = vec![1.0f32, 2.0, 3.0];
        let shadow = create_shadow_vector(&v);
        let sim = cosine_similarity(&v, &shadow);
        assert!((sim - (-1.0)).abs() < 1e-9, "sim={sim}");
    }

    #[test]
    fn test_cosine_similarity_empty_or_mismatched() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn test_is_l2_normalized() {
        let unit = vec![1.0f32, 0.0, 0.0];
        assert!(is_l2_normalized(&unit));

        let non_unit = vec![2.0f32, 0.0, 0.0];
        assert!(!is_l2_normalized(&non_unit));
    }

    #[test]
    fn test_normalize_l2() {
        let v = vec![3.0f32, 4.0];
        let norm = normalize_l2(&v);
        assert!(is_l2_normalized(&norm), "norm not unit: {norm:?}");
        assert!((norm[0] as f64 - 0.6).abs() < 1e-5);
        assert!((norm[1] as f64 - 0.8).abs() < 1e-5);
    }

    #[test]
    fn test_normalize_l2_zero_vector() {
        let v = vec![0.0f32, 0.0];
        let norm = normalize_l2(&v);
        assert_eq!(norm, vec![0.0, 0.0]);
    }

    #[test]
    fn test_classify_document_thresholds() {
        assert_eq!(
            classify_document(0.3, 0.9),
            DocumentClassification::Contradicting
        );
        assert_eq!(
            classify_document(0.9, 0.3),
            DocumentClassification::Supporting
        );
        assert_eq!(
            classify_document(0.5, 0.5),
            DocumentClassification::Neutral
        );
    }

    /// Helper to build a ShadowDocument with a given embedding and credibility.
    fn make_doc(id: &str, embedding: Vec<f32>, credibility: f64) -> ShadowDocument {
        ShadowDocument {
            id: id.to_string(),
            content: format!("content of {id}"),
            embedding,
            credibility,
        }
    }

    #[test]
    fn test_find_contradictions() {
        let mut search = ShadowVectorSearch::new();

        // Claim embedding points in +x direction.
        let claim = vec![1.0f32, 0.0, 0.0];
        // A contradicting doc points in -x direction (same as shadow).
        search.add_document(make_doc("contra", vec![-1.0, 0.0, 0.0], 0.9));
        // A supporting doc points in +x direction.
        search.add_document(make_doc("support", vec![1.0, 0.0, 0.0], 0.9));
        // A neutral doc is orthogonal.
        search.add_document(make_doc("neutral", vec![0.0, 1.0, 0.0], 0.9));

        let opts = ShadowSearchOptions::default();
        let results = search.find_contradictions(&claim, &opts);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document_id, "contra");
    }

    #[test]
    fn test_find_supporting_evidence() {
        let mut search = ShadowVectorSearch::new();
        let claim = vec![1.0f32, 0.0, 0.0];

        search.add_document(make_doc("contra", vec![-1.0, 0.0, 0.0], 0.9));
        search.add_document(make_doc("support", vec![1.0, 0.0, 0.0], 0.9));
        search.add_document(make_doc("neutral", vec![0.0, 1.0, 0.0], 0.9));

        let opts = ShadowSearchOptions::default();
        let results = search.find_supporting_evidence(&claim, &opts);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document_id, "support");
    }

    #[test]
    fn test_validation_report_supported() {
        let mut search = ShadowVectorSearch::new();
        let claim = vec![1.0f32, 0.0, 0.0];

        // Multiple supporting, no contradicting
        search.add_document(make_doc("s1", vec![1.0, 0.0, 0.0], 0.9));
        search.add_document(make_doc("s2", vec![0.95, 0.05, 0.0], 0.8));

        let opts = ShadowSearchOptions {
            similarity_threshold: 0.7,
            max_results: 10,
            min_credibility: 0.3,
        };

        let report = search.generate_validation_report("test claim", &claim, &opts);
        assert_eq!(report.verdict, Verdict::Supported);
        assert!(report.confidence > 0.0);
        assert!(report.contradictions.is_empty());
        assert!(!report.supporting.is_empty());
    }

    #[test]
    fn test_validation_report_refuted() {
        let mut search = ShadowVectorSearch::new();
        let claim = vec![1.0f32, 0.0, 0.0];

        // Strong contradiction, no support
        search.add_document(make_doc("c1", vec![-1.0, 0.0, 0.0], 0.95));
        search.add_document(make_doc("c2", vec![-0.95, -0.05, 0.0], 0.9));

        let opts = ShadowSearchOptions::default();
        let report = search.generate_validation_report("false claim", &claim, &opts);
        assert_eq!(report.verdict, Verdict::Refuted);
        assert!(!report.contradictions.is_empty());
    }

    #[test]
    fn test_empty_document_store() {
        let search = ShadowVectorSearch::new();
        let claim = vec![1.0f32, 0.0, 0.0];
        let opts = ShadowSearchOptions::default();

        let contradictions = search.find_contradictions(&claim, &opts);
        assert!(contradictions.is_empty());

        let supporting = search.find_supporting_evidence(&claim, &opts);
        assert!(supporting.is_empty());

        let report = search.generate_validation_report("claim", &claim, &opts);
        assert_eq!(report.verdict, Verdict::Inconclusive);
        assert_eq!(report.confidence, 0.0);
    }
}
