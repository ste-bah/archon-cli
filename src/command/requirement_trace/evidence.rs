//! Gathering the two halves of `Exercised`: what ran, and what it read.
//!
//! Both come from things a run already records, and neither is produced here.
//! That is the point — a report that generated its own evidence would be F1
//! with extra steps.
//!
//! - **What ran** is verifier `commands_run` evidence, the
//!   `WorkflowV2CommandRecord` list a run's final report carries. Read from a
//!   JSON file, so any run's report can be pointed at without this command
//!   having to know how results are stored.
//! - **What it read** is `TraceKind::FileRead` from the ambient trace under
//!   `.archon/topology/<graph-id>/trace.jsonl`, which the tool tap appends on
//!   every tool call.
//!
//! Absence is a fact, not a default: no evidence file and no graph mean every
//! anchor stays at `Candidate` with `NoTrace` named against it, which is what a
//! run that proved nothing looks like.

use std::path::Path;

use anyhow::{Context, Result};
use archon_knowledge::traceability::{CommandEvidence, ReadEvidence};
use archon_topology::ir::WriteTarget;
use archon_topology::trace::{TopologyPaths, TraceKind, read_trace};
use archon_workflow::v2::result::{WorkflowV2CommandRecord, WorkflowV2CommandStatus};

/// The `commands_run` / `tests_run` shape of a run's final report.
///
/// Deserialized structurally rather than as `WorkflowV2FinalReport` so that a
/// hand-written evidence file, or a report from an older build with fields this
/// one does not know, still parses. Both list fields default to empty: a report
/// that ran no commands is a legitimate report.
#[derive(Debug, Default, serde::Deserialize)]
struct EvidenceFile {
    #[serde(default)]
    commands_run: Vec<WorkflowV2CommandRecord>,
    #[serde(default)]
    tests_run: Vec<WorkflowV2CommandRecord>,
}

/// Read recorded command evidence from a run report.
///
/// `tests_run` is folded in alongside `commands_run` because a focused test is
/// a verifier by any other name, and the final report splits them.
pub(super) fn load_commands(path: &Path) -> Result<Vec<CommandEvidence>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading verifier evidence from {}", path.display()))?;
    let file: EvidenceFile = serde_json::from_str(&raw)
        .with_context(|| format!("parsing verifier evidence from {}", path.display()))?;
    Ok(file
        .commands_run
        .into_iter()
        .chain(file.tests_run)
        .map(|record| CommandEvidence {
            command: record.command,
            succeeded: record.status == WorkflowV2CommandStatus::Succeeded,
            exit_code: record.exit_code,
        })
        .collect())
}

/// Read `FileRead` observations for one recorded graph.
///
/// Only `TraceKind::FileRead` is consulted. `FileWritten` says a node produced a
/// file, which is not evidence that a verifier exercised it, and folding writes
/// in here would let a task promote its own output.
///
/// A missing trace file yields an empty list rather than an error — a graph that
/// was never run has no reads, and that is the answer.
pub(super) fn load_reads(project_root: &Path, graph_id: &str) -> Result<Vec<ReadEvidence>> {
    let paths = TopologyPaths::for_project(project_root);
    let readout = read_trace(&paths.trace_jsonl(graph_id)).with_context(|| {
        format!(
            "reading ambient trace for graph '{graph_id}' under {}",
            paths.root().display()
        )
    })?;

    let mut reads = Vec::new();
    for record in readout.records {
        if record.kind != TraceKind::FileRead {
            continue;
        }
        for target in &record.reads {
            if let WriteTarget::Path(path) = target {
                reads.push(ReadEvidence {
                    node_id: record.node_id.clone(),
                    file_path: normalize(path),
                });
            }
        }
    }
    reads.sort_by(|a, b| (&a.node_id, &a.file_path).cmp(&(&b.node_id, &b.file_path)));
    reads.dedup();
    Ok(reads)
}

/// Repository-relative, forward slashes, no `./` — the same normal form anchors
/// use, so equality between the two is meaningful.
fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests;
