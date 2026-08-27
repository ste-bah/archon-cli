//! Pure set-level validation for structured task dependency contracts.

use std::collections::{BTreeMap, BTreeSet};

use crate::task_skeleton::{ConsumedArtifact, TaskSetFinding, TaskSkeleton};
use crate::task_universe::WorkflowV2DeliverableContract;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskSetEdgeAnalysis {
    pub blockers: Vec<TaskSetFinding>,
    pub information: Vec<TaskSetFinding>,
}

pub fn validate_task_set_edges(skeleton: &TaskSkeleton) -> Vec<TaskSetFinding> {
    analyze_task_set_edges(skeleton).blockers
}

pub fn validate_dependency_declarations(
    task_id: &str,
    dependencies: &[crate::task_skeleton::FrozenDependency],
) -> Vec<TaskSetFinding> {
    let mut findings = Vec::new();
    let mut producers = BTreeSet::new();
    for dependency in dependencies {
        if !producers.insert(dependency.task_id.as_str()) {
            findings.push(TaskSetFinding {
                field: "depends_on".into(),
                message: format!(
                    "task '{task_id}' declares producer '{}' more than once; merge its consumes entries into one dependency object",
                    dependency.task_id
                ),
            });
        }
        let has_consumes = !dependency.consumes.is_empty();
        if has_consumes == dependency.ordering_only {
            let remedy = if has_consumes {
                "choose exactly one: keep consumes and set ordering_only: false, or remove consumes and keep ordering_only: true"
            } else {
                "add a non-empty consumes list or set ordering_only: true"
            };
            findings.push(TaskSetFinding {
                field: "depends_on".into(),
                message: format!(
                    "task '{task_id}' depends_on '{}', but the edge declares consumes={} and ordering_only={}; {remedy}",
                    dependency.task_id, has_consumes, dependency.ordering_only
                ),
            });
        }
        for consumed in &dependency.consumes {
            if normalize_path(&consumed.artifact_path).is_empty() {
                findings.push(TaskSetFinding {
                    field: "consumes".into(),
                    message: format!(
                        "task '{task_id}' consumes an empty artifact_path from '{}'; write the exact producer artifact path",
                        dependency.task_id
                    ),
                });
            }
        }
    }
    findings.sort_by(finding_key);
    findings.dedup();
    findings
}

pub fn analyze_task_set_edges(skeleton: &TaskSkeleton) -> TaskSetEdgeAnalysis {
    let tasks: BTreeMap<_, _> = skeleton
        .tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect();
    let mut producers: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for task in &skeleton.tasks {
        for contract in &task.deliverable_contracts {
            let path = normalize_path(&contract.artifact_path);
            if !path.is_empty() {
                producers.entry(path).or_default().insert(&task.task_id);
            }
        }
    }

    let mut analysis = TaskSetEdgeAnalysis::default();
    let mut edge_count = 0usize;
    let mut consumed_edge_count = 0usize;
    for consumer in &skeleton.tasks {
        analysis.blockers.extend(validate_dependency_declarations(
            &consumer.task_id,
            &consumer.depends_on,
        ));
        for dependency in &consumer.depends_on {
            edge_count += 1;
            let has_consumes = !dependency.consumes.is_empty();
            if has_consumes == dependency.ordering_only {
                continue;
            }
            if dependency.ordering_only {
                continue;
            }
            consumed_edge_count += 1;
            let Some(producer) = tasks.get(dependency.task_id.as_str()) else {
                analysis.blockers.push(TaskSetFinding {
                    field: "depends_on".into(),
                    message: format!(
                        "task '{}' consumes from missing producer '{}'; add that producer to the skeleton or remove the edge",
                        consumer.task_id, dependency.task_id
                    ),
                });
                continue;
            };
            for consumed in &dependency.consumes {
                validate_consumed_artifact(
                    consumer.task_id.as_str(),
                    producer.task_id.as_str(),
                    consumed,
                    &producer.deliverable_contracts,
                    &producers,
                    &mut analysis,
                );
            }
        }
    }

    for producer in &skeleton.tasks {
        for blocked in &producer.blocks {
            let declared = tasks.get(blocked.as_str()).is_some_and(|consumer| {
                consumer
                    .depends_on
                    .iter()
                    .any(|dependency| dependency.task_id == producer.task_id)
            });
            if !declared {
                analysis.blockers.push(TaskSetFinding {
                    field: "blocks".into(),
                    message: format!(
                        "block-only edge '{} -> {}' has no consumer-side structured declaration; add {} to {} depends_on with a non-empty consumes list or ordering_only: true",
                        producer.task_id, blocked, producer.task_id, blocked
                    ),
                });
            }
        }
    }

    if edge_count > 0
        && analysis.blockers.is_empty()
        && consumed_edge_count == 0
        && !skeleton
            .tasks
            .iter()
            .flat_map(|task| &task.deliverable_contracts)
            .any(has_positive_data_obligation)
    {
        analysis.blockers.push(TaskSetFinding {
            field: "graph".into(),
            message: "every dependency is ordering_only and no contract declares a positive/source-bound instance set; declare at least one consumes edge or a positive/source-bound instance contract so the graph carries a falsifiable data obligation".into(),
        });
    }

    analysis.blockers.sort_by(finding_key);
    analysis.blockers.dedup();
    analysis.information.sort_by(finding_key);
    analysis.information.dedup();
    analysis
}

