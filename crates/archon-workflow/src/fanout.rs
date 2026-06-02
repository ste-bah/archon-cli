use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::spec::StageSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanoutItem {
    pub id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanoutSchedule {
    pub max_parallelism: usize,
    pub batches: Vec<Vec<String>>,
}

pub fn extract_items(stage: &StageSpec) -> Vec<FanoutItem> {
    match stage.input.get("items") {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(idx, payload)| FanoutItem {
                id: format!("{}-{idx}", stage.id),
                payload: payload.clone(),
            })
            .collect(),
        _ => vec![FanoutItem {
            id: format!("{}-0", stage.id),
            payload: stage.input.clone(),
        }],
    }
}

pub fn schedule(items: &[FanoutItem], max_parallelism: usize) -> FanoutSchedule {
    let width = max_parallelism.max(1);
    let mut batches = Vec::new();
    for chunk in items.chunks(width) {
        batches.push(chunk.iter().map(|item| item.id.clone()).collect());
    }
    FanoutSchedule {
        max_parallelism: width,
        batches,
    }
}
