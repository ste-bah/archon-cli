//! Agent/model trust-score automation.
//!
//! Scoring algorithm:
//! - Scores are grouped by `workspace_id + agent_key + model + task_type`.
//! - `verified_completion_count` is the number of verified completion claims
//!   for the group.
//! - `false_completion_count` is the raw number of persisted
//!   [`FalseCompletionIncident`] rows for the group.
//! - `completion_reliability` is a Laplace-smoothed beta mean:
//!   `(verified + 1) / (verified + severity_weighted_false + 2)`.
//! - False-completion severity weights are Low=0.5, Medium=1.0, High=2.0,
//!   Critical=3.0, so serious incidents reduce trust faster while the stored
//!   count remains the raw incident count.
//! - `evidence_quality` is the verified-claim ratio for the group, or 0.5 when
//!   no claims exist yet.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use cozo::DbInstance;

use crate::models::{
    AgentModelTrustScore, CompletionClaim, CompletionRunContext, FalseCompletionIncident,
    IncidentSeverity,
};
use crate::store;

pub const DEFAULT_WORKSPACE_ID: &str = "default";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TrustKey {
    workspace_id: String,
    agent_key: Option<String>,
    model: Option<String>,
    task_type: String,
}

impl TrustKey {
    fn from_claim(
        claim: &CompletionClaim,
        contexts: &BTreeMap<String, CompletionRunContext>,
    ) -> Self {
        Self::from_parts(
            &claim.run_id,
            claim.agent_key.clone(),
            claim.model.clone(),
            claim.task_type.clone(),
            contexts,
        )
    }

    fn from_incident(
        incident: &FalseCompletionIncident,
        contexts: &BTreeMap<String, CompletionRunContext>,
    ) -> Self {
        Self::from_parts(
            &incident.run_id,
            incident.agent_key.clone(),
            incident.model.clone(),
            incident.task_type.clone(),
            contexts,
        )
    }

    fn from_parts(
        run_id: &str,
        agent_key: Option<String>,
        model: Option<String>,
        task_type: String,
        contexts: &BTreeMap<String, CompletionRunContext>,
    ) -> Self {
        let context = contexts.get(run_id);
        Self {
            workspace_id: context
                .map(|ctx| ctx.workspace_id.clone())
                .unwrap_or_else(|| DEFAULT_WORKSPACE_ID.to_string()),
            agent_key: context.and_then(|ctx| ctx.agent_key.clone()).or(agent_key),
            model: context.and_then(|ctx| ctx.model.clone()).or(model),
            task_type,
        }
    }
}

/// Recompute and persist trust scores for every observed agent/model/task group.
pub fn recompute_all_trust_scores(db: &DbInstance) -> Result<Vec<AgentModelTrustScore>> {
    let claims = store::get_all_completion_claims(db)?;
    let incidents = store::get_all_incidents(db)?;
    let contexts = completion_contexts(db)?;
    let keys = collect_keys(&claims, &incidents, &contexts);
    persist_scores_for_keys(db, &claims, &incidents, &contexts, keys)
}

/// Recompute and persist scores for groups touched by one completion verify run.
pub fn recompute_trust_scores_for_run(
    db: &DbInstance,
    run_id: &str,
) -> Result<Vec<AgentModelTrustScore>> {
    let claims = store::get_all_completion_claims(db)?;
    let incidents = store::get_all_incidents(db)?;
    let contexts = completion_contexts(db)?;
    let keys = collect_keys_for_run(run_id, &claims, &incidents, &contexts);
    persist_scores_for_keys(db, &claims, &incidents, &contexts, keys)
}

fn completion_contexts(db: &DbInstance) -> Result<BTreeMap<String, CompletionRunContext>> {
    Ok(store::get_all_completion_run_contexts(db)?
        .into_iter()
        .map(|ctx| (ctx.run_id.clone(), ctx))
        .collect())
}

