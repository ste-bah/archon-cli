//! Lowering a lint source to a [`TaskGraph`]: a task directory, a workflow
//! spec, or a recorded graph (declared, reconstructed from its trace, or both).

use std::path::Path;

use anyhow::{Result, anyhow};
use archon_topology::ir::{GraphOrigin, TaskGraph};
use archon_topology::reconstruct::reconstruct_graph;
use archon_topology::trace::{TopologyPaths, read_trace};

use super::{LintSource, absolute};
use crate::command::topology_task_graph::task_graph_from_root;

pub(super) fn load_graph(cwd: &Path, source: &LintSource) -> Result<TaskGraph> {
    match source {
        LintSource::TaskFile(path) => Err(anyhow!(
            "task-file lint is file-level only and does not lower {} to a graph",
            absolute(cwd, path).display()
        )),
        LintSource::Tasks(path) => {
            let root = absolute(cwd, path);
            Ok(task_graph_from_root(&root)?)
        }
        LintSource::Spec(path) => {
            let spec = crate::command::workflow::load_spec_file(cwd, &path.display().to_string())?;
            let run_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("spec")
                .to_string();
            Ok(archon_workflow::lower_workflow_spec(&spec, run_id))
        }
        LintSource::Graph(id) => load_recorded_graph(cwd, id),
    }
}

/// A recorded graph, preferring the declared `graph.json` and falling back to
/// reconstruction from the trace.
///
/// The declared graph carries authored roles and fan-out but no observed reads;
/// the trace carries observed reads but only reconstructed structure. Where both
/// exist the declared shape wins and the trace supplies the reads it is missing,
/// which is the only combination that lets all three lints run at once.
fn load_recorded_graph(cwd: &Path, graph_id: &str) -> Result<TaskGraph> {
    let paths = TopologyPaths::for_project(cwd);
    let readout = read_trace(&paths.trace_jsonl(graph_id))?;
    let declared = paths.read_graph(graph_id)?;

    match declared {
        Some(mut graph) => {
            let observed = reconstruct_graph(
                graph_id,
                GraphOrigin::Session {
                    session_id: graph_id.to_string(),
                },
                &readout.records,
            );
            for node in &mut graph.nodes {
                if node.reads_are_known() {
                    continue;
                }
                if let Some(seen) = observed.node(&node.id) {
                    node.reads.clone_from(&seen.reads);
                }
            }
            Ok(graph)
        }
        None if readout.is_empty() => Err(anyhow!(
            "no graph.json and no trace records for graph '{graph_id}' under {}",
            paths.root().display()
        )),
        None => Ok(reconstruct_graph(
            graph_id,
            GraphOrigin::Session {
                session_id: graph_id.to_string(),
            },
            &readout.records,
        )),
    }
}
