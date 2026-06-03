use crate::error::WorkflowResult;
use crate::reducers::{
    Reducer, ReducerInputs, ReducerOutput, append_sources, collect_dissent, sectioned_output,
};

pub struct EvidenceWeightedReportReducer;
pub struct ClaimVoteReducer;
pub struct AdversarialFindingsMergeReducer;
pub struct CodeReviewSynthesisReducer;

impl Reducer for EvidenceWeightedReportReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        Ok(report(
            "Evidence Weighted Report",
            "combine accepted evidence while preserving dissent and failures",
            inputs,
        ))
    }
}

impl Reducer for ClaimVoteReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        let approvals = count_label(inputs, "approve") + count_label(inputs, "accepted");
        let rejections = count_label(inputs, "reject") + count_label(inputs, "fail");
        let mut body = format!(
            "Vote summary:\n\n- Approve-like claims: {approvals}\n- Reject/fail-like claims: {rejections}\n\n"
        );
        append_sources(&mut body, &inputs.accepted);
        Ok(sectioned_output(
            "Claim Vote",
            "aggregate majority and minority claim positions",
            inputs,
            body,
            collect_dissent(inputs),
        ))
    }
}

impl Reducer for AdversarialFindingsMergeReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        Ok(report(
            "Adversarial Findings Merge",
            "merge reviewer findings without dropping minority blockers",
            inputs,
        ))
    }
}

impl Reducer for CodeReviewSynthesisReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        Ok(report(
            "Code Review Synthesis",
            "combine module reviews into one implementation risk report",
            inputs,
        ))
    }
}

fn report(title: &str, purpose: &str, inputs: &ReducerInputs) -> ReducerOutput {
    let mut body = String::new();
    append_sources(&mut body, &inputs.accepted);
    sectioned_output(title, purpose, inputs, body, collect_dissent(inputs))
}

fn count_label(inputs: &ReducerInputs, label: &str) -> usize {
    inputs
        .accepted
        .iter()
        .filter(|input| input.content.to_ascii_lowercase().contains(label))
        .count()
}