fn collect_keys(
    claims: &[CompletionClaim],
    incidents: &[FalseCompletionIncident],
    contexts: &BTreeMap<String, CompletionRunContext>,
) -> BTreeSet<TrustKey> {
    let mut keys = BTreeSet::new();
    keys.extend(
        claims
            .iter()
            .map(|claim| TrustKey::from_claim(claim, contexts)),
    );
    keys.extend(
        incidents
            .iter()
            .map(|incident| TrustKey::from_incident(incident, contexts)),
    );
    keys
}

fn collect_keys_for_run(
    run_id: &str,
    claims: &[CompletionClaim],
    incidents: &[FalseCompletionIncident],
    contexts: &BTreeMap<String, CompletionRunContext>,
) -> BTreeSet<TrustKey> {
    let mut keys = BTreeSet::new();
    keys.extend(
        claims
            .iter()
            .filter(|claim| claim.run_id == run_id)
            .map(|claim| TrustKey::from_claim(claim, contexts)),
    );
    keys.extend(
        incidents
            .iter()
            .filter(|incident| incident.run_id == run_id)
            .map(|incident| TrustKey::from_incident(incident, contexts)),
    );
    keys
}

fn persist_scores_for_keys(
    db: &DbInstance,
    claims: &[CompletionClaim],
    incidents: &[FalseCompletionIncident],
    contexts: &BTreeMap<String, CompletionRunContext>,
    keys: BTreeSet<TrustKey>,
) -> Result<Vec<AgentModelTrustScore>> {
    let mut scores = Vec::new();
    for key in keys {
        let score = compute_score(&key, claims, incidents, contexts);
        store::insert_trust_score(db, &score)?;
        scores.push(score);
    }
    Ok(scores)
}

fn compute_score(
    key: &TrustKey,
    claims: &[CompletionClaim],
    incidents: &[FalseCompletionIncident],
    contexts: &BTreeMap<String, CompletionRunContext>,
) -> AgentModelTrustScore {
    let matching_claims: Vec<&CompletionClaim> = claims
        .iter()
        .filter(|claim| claim_matches(key, claim, contexts))
        .collect();
    let matching_incidents: Vec<&FalseCompletionIncident> = incidents
        .iter()
        .filter(|incident| incident_matches(key, incident, contexts))
        .collect();

    let verified = matching_claims
        .iter()
        .filter(|claim| claim.verified)
        .count() as u32;
    let false_count = matching_incidents.len() as u32;
    let severity_weighted_false: f32 = matching_incidents
        .iter()
        .map(|incident| severity_weight(&incident.severity))
        .sum();
    let reliability = ((verified as f32 + 1.0) / (verified as f32 + severity_weighted_false + 2.0))
        .clamp(0.0, 1.0);
    let evidence_quality = if matching_claims.is_empty() {
        0.5
    } else {
        verified as f32 / matching_claims.len() as f32
    };

    AgentModelTrustScore {
        score_id: score_id(key),
        workspace_id: key.workspace_id.clone(),
        agent_key: key.agent_key.clone(),
        model: key.model.clone(),
        task_type: key.task_type.clone(),
        completion_reliability: reliability,
        evidence_quality,
        false_completion_count: false_count,
        verified_completion_count: verified,
        last_updated: chrono::Utc::now().to_rfc3339(),
    }
}

fn claim_matches(
    key: &TrustKey,
    claim: &CompletionClaim,
    contexts: &BTreeMap<String, CompletionRunContext>,
) -> bool {
    key == &TrustKey::from_claim(claim, contexts)
}

fn incident_matches(
    key: &TrustKey,
    incident: &FalseCompletionIncident,
    contexts: &BTreeMap<String, CompletionRunContext>,
) -> bool {
    key == &TrustKey::from_incident(incident, contexts)
}

fn severity_weight(severity: &IncidentSeverity) -> f32 {
    match severity {
        IncidentSeverity::Low => 0.5,
        IncidentSeverity::Medium => 1.0,
        IncidentSeverity::High => 2.0,
        IncidentSeverity::Critical => 3.0,
    }
}

