//! Structured agent knowledge digest claims stored as Cozo learning events.

use anyhow::Result;
use cozo::DbInstance;
use serde::{Deserialize, Serialize};

use crate::models::{LearningEvent, LearningEventType};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentKnowledgeClaimRecord {
    pub claim_id: String,
    pub agent_type: String,
    pub claim_type: String,
    pub claim: String,
    pub confidence: f32,
    pub evidence_ids: Vec<String>,
    pub freshness: String,
    pub contradictions: Vec<String>,
    pub open_questions: Vec<String>,
    pub created_at: String,
}

impl AgentKnowledgeClaimRecord {
    pub fn new(
        claim_id: impl Into<String>,
        agent_type: impl Into<String>,
        claim_type: impl Into<String>,
        claim: impl Into<String>,
        confidence: f32,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            claim_id: claim_id.into(),
            agent_type: agent_type.into(),
            claim_type: claim_type.into(),
            claim: claim.into(),
            confidence: confidence.clamp(0.0, 1.0),
            evidence_ids: Vec::new(),
            freshness: "current".to_string(),
            contradictions: Vec::new(),
            open_questions: Vec::new(),
            created_at: created_at.into(),
        }
    }

    pub fn with_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self.evidence_ids.contains(&evidence_id) {
            self.evidence_ids.push(evidence_id);
        }
        self
    }

    pub fn with_freshness(mut self, freshness: impl Into<String>) -> Self {
        self.freshness = freshness.into();
        self
    }

    pub fn with_contradiction(mut self, contradiction: impl Into<String>) -> Self {
        self.contradictions.push(contradiction.into());
        self
    }

    pub fn with_open_question(mut self, open_question: impl Into<String>) -> Self {
        self.open_questions.push(open_question.into());
        self
    }
}

pub fn insert_agent_knowledge_claim(
    db: &DbInstance,
    workspace_id: &str,
    claim: &AgentKnowledgeClaimRecord,
) -> Result<LearningEvent> {
    let event = claim_to_learning_event(workspace_id, claim);
    crate::store::insert_learning_event(db, &event)?;
    Ok(event)
}

pub fn list_agent_knowledge_claims(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Vec<AgentKnowledgeClaimRecord>> {
    let events = crate::store::list_learning_events_by_type(
        db,
        LearningEventType::AgentKnowledgeClaim.as_str(),
    )?;
    let mut claims = events
        .iter()
        .filter_map(event_to_claim)
        .filter(|claim| claim.agent_type == agent_type)
        .collect::<Vec<_>>();
    claims.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(claims)
}

fn claim_to_learning_event(workspace_id: &str, claim: &AgentKnowledgeClaimRecord) -> LearningEvent {
    LearningEvent {
        event_id: claim.claim_id.clone(),
        workspace_id: workspace_id.to_string(),
        event_type: LearningEventType::AgentKnowledgeClaim,
        source_artifact_id: claim.agent_type.clone(),
        outcome_artifact_id: None,
        signal: serde_json::json!({
            "claim_id": claim.claim_id,
            "agent_type": claim.agent_type,
            "claim_type": claim.claim_type,
            "claim": claim.claim,
            "confidence": claim.confidence,
            "evidence_ids": claim.evidence_ids,
            "freshness": claim.freshness,
            "contradictions": claim.contradictions,
            "open_questions": claim.open_questions,
        }),
        confidence: claim.confidence,
        provenance_record_id: String::new(),
        created_at: claim.created_at.clone(),
    }
}

fn event_to_claim(event: &LearningEvent) -> Option<AgentKnowledgeClaimRecord> {
    if event.event_type != LearningEventType::AgentKnowledgeClaim {
        return None;
    }
    let signal = &event.signal;
    Some(AgentKnowledgeClaimRecord {
        claim_id: string_field(signal, "claim_id").unwrap_or_else(|| event.event_id.clone()),
        agent_type: string_field(signal, "agent_type")
            .unwrap_or_else(|| event.source_artifact_id.clone()),
        claim_type: string_field(signal, "claim_type").unwrap_or_else(|| "unknown".to_string()),
        claim: string_field(signal, "claim").unwrap_or_default(),
        confidence: signal
            .get("confidence")
            .and_then(|value| value.as_f64())
            .unwrap_or(event.confidence as f64)
            .clamp(0.0, 1.0) as f32,
        evidence_ids: string_vec_field(signal, "evidence_ids"),
        freshness: string_field(signal, "freshness").unwrap_or_else(|| "current".to_string()),
        contradictions: string_vec_field(signal, "contradictions"),
        open_questions: string_vec_field(signal, "open_questions"),
        created_at: event.created_at.clone(),
    })
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn string_vec_field(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-agent-knowledge-digest-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn agent_knowledge_claim_roundtrips_via_learning_events() {
        let db = test_db();
        let claim = AgentKnowledgeClaimRecord::new(
            "claim-1",
            "reviewer",
            "repeated_correction",
            "Reviewer has repeated source-weighting corrections.",
            1.2,
            "2026-05-08T12:00:00Z",
        )
        .with_evidence("ledger-1")
        .with_open_question("Should this become an agent memory?");

        let event = insert_agent_knowledge_claim(&db, "archon", &claim).unwrap();
        let claims = list_agent_knowledge_claims(&db, "reviewer").unwrap();

        assert_eq!(event.event_type, LearningEventType::AgentKnowledgeClaim);
        assert_eq!(event.confidence, 1.0);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim_id, "claim-1");
        assert_eq!(claims[0].evidence_ids, vec!["ledger-1"]);
        assert_eq!(
            claims[0].open_questions,
            vec!["Should this become an agent memory?"]
        );
    }
}
