//! ProvenanceStore — source tracking and L-Score management.
//! Implements REQ-LEARN-009.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type SourceID = String;
pub type ProvenanceID = String;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceID,
    pub url: String,
    pub l_score: f64,
    pub embedding: Vec<f32>,
    pub metadata: HashMap<String, String>,
    pub last_verified: u64,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivationStep {
    pub description: String,
    pub confidence: f64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub id: ProvenanceID,
    pub claim: String,
    pub source_id: SourceID,
    pub evidence_text: String,
    pub derivation_steps: Vec<DerivationStep>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationPath {
    pub source_ids: Vec<SourceID>,
    pub hop_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LScoreResult {
    pub score: f64,
    pub recency_factor: f64,
    pub authority_factor: f64,
    pub corroboration_factor: f64,
    pub domain_relevance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceConfig {
    pub enforce_l_score: bool,
    pub min_l_score: f64,
    pub auto_persist: bool,
}

impl Default for ProvenanceConfig {
    fn default() -> Self {
        Self {
            enforce_l_score: true,
            min_l_score: 0.3,
            auto_persist: true,
        }
    }
}

// ---------------------------------------------------------------------------
// ProvenanceStore
// ---------------------------------------------------------------------------

pub struct ProvenanceStore {
    sources: HashMap<SourceID, Source>,
    provenances: HashMap<ProvenanceID, Provenance>,
    /// source_id -> list of provenance_ids referencing it
    source_to_provenances: HashMap<SourceID, Vec<ProvenanceID>>,
    config: ProvenanceConfig,
}

impl ProvenanceStore {
    pub fn new(config: ProvenanceConfig) -> Self {
        Self {
            sources: HashMap::new(),
            provenances: HashMap::new(),
            source_to_provenances: HashMap::new(),
            config,
        }
    }

    /// Register a new source.  Returns existing source id if URL was already registered.
    pub fn register_source(
        &mut self,
        url: &str,
        embedding: Vec<f32>,
        metadata: HashMap<String, String>,
    ) -> Result<SourceID> {
        validate_embedding(&embedding)?;

        let id = generate_source_id(url);

        if self.sources.contains_key(&id) {
            return Ok(id);
        }

        let now = now_epoch();

        // Calculate an initial L-Score with reasonable defaults.
        let initial = Self::calculate_l_score(0.0, 0.5, 1, 0.5);

        let source = Source {
            id: id.clone(),
            url: url.to_string(),
            l_score: initial.score,
            embedding,
            metadata,
            last_verified: now,
            created_at: now,
        };

        self.sources.insert(id.clone(), source);
        Ok(id)
    }

    /// Create a provenance record linking a claim to a source.
    pub fn create_provenance(
        &mut self,
        claim: &str,
        source_id: &str,
        evidence: &str,
    ) -> Result<ProvenanceID> {
        if !self.sources.contains_key(source_id) {
            bail!("source not found: {source_id}");
        }

        let id = generate_provenance_id();
        let provenance = Provenance {
            id: id.clone(),
            claim: claim.to_string(),
            source_id: source_id.to_string(),
            evidence_text: evidence.to_string(),
            derivation_steps: Vec::new(),
            created_at: now_epoch(),
        };

        self.provenances.insert(id.clone(), provenance);
        self.source_to_provenances
            .entry(source_id.to_string())
            .or_default()
            .push(id.clone());

        Ok(id)
    }

    /// Add a derivation step to an existing provenance chain.
    pub fn add_derivation_step(
        &mut self,
        provenance_id: &str,
        description: &str,
        confidence: f64,
    ) -> Result<()> {
        let prov = self
            .provenances
            .get_mut(provenance_id)
            .ok_or_else(|| anyhow::anyhow!("provenance not found: {provenance_id}"))?;

        prov.derivation_steps.push(DerivationStep {
            description: description.to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            timestamp: now_epoch(),
        });

        Ok(())
    }

    /// Multi-factor L-Score calculation.
    ///
    /// - `recency_days`: age of the source in days (0 = brand new)
    /// - `authority_weight`: 0.0-1.0 authority of the source
    /// - `corroboration_count`: number of corroborating sources
    /// - `domain_relevance`: 0.0-1.0 relevance to domain
    pub fn calculate_l_score(
        recency_days: f64,
        authority_weight: f64,
        corroboration_count: usize,
        domain_relevance: f64,
    ) -> LScoreResult {
        let recency_factor = (-recency_days / 365.0).exp();
        let authority_factor = authority_weight.clamp(0.0, 1.0);
        let corroboration_factor = (1.0 + (corroboration_count as f64).ln_1p()).min(2.0);
        let domain_relevance = domain_relevance.clamp(0.0, 1.0);

        let score = (recency_factor * 0.25
            + authority_factor * 0.35
            + corroboration_factor * 0.20
            + domain_relevance * 0.20)
            .clamp(0.0, 1.0);

        LScoreResult {
            score,
            recency_factor,
            authority_factor,
            corroboration_factor,
            domain_relevance,
        }
    }

    /// BFS through provenance chains up to `max_hops`.
    ///
    /// Starting from `start_source_id`, follows provenance records that
    /// reference the source, then looks at additional sources referenced by
    /// those provenance records' claims (via substring match of source URLs
    /// in evidence text).  This is intentionally a lightweight heuristic —
    /// full graph traversal would require an explicit link table.
    pub fn traverse_citation_path(&self, start_source_id: &str, max_hops: usize) -> CitationPath {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut path: Vec<SourceID> = Vec::new();

        queue.push_back(start_source_id.to_string());
        visited.insert(start_source_id.to_string());

        let mut hops: usize = 0;

        while !queue.is_empty() && hops < max_hops {
            let level_size = queue.len();
            for _ in 0..level_size {
                let current_id = match queue.pop_front() {
                    Some(id) => id,
                    None => break,
                };

                path.push(current_id.clone());

                // Look at provenances that reference this source.
                if let Some(prov_ids) = self.source_to_provenances.get(&current_id) {
                    for prov_id in prov_ids {
                        if let Some(prov) = self.provenances.get(prov_id) {
                            // Check if the evidence text mentions other source URLs.
                            for (sid, src) in &self.sources {
                                if *sid != current_id
                                    && !visited.contains(sid)
                                    && prov.evidence_text.contains(&src.url)
                                {
                                    visited.insert(sid.clone());
                                    queue.push_back(sid.clone());
                                }
                            }
                        }
                    }
                }
            }
            hops += 1;
        }

        // Drain remaining queued nodes that were discovered but not yet added.
        for id in &queue {
            if !path.contains(id) {
                path.push(id.clone());
            }
        }

        let hop_count = path.len().saturating_sub(1);
        CitationPath {
            source_ids: path,
            hop_count,
        }
    }

    pub fn get_source(&self, id: &str) -> Option<&Source> {
        self.sources.get(id)
    }

    pub fn get_provenance(&self, id: &str) -> Option<&Provenance> {
        self.provenances.get(id)
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub fn provenance_count(&self) -> usize {
        self.provenances.len()
    }

    pub fn config(&self) -> &ProvenanceConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn generate_source_id(url: &str) -> SourceID {
    format!("src-{}", &sha256_hex(url)[..16])
}

fn generate_provenance_id() -> ProvenanceID {
    format!("prov-{}", uuid::Uuid::new_v4())
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

fn validate_embedding(embedding: &[f32]) -> Result<()> {
    if embedding.is_empty() {
        bail!("embedding cannot be empty");
    }
    for v in embedding {
        if v.is_nan() || v.is_infinite() {
            bail!("embedding contains NaN/Inf");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_embedding() -> Vec<f32> {
        vec![0.1, 0.2, 0.3, 0.4]
    }

    fn make_store() -> ProvenanceStore {
        ProvenanceStore::new(ProvenanceConfig::default())
    }

    #[test]
    fn test_register_source_stores_and_retrieves() {
        let mut store = make_store();
        let id = store
            .register_source("https://example.com", sample_embedding(), HashMap::new())
            .unwrap();

        let src = store.get_source(&id).unwrap();
        assert_eq!(src.url, "https://example.com");
        assert_eq!(store.source_count(), 1);
    }

    #[test]
    fn test_l_score_calculation() {
        // Brand-new, mid-authority, 1 corroboration, mid-relevance
        let result = ProvenanceStore::calculate_l_score(0.0, 0.5, 1, 0.5);
        // recency = exp(0) = 1.0, authority = 0.5, corroboration = 1+ln(2) ≈ 1.693, domain = 0.5
        // score = 1.0*0.25 + 0.5*0.35 + 1.693*0.20 + 0.5*0.20
        //       = 0.25 + 0.175 + 0.3386 + 0.10 = 0.8636
        assert!(
            result.score > 0.8 && result.score < 0.9,
            "score={}",
            result.score
        );
        assert!((result.recency_factor - 1.0).abs() < 1e-6);

        // Old source (365 days) with low authority
        let old = ProvenanceStore::calculate_l_score(365.0, 0.1, 0, 0.2);
        assert!(old.score < result.score, "old source should score lower");
    }

    #[test]
    fn test_create_provenance_links_to_source() {
        let mut store = make_store();
        let sid = store
            .register_source("https://a.com", sample_embedding(), HashMap::new())
            .unwrap();
        let pid = store
            .create_provenance("Earth is round", &sid, "NASA says so")
            .unwrap();

        let prov = store.get_provenance(&pid).unwrap();
        assert_eq!(prov.source_id, sid);
        assert_eq!(prov.claim, "Earth is round");
        assert_eq!(store.provenance_count(), 1);
    }

    #[test]
    fn test_create_provenance_unknown_source() {
        let mut store = make_store();
        let result = store.create_provenance("claim", "nonexistent", "evidence");
        assert!(result.is_err());
    }

    #[test]
    fn test_derivation_step_added() {
        let mut store = make_store();
        let sid = store
            .register_source("https://b.com", sample_embedding(), HashMap::new())
            .unwrap();
        let pid = store.create_provenance("claim", &sid, "evidence").unwrap();

        store
            .add_derivation_step(&pid, "deduced from X", 0.9)
            .unwrap();
        store
            .add_derivation_step(&pid, "confirmed by Y", 0.95)
            .unwrap();

        let prov = store.get_provenance(&pid).unwrap();
        assert_eq!(prov.derivation_steps.len(), 2);
        assert_eq!(prov.derivation_steps[0].description, "deduced from X");
        assert!((prov.derivation_steps[0].confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_citation_path_traversal() {
        let mut store = make_store();

        // Create a chain: src_a evidence mentions url of src_b
        let sid_a = store
            .register_source("https://a.com", sample_embedding(), HashMap::new())
            .unwrap();
        let _sid_b = store
            .register_source("https://b.com", sample_embedding(), HashMap::new())
            .unwrap();
        let _sid_c = store
            .register_source("https://c.com", sample_embedding(), HashMap::new())
            .unwrap();

        // Provenance for a references b's url in evidence
        store
            .create_provenance("claim-a", &sid_a, "per https://b.com study")
            .unwrap();

        let path = store.traverse_citation_path(&sid_a, 10);
        assert!(path.source_ids.contains(&sid_a));
        // b should be discovered via evidence text mention
        assert!(
            path.source_ids.len() >= 2,
            "expected >=2 sources in path, got {}",
            path.source_ids.len()
        );
    }

    #[test]
    fn test_validate_embedding_rejects_nan() {
        assert!(validate_embedding(&[]).is_err());
        assert!(validate_embedding(&[f32::NAN]).is_err());
        assert!(validate_embedding(&[f32::INFINITY]).is_err());
        assert!(validate_embedding(&[0.1, 0.2]).is_ok());
    }

    #[test]
    fn test_source_id_deterministic() {
        let id1 = generate_source_id("https://same.com");
        let id2 = generate_source_id("https://same.com");
        assert_eq!(id1, id2);

        let id3 = generate_source_id("https://other.com");
        assert_ne!(id1, id3);
    }
}
