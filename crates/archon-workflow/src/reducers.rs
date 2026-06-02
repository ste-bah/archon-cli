use serde::{Deserialize, Serialize};

use crate::error::WorkflowResult;
use crate::spec::ReducerKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerInput {
    pub stage_id: String,
    pub content: String,
    pub accepted: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerOutput {
    pub title: String,
    pub body: String,
    pub accepted_inputs: usize,
    pub failed_inputs: usize,
    pub dissent: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReducerRegistry;

impl ReducerRegistry {
    pub fn reduce(
        &self,
        kind: ReducerKind,
        inputs: &[ReducerInput],
    ) -> WorkflowResult<ReducerOutput> {
        let accepted_inputs = inputs.iter().filter(|input| input.accepted).count();
        let failed_inputs = inputs.iter().filter(|input| input.failed).count();
        let dissent = inputs
            .iter()
            .filter(|input| input.failed || !input.accepted)
            .map(|input| input.stage_id.clone())
            .collect();
        let mut body = format!("# {}\n\n", reducer_title(kind));
        body.push_str("## Evidence Coverage\n\n");
        body.push_str(&format!(
            "- Accepted inputs: {accepted_inputs}\n- Failed or dissenting inputs: {failed_inputs}\n\n"
        ));
        for input in inputs {
            body.push_str(&format!(
                "## Source: {}\n\n{}\n\n",
                input.stage_id, input.content
            ));
        }
        Ok(ReducerOutput {
            title: reducer_title(kind).to_string(),
            body,
            accepted_inputs,
            failed_inputs,
            dissent,
        })
    }
}

fn reducer_title(kind: ReducerKind) -> &'static str {
    match kind {
        ReducerKind::EvidenceWeightedReport => "Evidence Weighted Report",
        ReducerKind::ClaimVote => "Claim Vote",
        ReducerKind::AdversarialFindingsMerge => "Adversarial Findings Merge",
        ReducerKind::CitationReconciliation => "Citation Reconciliation",
        ReducerKind::CodeReviewSynthesis => "Code Review Synthesis",
        ReducerKind::ChapterAssembly => "Chapter Assembly",
        ReducerKind::TaskDecomposition => "Task Decomposition",
    }
}
