//! Resolves fixed decomposition reports through their append-only event chain.
use anyhow::{Context, Result};
use archon_workflow::{
    WorkflowEventKind, WorkflowStore, WorkflowV2HostMethod, WorkflowV2ResultStore,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatestReport {
    pub id: String,
    pub outcome: String,
    pub superseded_id: Option<String>,
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone)]
struct ReportRef {
    id: String,
    supersedes_id: Option<String>,
    outcome: String,
    subjects: Vec<String>,
}

#[derive(Debug, Clone)]
enum ReportEvent {
    Reverified {
        report_id: String,
        supersedes_id: String,
    },
    Finalized {
        report_id: String,
    },
}

pub(crate) fn latest_report(store: &WorkflowStore, run_id: &str) -> Result<Option<LatestReport>> {
    let run_dir = store.run_dir(run_id);
    let result_store = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let records = result_store.load_call_records()?;
    let reports = records
        .iter()
        .filter(|r| r.call.method == WorkflowV2HostMethod::FinalReport)
        .map(|r| {
            let data = &r.result.data;
            ReportRef {
                id: r.call.id.clone(),
                supersedes_id: data
                    .get("supersedes_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                outcome: data
                    .get("outcome")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        if r.status == archon_workflow::WorkflowV2Status::Accepted {
                            "accepted"
                        } else {
                            "needs_remediation"
                        }
                        .into()
                    }),
                subjects: data
                    .get("subjects")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.get("task_id").and_then(Value::as_str).map(str::to_owned))
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    let event_path = store.events_path(run_id);
    if !event_path.exists() {
        return Ok(None);
    }
    let (workflow_events, _) = archon_workflow::events::read_event_log(&event_path)
        .with_context(|| format!("reading {}", event_path.display()))?;
    let mut events = Vec::new();
    for event in workflow_events {
        let chain_event = match event.kind {
            WorkflowEventKind::DecompositionReverified => Some(ReportEvent::Reverified {
                report_id: event
                    .detail
                    .get("report_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!("decomposition_reverified event has no report_id")
                    })?
                    .to_owned(),
                supersedes_id: event
                    .detail
                    .get("supersedes_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!("decomposition_reverified event has no supersedes_id")
                    })?
                    .to_owned(),
            }),
            WorkflowEventKind::DecompositionCompleted => Some(ReportEvent::Finalized {
                report_id: event
                    .detail
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("decomposition_completed event has no call_id"))?
                    .to_owned(),
            }),
            _ => None,
        };
        events.extend(chain_event);
    }
    resolve_report_chain(&reports, &events)
}

fn resolve_report_chain(
    reports: &[ReportRef],
    events: &[ReportEvent],
) -> Result<Option<LatestReport>> {
    let first_reverify = events.iter().find_map(|event| match event {
        ReportEvent::Reverified {
            report_id,
            supersedes_id,
        } => Some((report_id.as_str(), supersedes_id.as_str())),
        _ => None,
    });
    let initial_id = if let Some((report_id, supersedes_id)) = first_reverify {
        let report = reports
            .iter()
            .find(|report| report.id == report_id)
            .ok_or_else(|| {
                anyhow::anyhow!("reverify event references missing report {report_id}")
            })?;
        if report.supersedes_id.as_deref() != Some(supersedes_id) {
            anyhow::bail!(
                "reverify report {report_id} does not supersede event target {supersedes_id}"
            );
        }
        supersedes_id
    } else {
        let initial_id = events
            .iter()
            .rev()
            .find_map(|event| match event {
                ReportEvent::Finalized { report_id } => Some(report_id.as_str()),
                _ => None,
            })
            .or_else(|| {
                let originals = reports
                    .iter()
                    .filter(|report| {
                        !report.id.starts_with("reverify-") && report.supersedes_id.is_none()
                    })
                    .collect::<Vec<_>>();
                (originals.len() == 1).then(|| originals[0].id.as_str())
            });
        match initial_id {
            Some(id) => id,
            None if events.is_empty() && reports.is_empty() => return Ok(None),
            None => anyhow::bail!("no final report is named by the event chain"),
        }
    };
    let mut current = reports
        .iter()
        .find(|r| r.id == initial_id)
        .ok_or_else(|| anyhow::anyhow!("report chain root {initial_id} has no call record"))?
        .clone();
    for event in events {
        match event {
            ReportEvent::Reverified {
                report_id,
                supersedes_id,
            } => {
                if supersedes_id != &current.id {
                    anyhow::bail!(
                        "reverify report {report_id} supersedes {supersedes_id}, but current report is {}",
                        current.id
                    );
                }
                current = reports
                    .iter()
                    .find(|r| {
                        &r.id == report_id
                            && r.supersedes_id.as_deref() == Some(current.id.as_str())
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("reverify event references unlinked report {report_id}")
                    })?
                    .clone();
            }
            ReportEvent::Finalized { report_id } if current.id.starts_with("reverify-") => {
                let mut next = reports
                    .iter()
                    .find(|r| &r.id == report_id && !r.id.starts_with("reverify-"))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "decomposition completion references missing final report {report_id}"
                        )
                    })?
                    .clone();
                next.supersedes_id = Some(current.id.clone());
                current = next;
            }
            _ => {}
        }
    }
    Ok(Some(LatestReport {
        id: current.id,
        outcome: current.outcome,
        superseded_id: current.supersedes_id,
        subjects: current.subjects,
    }))
}

#[path = "workflow_decompose_latest_report_tests.rs"]
#[cfg(test)]
mod tests;
