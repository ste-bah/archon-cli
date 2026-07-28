use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::{LifecycleContract, support};

pub(super) fn inventory_contradicts_noop(
    contract: &LifecycleContract<'_>,
    item: &Value,
    gaps: &[Value],
    task_coverage: &[Value],
) -> bool {
    let task_ids = contract.canonical_ids_for(item);
    task_coverage.iter().any(|coverage| {
        coverage_task_ids(coverage)
            .iter()
            .any(|id| task_ids.contains(id))
            && coverage
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| {
                    !matches!(
                        status.to_ascii_lowercase().as_str(),
                        "accepted" | "complete" | "completed" | "noop" | "verified_noop"
                    )
                })
    }) || gaps
        .iter()
        .any(|gap| gap_references_item(contract, gap, item, &task_ids))
}

fn coverage_task_ids(coverage: &Value) -> Vec<String> {
    let mut ids = support::strings_of(coverage.get("canonical_task_ids"));
    ids.extend(support::strings_of(coverage.get("task_ids")));
    for key in ["canonical_task_id", "task_id"] {
        if let Some(id) = coverage.get(key).and_then(Value::as_str) {
            ids.push(id.to_string());
        }
    }
    ids
}

fn gap_references_item(
    contract: &LifecycleContract<'_>,
    gap: &Value,
    item: &Value,
    task_ids: &[String],
) -> bool {
    let gap_text = serde_json::to_string(gap)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if task_ids
        .iter()
        .any(|task_id| gap_text.contains(&task_id.to_ascii_lowercase()))
    {
        return true;
    }
    let mut references = support::strings_of(item.get("artifact_requirements"));
    references.extend(support::strings_of(item.get("deliverable_contracts")));
    if references.into_iter().any(|reference| {
        reference_tokens(&reference)
            .into_iter()
            .any(|token| gap_text.contains(&token))
    }) {
        return true;
    }
    descriptor_tokens(contract, item, task_ids)
        .intersection(&lexical_tokens(&gap_text))
        .next()
        .is_some()
}

fn reference_tokens(reference: &str) -> Vec<String> {
    let lower = reference.to_ascii_lowercase();
    let mut tokens = Vec::new();
    if lower.len() >= 8 {
        tokens.push(lower);
    }
    if let Some(name) = Path::new(reference)
        .file_name()
        .and_then(|name| name.to_str())
    {
        let name = name.to_ascii_lowercase();
        if name.len() >= 8 && !name.contains('*') {
            tokens.push(name);
        }
    }
    tokens
}

fn descriptor_tokens(
    contract: &LifecycleContract<'_>,
    item: &Value,
    task_ids: &[String],
) -> BTreeSet<String> {
    let mut descriptors = Vec::new();
    if let Some(item_id) = item
        .get("item_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
    {
        descriptors.push(item_id.to_string());
    }
    descriptors.extend(
        contract
            .task_universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
            .filter_map(|task| task.title.clone()),
    );
    let mut tokens = BTreeSet::new();
    for descriptor in descriptors {
        let words = lexical_words(&descriptor);
        tokens.extend(words.iter().filter_map(|word| {
            (word.len() >= 4 && !descriptor_stopword(word)).then_some(word.clone())
        }));
        tokens.extend(words.windows(2).filter_map(|pair| {
            let acronym = pair
                .iter()
                .filter_map(|word| word.chars().next())
                .collect::<String>();
            (acronym.len() == 2 && !acronym.chars().any(|ch| ch.is_ascii_digit()))
                .then_some(acronym)
        }));
    }
    tokens
}

fn lexical_tokens(text: &str) -> BTreeSet<String> {
    lexical_words(text).into_iter().collect()
}

fn lexical_words(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn descriptor_stopword(word: &str) -> bool {
    matches!(
        word,
        "artifact"
            | "current"
            | "implementation"
            | "latest"
            | "noop"
            | "refuted"
            | "task"
            | "verified"
    )
}
