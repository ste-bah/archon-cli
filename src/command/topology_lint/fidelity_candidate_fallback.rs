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
pub(super) struct WholeSetFallback {
    pub(super) set_established: BTreeSet<String>,
    pub(super) body_defect: BTreeSet<String>,
    pub(super) set_standings: BTreeMap<String, Standing>,
}

pub(super) async fn resolve_whole_set(
    client: &dyn WorkflowLlmClient,
    store: &VerdictStore,
    inputs: &[Cluster],
    standings: &[Vec<Standing>],
    task_texts: &BTreeMap<String, (String, PathBuf)>,
    skeleton: &SkeletonSummary,
    resume: &FreezeResume,
    candidate_task_ids: Option<&[String]>,
    candidate_body: Option<&str>,
) -> Result<WholeSetFallback> {
    let all_tasks: Vec<ClaimingTask> = task_texts
        .iter()
        .map(|(task_id, (text, _))| ClaimingTask {
            task_id: task_id.clone(),
            text: text.clone(),
        })
        .collect();
    let mut result = WholeSetFallback::default();
    for ((obligations, _), cluster_standings) in inputs.iter().zip(standings) {
        for (obligation, standing) in obligations.iter().zip(cluster_standings) {
            match standing {
                Standing::Verdict(verdict) if !verdict.necessarily_true => {}
                Standing::Contradiction { .. } => {}
                _ => continue,
            }
            let fallback_input = vec![(vec![obligation.clone()], all_tasks.clone())];
            let fallback = resolve(client, store, &fallback_input, skeleton, resume).await?;
            let full_set_standing = match fallback.standings.into_iter().flatten().next() {
                Some(Standing::Verdict(verdict)) if verdict.necessarily_true => {
                    result.set_established.insert(verdict.obligation_id.clone());
                    result
                        .set_standings
                        .insert(verdict.obligation_id.clone(), Standing::Verdict(verdict));
                    continue;
                }
                Some(standing @ Standing::Verdict(_)) => standing,
                Some(standing @ Standing::Contradiction { .. }) => standing,
                _ => continue,
            };
            let full_set_verdict = match &full_set_standing {
                Standing::Verdict(verdict) => verdict,
                Standing::Contradiction { refuted, .. } => refuted,
            };
            let obligation_id = full_set_verdict.obligation_id.clone();
            if let (Some(candidate_task_ids), Some(candidate_body)) =
                (candidate_task_ids, candidate_body)
                && candidate_task_ids.contains(&full_set_verdict.weakest_task_id)
                && !full_set_verdict.weakening_quotes.is_empty()
                && full_set_verdict
                    .weakening_quotes
                    .iter()
                    .all(|quote| candidate_body.contains(quote))
            {
                result.body_defect.insert(obligation_id.clone());
            }
            result
                .set_standings
                .insert(obligation_id, full_set_standing);
        }
    }
    Ok(result)
}
