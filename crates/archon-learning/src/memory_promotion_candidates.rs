use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryPromotionCandidateRecord {
    pub candidate_id: String,
    pub agent_type: String,
    pub signal_source: String,
    pub target: String,
    pub claim: String,
    pub confidence: f64,
    pub frequency_score: f64,
    pub recency_score: f64,
    pub diversity_score: f64,
    pub evidence_quality: f64,
    pub evidence_ids: Vec<String>,
    pub proposal_required: bool,
    pub created_at: String,
}

impl MemoryPromotionCandidateRecord {
    pub fn new(
        candidate_id: impl Into<String>,
        agent_type: impl Into<String>,
        signal_source: impl Into<String>,
        target: impl Into<String>,
        claim: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            candidate_id: candidate_id.into(),
            agent_type: agent_type.into(),
            signal_source: signal_source.into(),
            target: target.into(),
            claim: claim.into(),
            confidence: 0.0,
            frequency_score: 0.0,
            recency_score: 0.0,
            diversity_score: 0.0,
            evidence_quality: 0.0,
            evidence_ids: Vec::new(),
            proposal_required: true,
            created_at: created_at.into(),
        }
    }

    pub fn with_scores(
        mut self,
        confidence: f64,
        frequency_score: f64,
        recency_score: f64,
        diversity_score: f64,
        evidence_quality: f64,
    ) -> Self {
        self.confidence = clamp_unit(confidence);
        self.frequency_score = clamp_unit(frequency_score);
        self.recency_score = clamp_unit(recency_score);
        self.diversity_score = clamp_unit(diversity_score);
        self.evidence_quality = clamp_unit(evidence_quality);
        self
    }

    pub fn with_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self.evidence_ids.contains(&evidence_id) {
            self.evidence_ids.push(evidence_id);
        }
        self
    }

    pub fn without_required_proposal(mut self) -> Self {
        self.proposal_required = false;
        self
    }
}

pub fn insert_memory_promotion_candidate(
    db: &DbInstance,
    candidate: &MemoryPromotionCandidateRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "cid".into(),
        DataValue::from(candidate.candidate_id.as_str()),
    );
    params.insert(
        "agent".into(),
        DataValue::from(candidate.agent_type.as_str()),
    );
    params.insert(
        "source".into(),
        DataValue::from(candidate.signal_source.as_str()),
    );
    params.insert("target".into(), DataValue::from(candidate.target.as_str()));
    params.insert("claim".into(), DataValue::from(candidate.claim.as_str()));
    params.insert(
        "confidence".into(),
        DataValue::from(clamp_unit(candidate.confidence)),
    );
    params.insert(
        "frequency".into(),
        DataValue::from(clamp_unit(candidate.frequency_score)),
    );
    params.insert(
        "recency".into(),
        DataValue::from(clamp_unit(candidate.recency_score)),
    );
    params.insert(
        "diversity".into(),
        DataValue::from(clamp_unit(candidate.diversity_score)),
    );
    params.insert(
        "quality".into(),
        DataValue::from(clamp_unit(candidate.evidence_quality)),
    );
    params.insert(
        "evidence".into(),
        DataValue::from(serde_json::to_string(&candidate.evidence_ids)?.as_str()),
    );
    params.insert(
        "proposal_required".into(),
        DataValue::from(candidate.proposal_required),
    );
    params.insert(
        "created".into(),
        DataValue::from(candidate.created_at.as_str()),
    );

    db.run_script(
        memory_promotion_candidate_put_script(),
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert memory_promotion_candidates failed: {e}"))?;
    Ok(())
}