fn validate_consumed_artifact(
    consumer_id: &str,
    producer_id: &str,
    consumed: &ConsumedArtifact,
    contracts: &[WorkflowV2DeliverableContract],
    producers: &BTreeMap<String, BTreeSet<&str>>,
    analysis: &mut TaskSetEdgeAnalysis,
) {
    let path = normalize_path(&consumed.artifact_path);
    let Some(contract) = contracts
        .iter()
        .find(|contract| normalize_path(&contract.artifact_path) == path)
    else {
        analysis.blockers.push(TaskSetFinding {
            field: "consumes".into(),
            message: format!(
                "task '{consumer_id}' consumes '{path}' from producer '{producer_id}', but that producer declares no matching deliverable; add that exact artifact_path to the producer or correct the consumes path"
            ),
        });
        return;
    };

    if producers.get(&path).is_some_and(|owners| owners.len() > 1) {
        match record_binding(consumed) {
            None => analysis.blockers.push(TaskSetFinding {
                field: "consumes".into(),
                message: format!(
                    "task '{consumer_id}' consumes multi-writer artifact '{path}' from '{producer_id}' without a record binding; add {} to consumes or name a producer-specific artifact_path",
                    expected_record_binding(contract)
                ),
            }),
            Some((field, value)) => match producer_record_binding(contract) {
                None => analysis.blockers.push(TaskSetFinding {
                    field: "consumes".into(),
                    message: format!(
                        "task '{consumer_id}' declares {field}: {value} for multi-writer artifact '{path}', but producer '{producer_id}' declares no matching record field; add the same field to the producer contract or use a producer-specific path"
                    ),
                }),
                Some((producer_field, producer_value))
                    if field != producer_field || value != producer_value =>
                {
                    analysis.blockers.push(TaskSetFinding {
                        field: "consumes".into(),
                        message: format!(
                            "task '{consumer_id}' {field} '{value}' does not match producer field '{producer_value}' ({producer_field}) on '{producer_id}' for '{path}'; make the consumer binding exactly match the named producer"
                        ),
                    });
                }
                _ => {}
            },
        }
    }

    if let (Some(consumed_kind), producer_kind) = (consumed.kind.as_deref(), contract.kind.as_str())
        && consumed_kind != producer_kind
    {
        analysis.information.push(TaskSetFinding {
            field: "kind".into(),
            message: format!(
                "informational kind mismatch on '{path}': consumer '{consumer_id}' says '{consumed_kind}', producer '{producer_id}' says '{producer_kind}'; this informational mismatch does not block when the exact path and record binding match"
            ),
        });
    }
}

fn record_binding(consumed: &ConsumedArtifact) -> Option<(&'static str, &str)> {
    consumed
        .instance_source_records_field
        .as_deref()
        .map(|value| ("instance_source_records_field", value))
        .or_else(|| {
            consumed
                .registry_records_field
                .as_deref()
                .map(|value| ("registry_records_field", value))
        })
}

fn producer_record_binding(
    contract: &WorkflowV2DeliverableContract,
) -> Option<(&'static str, &str)> {
    contract
        .instance_source_records_field
        .as_deref()
        .map(|value| ("instance_source_records_field", value))
        .or_else(|| {
            contract
                .registry_records_field
                .as_deref()
                .map(|value| ("registry_records_field", value))
        })
}

fn expected_record_binding(contract: &WorkflowV2DeliverableContract) -> String {
    producer_record_binding(contract)
        .map(|(field, value)| format!("{field}: {value}"))
        .unwrap_or_else(|| {
            "instance_source_records_field or registry_records_field matching the producer"
                .to_string()
        })
}

fn has_positive_data_obligation(contract: &WorkflowV2DeliverableContract) -> bool {
    contract.min_instances >= 1
        || (contract.instance_artifact_field.is_some()
            && (contract.instance_source_path.is_some() || contract.registry_path.is_some())
            && (contract.instance_source_records_field.is_some()
                || contract.registry_records_field.is_some()))
}

fn normalize_path(path: &str) -> String {
    let normalized = path
        .trim()
        .trim_matches(['\'', '"', '`'])
        .replace('\\', "/");
    normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .trim_end_matches('/')
        .to_string()
}

fn finding_key(left: &TaskSetFinding, right: &TaskSetFinding) -> std::cmp::Ordering {
    (&left.field, &left.message).cmp(&(&right.field, &right.message))
}
