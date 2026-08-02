//! `archon workflow lint` — the milestone 4 advisory lint suite.
//!
//! # Advisory means advisory
//!
//! Nothing here can fail a run. The command reads a graph, runs three pure
//! analyses over it, and prints what it found; it never writes, never mutates a
//! spec, and never removes an edge it thinks is spurious. The exit status is
//! success whether or not findings were reported, because a finding is a
//! question for the author, not a verdict. Enforcement is milestone 3's
//! admission layer and it stays there.
//!
//! # Three sources, because a graph comes from three places
//!
//! - `--tasks <DIR>` — a decomposed-PRD `TASK-*.md` directory. This is the only
//!   surface in the tree that declares dataflow on both sides (contracted
//!   artifacts out, named artifacts in), so it is the only one on which
//!   [`TaskGraph::classify_edges`] can conclude anything.
//! - `--spec-file <PATH>` — a `WorkflowSpec`. Carries roles and fan-out, so
//!   diamond conformance is meaningful; carries no read declarations, so the
//!   dataflow lints stay silent by the crate's unknown rule.
//! - `--graph <ID>` — a recorded graph under `.archon/topology/`, declared or
//!   reconstructed from its trace. Reads come from the `FileRead` records the
//!   tool tap emits, so coupling between concurrent nodes is visible here and
//!   nowhere else.
//!
//! Exactly one must be given. Passing none is an error naming all three rather
//! than a guess at which was meant.

mod render;

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_topology::ir::{GraphOrigin, TaskGraph};
use archon_topology::reconstruct::reconstruct_graph;
use archon_topology::trace::{TopologyPaths, read_trace};

use crate::command::workflow_live::workflow_live_task_universe::task_graph_from_root;

/// Which graph to lint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LintSource {
    /// A directory of decomposed-PRD `TASK-*.md` files.
    Tasks(PathBuf),
    /// A `WorkflowSpec` YAML file.
    Spec(PathBuf),
    /// A recorded graph id under `<project>/.archon/topology`.
    Graph(String),
}

impl LintSource {
    /// Resolve the three mutually exclusive flags into one source.
    ///
    /// Fails when zero or more than one is given. There is no default: guessing
    /// which graph the caller meant would produce a report about something they
    /// did not ask about, and a lint report is only useful when you know what
    /// it is a report *of*.
    pub(crate) fn from_flags(
        tasks: Option<&Path>,
        spec_file: Option<&Path>,
        graph: Option<&str>,
    ) -> Result<Self> {
        let mut chosen: Vec<LintSource> = Vec::new();
        if let Some(path) = tasks {
            chosen.push(LintSource::Tasks(path.to_path_buf()));
        }
        if let Some(path) = spec_file {
            chosen.push(LintSource::Spec(path.to_path_buf()));
        }
        if let Some(id) = graph {
            chosen.push(LintSource::Graph(id.to_string()));
        }
        match chosen.len() {
            1 => Ok(chosen.remove(0)),
            0 => Err(anyhow!(
                "workflow lint needs exactly one of --tasks <DIR>, --spec-file <PATH>, or --graph <ID>"
            )),
            _ => Err(anyhow!(
                "workflow lint takes exactly one of --tasks, --spec-file, or --graph; {} were given",
                chosen.len()
            )),
        }
    }
}

/// Load the named graph and render its lint report.
pub(crate) fn run_lint(cwd: &Path, source: &LintSource) -> Result<String> {
    let graph = load_graph(cwd, source)?;
    render::report(&graph, &describe(source))
}

fn describe(source: &LintSource) -> String {
    match source {
        LintSource::Tasks(path) => format!("task directory {}", path.display()),
        LintSource::Spec(path) => format!("workflow spec {}", path.display()),
        LintSource::Graph(id) => format!("recorded graph {id}"),
    }
}

fn load_graph(cwd: &Path, source: &LintSource) -> Result<TaskGraph> {
    match source {
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
            Ok(archon_topology::lower_workflow_spec(&spec, run_id))
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

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests;
