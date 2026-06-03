use std::collections::BTreeSet;

use crate::error::WorkflowResult;
use crate::reducers::{
    Reducer, ReducerInputs, ReducerOutput, append_sources, collect_dissent, sectioned_output,
};

pub struct CitationReconciliationReducer;
pub struct ChapterAssemblyReducer;
pub struct TaskDecompositionReducer;

impl Reducer for CitationReconciliationReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        let mut citations = BTreeSet::new();
        for input in &inputs.accepted {
            for line in input.content.lines().map(str::trim) {
                if line.contains("http") || line.contains("doi:") || line.starts_with('[') {
                    citations.insert(line.to_string());
                }
            }
        }
        let mut body = String::from("Reconciled citation set:\n\n");
        if citations.is_empty() {
            body.push_str("No citation-like inputs were found.\n\n");
        } else {
            for citation in citations {
                body.push_str(&format!("- {citation}\n"));
            }
            body.push('\n');
        }
        Ok(sectioned_output(
            "Citation Reconciliation",
            "deduplicate and order citation-like evidence",
            inputs,
            body,
            collect_dissent(inputs),
        ))
    }
}

impl Reducer for ChapterAssemblyReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        let mut body = String::new();
        for (idx, input) in inputs.accepted.iter().enumerate() {
            body.push_str(&format!(
                "## Chapter {}\n\nSource `{}`\n\n{}\n\n",
                idx + 1,
                input.stage_id,
                input.content
            ));
        }
        Ok(sectioned_output(
            "Chapter Assembly",
            "assemble accepted chapter artifacts in stable order",
            inputs,
            body,
            collect_dissent(inputs),
        ))
    }
}

impl Reducer for TaskDecompositionReducer {
    fn reduce(&self, inputs: &ReducerInputs) -> WorkflowResult<ReducerOutput> {
        let mut body = String::from("Task decomposition:\n\n");
        for input in &inputs.accepted {
            body.push_str(&format!(
                "- [{}] {}\n",
                input.stage_id,
                first_task_line(input)
            ));
        }
        if inputs.accepted.is_empty() {
            body.push_str("No accepted task inputs were available.\n");
        }
        body.push('\n');
        append_sources(&mut body, &inputs.accepted);
        Ok(sectioned_output(
            "Task Decomposition",
            "turn accepted planning evidence into stable task rows",
            inputs,
            body,
            collect_dissent(inputs),
        ))
    }
}

fn first_task_line(input: &crate::reducers::ReducerInput) -> String {
    input
        .content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("empty task input")
        .trim()
        .trim_start_matches("- ")
        .chars()
        .take(180)
        .collect()
}
