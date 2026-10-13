//! Whole-set follow-up for false candidate-cluster verdicts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Result;
use archon_workflow::fidelity_audit::{ClaimingTask, SkeletonSummary};
use archon_workflow::llm_client_port::WorkflowLlmClient;

use super::fidelity_resume::{Cluster, resolve};
use super::fidelity_store::{Standing, VerdictStore};
use crate::command::workflow_freeze_budget::FreezeResume;

#[derive(Default)]
pub(super) struct CandidateFallback {
    pub(super) set_established: BTreeSet<String>,
    pub(super) body_defect: BTreeSet<String>,
}

pub(super) async fn authorize(
    client: &dyn WorkflowLlmClient,
    store: &VerdictStore,
    inputs: &[Cluster],
    standings: &[Vec<Standing>],
    task_texts: &BTreeMap<String, (String, PathBuf)>,
    skeleton: &SkeletonSummary,
    resume: &FreezeResume,
    candidate_task_ids: Option<&[String]>,
    candidate_body: Option<&str>,
) -> Result<CandidateFallback> {
    let (Some(candidate_task_ids), Some(candidate_body)) = (candidate_task_ids, candidate_body)
    else {
        return Ok(CandidateFallback::default());
    };
    let all_tasks: Vec<ClaimingTask> = task_texts
        .iter()
        .map(|(task_id, (text, _))| ClaimingTask {
            task_id: task_id.clone(),
            text: text.clone(),
        })
        .collect();
    let mut result = CandidateFallback::default();
    for ((obligations, _), cluster_standings) in inputs.iter().zip(standings) {
        for (obligation, standing) in obligations.iter().zip(cluster_standings) {
            match standing {
                Standing::Verdict(verdict) if !verdict.necessarily_true => {}
                Standing::Contradiction { .. } => {}
                _ => continue,
            }
            let fallback_input = vec![(vec![obligation.clone()], all_tasks.clone())];
            let fallback = resolve(client, store, &fallback_input, skeleton, resume).await?;
            let full_set_verdict = match fallback.standings.into_iter().flatten().next() {
                Some(Standing::Verdict(verdict)) if verdict.necessarily_true => {
                    result.set_established.insert(verdict.obligation_id);
                    continue;
                }
                Some(Standing::Verdict(verdict)) => verdict,
                Some(Standing::Contradiction { refuted, .. }) => refuted,
                _ => continue,
            };
            if candidate_task_ids.contains(&full_set_verdict.weakest_task_id)
                && candidate_body.contains(&full_set_verdict.quoted_task_text)
            {
                result.body_defect.insert(full_set_verdict.obligation_id);
            }
        }
    }
    Ok(result)
}