fn score_id(key: &TrustKey) -> String {
    let mut stable = BTreeMap::new();
    stable.insert("workspace_id", key.workspace_id.as_str());
    stable.insert("agent_key", key.agent_key.as_deref().unwrap_or(""));
    stable.insert("model", key.model.as_deref().unwrap_or(""));
    stable.insert("task_type", key.task_type.as_str());
    let json = serde_json::to_string(&stable).unwrap_or_default();
    format!("trust-{:016x}", fnv1a64(json.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CompletionClaimKind, CompletionState, EvidenceKind, FalseCompletionIncident,
    };

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-completion-trust-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    fn claim(id: &str, verified: bool) -> CompletionClaim {
        CompletionClaim {
            claim_id: id.into(),
            run_id: "run-1".into(),
            agent_key: Some("agent-a".into()),
            model: Some("model-a".into()),
            task_type: "coding".into(),
            claim_text: "done".into(),
            claim_kind: CompletionClaimKind::Done,
            required_evidence: vec![],
            linked_evidence_ids: vec![],
            verified,
            contradiction_ids: vec![],
            created_at: "2026-05-04T00:00:00Z".into(),
        }
    }

    fn incident(severity: IncidentSeverity) -> FalseCompletionIncident {
        FalseCompletionIncident {
            incident_id: "inc-1".into(),
            run_id: "run-1".into(),
            agent_key: Some("agent-a".into()),
            model: Some("model-a".into()),
            task_type: "coding".into(),
            claimed_state: "Done".into(),
            actual_state: CompletionState::Failed,
            missing_evidence: vec![EvidenceKind::TestRun],
            contradiction_ids: vec![],
            user_correction: None,
            severity,
            learning_event_id: "le-1".into(),
            created_at: "2026-05-04T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_scoring_math_uses_verified_and_severity_weighted_false_counts() {
        let claims = vec![
            claim("cl-1", true),
            claim("cl-2", true),
            claim("cl-3", false),
        ];
        let incidents = vec![incident(IncidentSeverity::High)];
        let contexts = BTreeMap::new();
        let key = TrustKey::from_claim(&claims[0], &contexts);

        let score = compute_score(&key, &claims, &incidents, &contexts);

        assert_eq!(score.verified_completion_count, 2);
        assert_eq!(score.false_completion_count, 1);
        assert!((score.completion_reliability - 0.5).abs() < 0.0001);
        assert!((score.evidence_quality - (2.0 / 3.0)).abs() < 0.0001);
    }

    #[test]
    fn test_score_id_is_stable_for_same_key() {
        let key = TrustKey {
            workspace_id: DEFAULT_WORKSPACE_ID.into(),
            agent_key: Some("agent-a".into()),
            model: Some("model-a".into()),
            task_type: "coding".into(),
        };
        assert_eq!(score_id(&key), score_id(&key));
    }

    #[test]
    fn test_recompute_persists_trust_score_source_of_truth() {
        let db = test_db();
        let verified = claim("cl-verified", true);
        let unverified = claim("cl-unverified", false);
        let incident = incident(IncidentSeverity::High);
        let context = CompletionRunContext {
            run_id: "run-1".into(),
            workspace_id: "workspace-a".into(),
            agent_key: Some("agent-a".into()),
            model: Some("model-a".into()),
            updated_at: "2026-05-04T00:00:00Z".into(),
        };

        store::insert_completion_run_context(&db, &context).unwrap();
        store::insert_completion_claim(&db, &verified).unwrap();
        store::insert_completion_claim(&db, &unverified).unwrap();
        store::insert_false_completion_incident(&db, &incident).unwrap();

        let scores = recompute_all_trust_scores(&db).unwrap();
        assert_eq!(scores.len(), 1);

        let persisted = store::find_trust_scores(&db, Some("agent-a"), Some("model-a")).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].workspace_id, "workspace-a");
        assert_eq!(persisted[0].verified_completion_count, 1);
        assert_eq!(persisted[0].false_completion_count, 1);
        assert!((persisted[0].completion_reliability - 0.4).abs() < 0.0001);
        assert!((persisted[0].evidence_quality - 0.5).abs() < 0.0001);
    }
}
