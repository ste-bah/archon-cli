use serde::{Deserialize, Serialize};

use crate::error::WorkflowResult;
use crate::spec::ReducerKind;

mod assembly;
mod synthesis;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerInput {
    pub stage_id: String,
    pub content: String,
    pub accepted: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerInputs {
    pub accepted: Vec<ReducerInput>,
    pub failed: Vec<ReducerInput>,
    pub skipped: Vec<ReducerInput>,
}

impl ReducerInputs {
    pub fn from_slice(inputs: &[ReducerInput]) -> Self {
        let mut accepted = Vec::new();
        let mut failed = Vec::new();
        for input in stable_inputs(inputs) {
            if input.accepted && !input.failed {
                accepted.push(input);
            } else {
                failed.push(input);
            }
        }
        Self {
            accepted,
            failed,
            skipped: Vec::new(),
        }
    }

    pub fn total(&self) -> usize {
        self.accepted.len() + self.failed.len() + self.skipped.len()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReducerOutput {
    pub title: String,
    pub body: String,
    pub accepted_inputs: usize,
    pub failed_inputs: usize,
    pub dissent: Vec<String>,
}

pub trait Reducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput>;
}

#[derive(Debug, Clone, Default)]
pub struct ReducerRegistry;

impl ReducerRegistry {
    pub fn reduce(
        &self,
        kind: ReducerKind,
        inputs: &[ReducerInput],
    ) -> WorkflowResult<ReducerOutput> {
        let grouped = ReducerInputs::from_slice(inputs);
        match kind {
            ReducerKind::EvidenceWeightedReport => {
                synthesis::EvidenceWeightedReportReducer.reduce(&grouped)
            }
            ReducerKind::ClaimVote => synthesis::ClaimVoteReducer.reduce(&grouped),
            ReducerKind::AdversarialFindingsMerge => {
                synthesis::AdversarialFindingsMergeReducer.reduce(&grouped)
            }
            ReducerKind::CodeReviewSynthesis => {
                synthesis::CodeReviewSynthesisReducer.reduce(&grouped)
            }
            ReducerKind::CitationReconciliation => {
                assembly::CitationReconciliationReducer.reduce(&grouped)
            }
            ReducerKind::ChapterAssembly => assembly::ChapterAssemblyReducer.reduce(&grouped),
            ReducerKind::TaskDecomposition => assembly::TaskDecompositionReducer.reduce(&grouped),
        }
    }
}

pub(crate) fn stable_inputs(inputs: &[ReducerInput]) -> Vec<ReducerInput> {
    let mut values = inputs.to_vec();
    values.sort_by(|a, b| a.stage_id.cmp(&b.stage_id).then(a.content.cmp(&b.content)));
    values
}

pub(crate) fn sectioned_output(
    title: &str,
    purpose: &str,
    inputs: &ReducerInputs,
    accepted_body: String,
    dissent: Vec<String>,
) -> ReducerOutput {
    let mut body = format!("# {title}\n\n");
    body.push_str("## Evidence Coverage\n\n");
    body.push_str(&format!(
        "- Total inputs: {}\n- Accepted inputs: {}\n- Failed inputs: {}\n- Skipped inputs: {}\n- Reducer purpose: {purpose}\n\n",
        inputs.total(),
        inputs.accepted.len(),
        inputs.failed.len(),
        inputs.skipped.len()
    ));
    if inputs.accepted.is_empty() {
        body.push_str("## Accepted Evidence\n\nNo accepted inputs were available. No findings were fabricated.\n\n");
    } else {
        body.push_str("## Accepted Evidence\n\n");
        body.push_str(&accepted_body);
        body.push('\n');
    }
    body.push_str("## Dissent And Minority Findings\n\n");
    if dissent.is_empty() {
        body.push_str("No dissenting accepted inputs were detected.\n\n");
    } else {
        for item in &dissent {
            body.push_str(&format!("- {item}\n"));
        }
        body.push('\n');
    }
    body.push_str("## Failed Or Skipped Inputs\n\n");
    append_failed(&mut body, "Failed", &inputs.failed);
    append_failed(&mut body, "Skipped", &inputs.skipped);
    ReducerOutput {
        title: title.to_string(),
        body,
        accepted_inputs: inputs.accepted.len(),
        failed_inputs: inputs.failed.len() + inputs.skipped.len(),
        dissent,
    }
}

pub(crate) fn append_sources(body: &mut String, inputs: &[ReducerInput]) {
    for input in inputs {
        body.push_str(&format!(
            "### Source: {}\n\n{}\n\n",
            input.stage_id, input.content
        ));
    }
}

pub(crate) fn collect_dissent(inputs: &ReducerInputs) -> Vec<String> {
    inputs
        .accepted
        .iter()
        .filter(|input| looks_dissenting(&input.content))
        .map(|input| format!("{}: {}", input.stage_id, first_line(&input.content)))
        .collect()
}

fn append_failed(body: &mut String, label: &str, inputs: &[ReducerInput]) {
    if inputs.is_empty() {
        body.push_str(&format!("No {label} inputs.\n\n"));
        return;
    }
    for input in inputs {
        body.push_str(&format!(
            "- {label} `{}`: {}\n",
            input.stage_id,
            first_line(&input.content)
        ));
    }
    body.push('\n');
}

fn looks_dissenting(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("reject")
        || lower.contains("dissent")
        || lower.contains("blocking")
        || lower.contains("fail")
}

fn first_line(content: &str) -> String {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("empty input")
        .trim()
        .chars()
        .take(240)
        .collect()
}
