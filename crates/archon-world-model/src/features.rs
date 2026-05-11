use serde::{Deserialize, Serialize};

use crate::schema::{WorldActionKind, WorldTraceRow};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionMetadata {
    pub source_dimensions: usize,
    pub projection_dimensions: usize,
    pub projection_version: String,
}

impl ProjectionMetadata {
    pub fn new(source_dimensions: usize, projection_dimensions: usize) -> Self {
        Self {
            source_dimensions,
            projection_dimensions,
            projection_version: "world-model-projection-v1".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphContextFeatures {
    pub session_neighbor_count: usize,
    pub same_agent_prior_count: usize,
    pub same_provider_prior_count: usize,
    pub prior_plan_updates: usize,
    pub prior_memory_surfaces: usize,
    pub prior_plan_ids: Vec<String>,
    pub prior_memory_ids: Vec<String>,
}

impl GraphContextFeatures {
    pub fn compact_text(&self) -> String {
        format!(
            "graph session_neighbors={} same_agent_prior={} same_provider_prior={} prior_plans={} prior_memory={} plan_ids={} memory_ids={}",
            self.session_neighbor_count,
            self.same_agent_prior_count,
            self.same_provider_prior_count,
            self.prior_plan_updates,
            self.prior_memory_surfaces,
            self.prior_plan_ids.join("|"),
            self.prior_memory_ids.join("|")
        )
    }
}

pub fn graph_context_for_row(rows: &[WorldTraceRow], row: &WorldTraceRow) -> GraphContextFeatures {
    let mut session_neighbor_count = 0;
    let mut same_agent_prior_count = 0;
    let mut same_provider_prior_count = 0;
    let mut prior_plan_updates = 0;
    let mut prior_memory_surfaces = 0;
    let mut prior_plan_ids = Vec::new();
    let mut prior_memory_ids = Vec::new();
    for candidate in rows {
        if candidate.session_id != row.session_id || candidate.row_id == row.row_id {
            continue;
        }
        session_neighbor_count += 1;
        if candidate.created_at > row.created_at {
            continue;
        }
        if row.agent.is_some() && candidate.agent == row.agent {
            same_agent_prior_count += 1;
        }
        if row.provider.is_some() && candidate.provider == row.provider {
            same_provider_prior_count += 1;
        }
        if matches!(candidate.action_kind, WorldActionKind::PlanUpdate) {
            prior_plan_updates += 1;
            collect_evidence_ids(candidate, "plan", &mut prior_plan_ids);
        }
        if matches!(candidate.action_kind, WorldActionKind::MemorySurface) {
            prior_memory_surfaces += 1;
            collect_evidence_ids(candidate, "memory", &mut prior_memory_ids);
        }
    }
    GraphContextFeatures {
        session_neighbor_count,
        same_agent_prior_count,
        same_provider_prior_count,
        prior_plan_updates,
        prior_memory_surfaces,
        prior_plan_ids,
        prior_memory_ids,
    }
}

fn collect_evidence_ids(row: &WorldTraceRow, source: &str, target: &mut Vec<String>) {
    for evidence in &row.evidence_refs {
        if evidence.source == source && target.len() < 8 {
            target.push(evidence.id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_context_counts_prior_session_neighbors() {
        let mut first = WorldTraceRow::new("s1", WorldActionKind::PlanUpdate).with_row_id("r1");
        first.agent = Some("coder".into());
        first.provider = Some("anthropic".into());
        first
            .evidence_refs
            .push(crate::schema::EvidenceRef::new("plan", "plan-1"));
        let mut second = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r2");
        second.agent = Some("coder".into());
        second.provider = Some("anthropic".into());

        let context = graph_context_for_row(&[first, second.clone()], &second);

        assert_eq!(context.session_neighbor_count, 1);
        assert_eq!(context.same_agent_prior_count, 1);
        assert_eq!(context.same_provider_prior_count, 1);
        assert_eq!(context.prior_plan_updates, 1);
        assert_eq!(context.prior_plan_ids, vec!["plan-1"]);
    }
}