pub fn get_memory_promotion_candidate(
    db: &DbInstance,
    candidate_id: &str,
) -> Result<Option<MemoryPromotionCandidateRecord>> {
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(candidate_id));
    let result = db
        .run_script(
            memory_promotion_candidate_query("candidate_id = $cid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get memory_promotion_candidate failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_candidate(row)))
}

pub fn list_memory_promotion_candidates_by_agent(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Vec<MemoryPromotionCandidateRecord>> {
    let mut params = BTreeMap::new();
    params.insert("agent".into(), DataValue::from(agent_type));
    let result = db
        .run_script(
            memory_promotion_candidate_query("agent_type = $agent"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list memory_promotion_candidates failed: {e}"))?;
    let mut records: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_candidate(row))
        .collect();
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

fn memory_promotion_candidate_put_script() -> &'static str {
    "?[candidate_id, agent_type, signal_source, target, claim, confidence, \
     frequency_score, recency_score, diversity_score, evidence_quality, \
     evidence_ids_json, proposal_required, created_at] <- [[$cid, $agent, \
     $source, $target, $claim, $confidence, $frequency, $recency, \
     $diversity, $quality, $evidence, $proposal_required, $created]] \
     :put memory_promotion_candidates { candidate_id => agent_type, \
     signal_source, target, claim, confidence, frequency_score, recency_score, \
     diversity_score, evidence_quality, evidence_ids_json, proposal_required, \
     created_at }"
}

fn memory_promotion_candidate_query(predicate: &'static str) -> &'static str {
    match predicate {
        "candidate_id = $cid" => {
            "?[candidate_id, agent_type, signal_source, target, claim, confidence, \
             frequency_score, recency_score, diversity_score, evidence_quality, \
             evidence_ids_json, proposal_required, created_at] := \
             *memory_promotion_candidates{candidate_id, agent_type, signal_source, \
             target, claim, confidence, frequency_score, recency_score, \
             diversity_score, evidence_quality, evidence_ids_json, \
             proposal_required, created_at}, candidate_id = $cid"
        }
        _ => {
            "?[candidate_id, agent_type, signal_source, target, claim, confidence, \
             frequency_score, recency_score, diversity_score, evidence_quality, \
             evidence_ids_json, proposal_required, created_at] := \
             *memory_promotion_candidates{candidate_id, agent_type, signal_source, \
             target, claim, confidence, frequency_score, recency_score, \
             diversity_score, evidence_quality, evidence_ids_json, \
             proposal_required, created_at}, agent_type = $agent"
        }
    }
}

fn row_to_candidate(row: &[DataValue]) -> MemoryPromotionCandidateRecord {
    MemoryPromotionCandidateRecord {
        candidate_id: str_col(row, 0).to_string(),
        agent_type: str_col(row, 1).to_string(),
        signal_source: str_col(row, 2).to_string(),
        target: str_col(row, 3).to_string(),
        claim: str_col(row, 4).to_string(),
        confidence: row[5].get_float().unwrap_or(0.0),
        frequency_score: row[6].get_float().unwrap_or(0.0),
        recency_score: row[7].get_float().unwrap_or(0.0),
        diversity_score: row[8].get_float().unwrap_or(0.0),
        evidence_quality: row[9].get_float().unwrap_or(0.0),
        evidence_ids: serde_json::from_str(str_col(row, 10)).unwrap_or_default(),
        proposal_required: row[11].get_bool().unwrap_or(true),
        created_at: str_col(row, 12).to_string(),
    }
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-memory-promotion-candidates-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn memory_promotion_candidate_roundtrips() {
        let db = test_db();
        let candidate = MemoryPromotionCandidateRecord::new(
            "memory-candidate-1",
            "reviewer",
            "agent_performance_ledger",
            "project_rule",
            "Prefer CozoDB for durable Archon learning state.",
            "2026-05-08T12:00:00Z",
        )
        .with_scores(0.95, 0.8, 0.7, 0.6, 0.9)
        .with_evidence("ledger-1")
        .with_evidence("shadow-eval-1");

        insert_memory_promotion_candidate(&db, &candidate).unwrap();
        let restored = get_memory_promotion_candidate(&db, "memory-candidate-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.agent_type, "reviewer");
        assert_eq!(restored.confidence, 0.95);
        assert_eq!(restored.evidence_ids, vec!["ledger-1", "shadow-eval-1"]);
        assert!(restored.proposal_required);
    }

    #[test]
    fn memory_promotion_candidates_list_by_agent() {
        let db = test_db();
        insert_memory_promotion_candidate(
            &db,
            &MemoryPromotionCandidateRecord::new(
                "memory-candidate-1",
                "planner",
                "pattern_miner",
                "agent_hint",
                "Ask before changing provider identity.",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_memory_promotion_candidate(
            &db,
            &MemoryPromotionCandidateRecord::new(
                "memory-candidate-2",
                "planner",
                "pattern_miner",
                "agent_hint",
                "Keep Anthropic Claude Code spoof headers intact.",
                "2026-05-08T12:01:00Z",
            )
            .without_required_proposal(),
        )
        .unwrap();

        let candidates = list_memory_promotion_candidates_by_agent(&db, "planner").unwrap();

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].candidate_id, "memory-candidate-2");
        assert!(!candidates[0].proposal_required);
    }
}
