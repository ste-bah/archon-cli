//! Evidence Engine knowledge layer.
//!
//! This crate owns claim, entity, relation, source-quality, contradiction,
//! and knowledge retrieval records stored in CozoDB.

pub mod claim_extractor;
pub mod contradiction_scanner;
pub mod entity_extractor;
pub mod errors;
pub mod hybrid_retriever;
pub mod relation_inferer;
pub mod schema;
pub mod source_quality;
pub mod store;

use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use errors::Result;
use schema::{ClaimRecord, ContradictionRecord, EntityRecord, RelationRecord, SourceQualityRecord};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ProcessOptions {
    pub claims: bool,
    pub entities: bool,
    pub relations: bool,
    pub contradictions: bool,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            claims: true,
            entities: true,
            relations: true,
            contradictions: true,
        }
    }
}

impl ProcessOptions {
    pub fn from_flags(claims: bool, entities: bool, relations: bool, contradictions: bool) -> Self {
        if !(claims || entities || relations || contradictions) {
            return Self::default();
        }
        Self {
            claims,
            entities,
            relations,
            contradictions,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessReport {
    pub chunks_seen: usize,
    pub claims_created: usize,
    pub entities_created: usize,
    pub relations_created: usize,
    pub source_quality_records: usize,
    pub contradictions_created: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeStats {
    pub claims: usize,
    pub entities: usize,
    pub relations: usize,
    pub source_quality_records: usize,
    pub contradictions: usize,
}

#[derive(Clone)]
pub struct KnowledgeEngine {
    db: DbInstance,
}

impl KnowledgeEngine {
    pub fn new(db: DbInstance) -> Result<Self> {
        schema::ensure_knowledge_schema(&db)?;
        Ok(Self { db })
    }

    pub fn db(&self) -> &DbInstance {
        &self.db
    }

    pub fn process_documents(&self, options: ProcessOptions) -> Result<ProcessReport> {
        let chunks = store::list_doc_chunks(&self.db)?;
        self.process_chunks(chunks, options)
    }

    pub fn process_kb(&self, kb_id: &str, options: ProcessOptions) -> Result<ProcessReport> {
        let chunks = store::list_doc_chunks_for_kb(&self.db, kb_id)?;
        self.process_chunks(chunks, options)
    }

    pub fn process_chunks(
        &self,
        chunks: Vec<store::DocumentChunk>,
        options: ProcessOptions,
    ) -> Result<ProcessReport> {
        let mut report = ProcessReport {
            chunks_seen: chunks.len(),
            ..ProcessReport::default()
        };

        for chunk in &chunks {
            source_quality::seed_source_quality(&self.db, &chunk.document_id)?;
            report.source_quality_records += 1;

            let claims = if options.claims || options.contradictions {
                claim_extractor::extract_claims(chunk)
            } else {
                Vec::new()
            };
            if options.claims || options.contradictions {
                for claim in &claims {
                    store::insert_claim(&self.db, claim)?;
                }
                report.claims_created += claims.len();
            }

            let entities = if options.entities || options.relations {
                entity_extractor::extract_entities(chunk)
            } else {
                Vec::new()
            };
            if options.entities || options.relations {
                for entity in &entities {
                    store::insert_entity(&self.db, entity)?;
                }
                report.entities_created += entities.len();
            }

            if options.relations {
                let relations = relation_inferer::infer_relations(chunk, &entities);
                for relation in &relations {
                    store::insert_relation(&self.db, relation)?;
                }
                report.relations_created += relations.len();
            }
        }

        if options.contradictions {
            let contradictions = contradiction_scanner::scan_and_store(&self.db)?;
            report.contradictions_created = contradictions.len();
            for contradiction in &contradictions {
                if let Some(claim) = self
                    .claims()?
                    .into_iter()
                    .find(|c| c.claim_id == contradiction.left_claim_id)
                {
                    source_quality::update_from_outcome(
                        &self.db,
                        &claim.document_id,
                        source_quality::QualityOutcome::ContradictionFound,
                    )?;
                }
            }
        }

        Ok(report)
    }

    pub fn claims(&self) -> Result<Vec<ClaimRecord>> {
        store::list_claims(&self.db)
    }

    pub fn entities(&self) -> Result<Vec<EntityRecord>> {
        store::list_entities(&self.db)
    }

    pub fn relations(&self) -> Result<Vec<RelationRecord>> {
        store::list_relations(&self.db)
    }

    pub fn source_quality(&self) -> Result<Vec<SourceQualityRecord>> {
        store::list_source_quality(&self.db)
    }

    pub fn contradictions(&self) -> Result<Vec<ContradictionRecord>> {
        store::list_contradictions(&self.db)
    }

    pub fn search(
        &self,
        query: &str,
        options: &hybrid_retriever::SearchOptions,
    ) -> Result<Vec<hybrid_retriever::KnowledgeSearchResult>> {
        hybrid_retriever::search(&self.db, query, options)
    }

    pub fn stats(&self) -> Result<KnowledgeStats> {
        Ok(KnowledgeStats {
            claims: self.claims()?.len(),
            entities: self.entities()?.len(),
            relations: self.relations()?.len(),
            source_quality_records: self.source_quality()?.len(),
            contradictions: self.contradictions()?.len(),
        })
    }
}

pub(crate) fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}-{}", &digest[..24])
}

pub(crate) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_options_defaults_when_no_flags_set() {
        let opts = ProcessOptions::from_flags(false, false, false, false);
        assert!(opts.claims);
        assert!(opts.entities);
        assert!(opts.relations);
        assert!(opts.contradictions);
    }

    #[test]
    fn process_options_respects_explicit_flags() {
        let opts = ProcessOptions::from_flags(true, false, false, false);
        assert!(opts.claims);
        assert!(!opts.entities);
        assert!(!opts.relations);
        assert!(!opts.contradictions);
    }

    #[test]
    fn stable_ids_are_deterministic() {
        assert_eq!(stable_id("x", &["a", "b"]), stable_id("x", &["a", "b"]));
        assert_ne!(stable_id("x", &["a", "b"]), stable_id("x", &["a", "c"]));
    }
}
