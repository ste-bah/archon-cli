//! Tap three: a workflow run's `events.jsonl` becomes a trace.
//!
//! Owns the event-kind mapping and the replay that drives it. Unlike the other
//! two taps this one runs at completion rather than per-event, because
//! `archon-workflow` must not grow an edge onto the binary to gain a callback;
//! replaying the file it already writes gets the same records for free.

use std::path::Path;

use archon_topology::ir::GraphOrigin;
use archon_topology::reconstruct::ROOT_NODE_ID;
use archon_topology::trace::{TraceKind, TraceRecord};

use super::AmbientTrace;
use super::payload::{workflow_stage_id, workflow_stage_writes};

/// Map one workflow event onto a trace record, or `None` when it carries no
/// node-shaped meaning.
///
/// Stage identifiers live in the event's `detail` payload rather than in a
/// typed field, so this reads `detail.stage` / `detail.stage_id` and attributes
/// to the turn root when neither is present.
///
/// Every other kind — lifecycle noise, and the eleven write-coordination kinds
/// — is skipped. Skipping is deliberate rather than lossy: the trace format
/// grows by adding kinds, not by guessing at what an unmapped one means.
pub(super) fn workflow_trace_record(
    graph_id: &str,
    event: &archon_workflow::WorkflowEvent,
) -> Option<TraceRecord> {
    use archon_workflow::WorkflowEventKind as Kind;

    let kind = match event.kind {
        Kind::StageStarted => TraceKind::NodeStarted,
        Kind::StageCompleted | Kind::StageSkipped => TraceKind::NodeFinished,
        Kind::StageFailed | Kind::StageStalled => TraceKind::Retry,
        Kind::ForcedAccepted => TraceKind::GatePassed,
        Kind::Completed | Kind::Cancelled => TraceKind::NodeFinished,
        // Deliberately not a trace record. A blocking gap is a property of a
        // call's RESULT, not a lifecycle transition of its node, and the same
        // call already contributes a `StageFailed`/`StageStalled` record.
        // Mapping it too would fabricate an extra retry per gap.
        Kind::BlockingGapDetected => return None,
        _ => return None,
    };

    let node = workflow_stage_id(&event.detail).unwrap_or_else(|| ROOT_NODE_ID.to_string());
    let mut record = TraceRecord::new(event.ts.to_rfc3339(), graph_id, kind).with_node(node);
    if matches!(event.kind, Kind::StageFailed | Kind::Cancelled) {
        record = record.with_outcome(false, true);
    }
    if let Some(writes) = workflow_stage_writes(&event.detail) {
        record = record.with_writes(writes);
    }
    Some(record)
}

/// Project a whole workflow run's `events.jsonl` into a topology trace.
///
/// This is the third tap. It runs at completion rather than per-event because
/// `WorkflowEventLog::emit` lives in `archon-workflow`, which depends on
/// exactly one Archon crate (`archon-llm`) and must not grow an edge onto the
/// binary to gain a callback — that thinness is why its persistence is
/// file-based in the first place. Replaying the file it already writes gets the
/// same records with no new dependency.
///
/// Returns the number of events projected. A missing or unreadable log is not
/// an error; it means the run wrote nothing worth folding.
pub(crate) fn project_workflow_run(
    project_root: &Path,
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
) -> usize {
    let Some(trace) = AmbientTrace::open(project_root, run_id, run_id).ok() else {
        return 0;
    };

    let path = store.events_path(run_id);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return 0;
    };

    // Only complete lines. `WorkflowStore::append_event_line` writes the body
    // and the newline as two separate `write_all` calls, so a concurrent reader
    // genuinely can catch a line mid-write there — unlike our own trace, which
    // writes both in one call.
    let complete = match contents.rfind('\n') {
        Some(index) => &contents[..=index],
        None => "",
    };

    let mut records = Vec::new();
    let mut origin_run_id = run_id.to_string();
    for line in complete.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<archon_workflow::WorkflowEvent>(line) else {
            continue;
        };
        if !event.run_id.is_empty() {
            origin_run_id = event.run_id.clone();
        }
        if let Some(record) = workflow_trace_record(run_id, &event) {
            records.push(record);
        }
    }

    for record in &records {
        trace.record(record.clone());
        // Milestone 3: a `GatePassed` record is the one thing in this replay
        // that admission needs. It comes from `WorkflowEventKind::ForcedAccepted`,
        // which is how a human gate is actually cleared today.
        //
        // Note what this cannot do. `archon workflow force-accept` runs in a
        // *separate process* from the run it unblocks, so by the time this
        // replay sees the event the deciding process is gone. Replaying it here
        // makes the gate visible to a resumed or attached run in this process
        // and to the corpus, not to the original one. There is no in-process
        // producer of "gate passed" anywhere in the tree — which is exactly why
        // `GateEnforcement::WhereDeclared` is the default rather than the
        // design's literal reading. See `archon_topology::live::GateEnforcement`.
        if record.kind == TraceKind::GatePassed && !record.node_id.is_empty() {
            crate::command::topology_admission::on_gate_passed(run_id, &record.node_id);
        }
    }

    // Declare the reconstruction explicitly rather than letting the fold build
    // it. The fold's fallback origin is `Session`, and a workflow run that
    // reported itself as a session would break the corpus join on `run_id` —
    // which is the whole point of recording an origin.
    if !records.is_empty() {
        let graph = archon_topology::reconstruct::reconstruct_graph(
            run_id,
            GraphOrigin::Workflow {
                run_id: origin_run_id,
            },
            &records,
        );
        trace.declare_graph(&graph);
    }

    records.len()
}
