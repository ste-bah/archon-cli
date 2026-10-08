//! `archon workflow lint` — the milestone 4 advisory lint suite.
//!
//! # Evaluation is separate from workflow admission
//!
//! The command reads inputs, runs pure analyses, and prints what it found; it
//! never writes or mutates a task spec. Policy findings follow startup
//! `workflow.gate_mode`: observe records them and exits zero, while enforce
//! exits non-zero. Operational input failures are errors in every mode. None of
//! these outcomes is consulted by workflow run admission.
//!
//! # Three sources, because a graph comes from three places
//!
//! - `--tasks <DIR>` — a decomposed-PRD `TASK-*.md` directory. This is the only
//!   surface in the tree that declares dataflow on both sides (contracted
//!   artifacts out, named artifacts in), so it is the only one on which
//!   [`TaskGraph::classify_edges`](archon_topology::ir::TaskGraph::classify_edges)
//!   can conclude anything.
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

mod candidate;
mod contracts;
mod coverage;
mod declarations;
mod fences;
mod fidelity;
mod fidelity_critic;
mod fidelity_resume;
mod fidelity_store;
mod fidelity_waivers;
mod focused_test_files;
mod graph_load;
mod owner_coverage;
mod preflight;
mod render;
mod repository_claims;
mod repository_observations;
mod scope_declarations;
mod task_file;
mod task_set;
mod tool_obligations;
mod unowned_obligations;

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

pub(crate) use candidate::evaluate_task_file_candidate;
pub(crate) use fences::outer_fence_with_surrounding_text;
pub(crate) use fences::unwrap_outer_fence;
pub(crate) use fidelity::{
    audit_task_file_candidate, evaluate_lint_with_fidelity, evaluate_lint_with_fidelity_resumable,
};
pub(crate) use fidelity_resume::resumable_exit;
pub(crate) use fidelity_waivers::{record_waivers, recorded_waivers, waivers_from_flags};
use graph_load::load_graph;
pub(crate) use owner_coverage::skeleton_defects as skeleton_owner_defects;

/// Which graph to lint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LintSource {
    /// Exactly one decomposed-PRD `TASK-*.md` file.
    TaskFile(PathBuf),
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
        task_file: Option<&Path>,
        tasks: Option<&Path>,
        spec_file: Option<&Path>,
        graph: Option<&str>,
    ) -> Result<Self> {
        let mut chosen: Vec<LintSource> = Vec::new();
        if let Some(path) = task_file {
            chosen.push(LintSource::TaskFile(path.to_path_buf()));
        }
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
                "workflow lint needs exactly one of --task-file <PATH>, --tasks <DIR>, --spec-file <PATH>, or --graph <ID>"
            )),
            _ => Err(anyhow!(
                "workflow lint takes exactly one of --task-file, --tasks, --spec-file, or --graph; {} were given",
                chosen.len()
            )),
        }
    }
}

/// One lint reads one version of the task set's frozen chain (Issue 294).
pub(super) fn chain_read(
    cwd: &Path,
    source: &LintSource,
) -> Result<Option<crate::command::workflow_task_set::ChainRead>> {
    let root = match source {
        LintSource::TaskFile(path) => absolute(cwd, path).parent().map(Path::to_path_buf),
        LintSource::Tasks(path) => Some(absolute(cwd, path)),
        LintSource::Spec(_) | LintSource::Graph(_) => None,
    };
    (root.map(|root| crate::command::workflow_task_set::ChainRead::of(cwd, &root))).transpose()
}

/// Load the named graph and render its lint report.
///
/// The fourth section, requirement coverage, is not a graph analysis: it
/// compares the task files' `implements:` claims against the requirement IDs of
/// the PRD they name, so it takes the task directory rather than the lowered
/// graph, and it only has anything to say for `--tasks`. It is advisory like the
/// other three — an unclaimed requirement is reported, never raised.
fn run_lint_with_mode(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
) -> Result<String> {
    let _read = chain_read(cwd, source)?;
    if let LintSource::TaskFile(path) = source {
        return Ok(task_file::inspect(cwd, path, mode).report);
    }
    let tasks_root = match source {
        LintSource::TaskFile(path) => absolute(cwd, path).parent().map(Path::to_path_buf),
        LintSource::Tasks(path) => Some(absolute(cwd, path)),
        LintSource::Spec(_) | LintSource::Graph(_) => None,
    };
    // A task set that will not lower to a graph still gets the file-level
    // sections. Refusing everything on one malformed file is how a real corpus
    // went unexamined: a single `artifact_paths` typo took the whole report
    // down, so the reader learned nothing about the other fourteen tasks and
    // the capability report never ran at all. The graph error is reported
    // first, in full, and then the analyses that do not need a graph continue.
    let graph = match load_graph(cwd, source) {
        Ok(graph) => Some(graph),
        Err(error) => {
            if tasks_root.is_none() {
                return Err(error);
            }
            None
        }
    };
    let mut out = match &graph {
        Some(graph) => render::report(graph, &describe(source))?,
        None => format!(
            "# topology lint — {}\n\n## graph\n  NOT ANALYSED: this task set does not \
             lower to a graph.\n  {}\n  The file-level sections below still ran.\n",
            describe(source),
            load_graph(cwd, source).unwrap_err()
        ),
    };
    out.push_str(&coverage::section(tasks_root.as_deref()));
    // Fifth section, and like coverage it is not a graph analysis: it asks
    // whether each task's frontmatter accounts for the commands that task
    // declares it will run. Advisory for the same reason the others are —
    // reported so the author can settle it, never raised.
    out.push_str(&declarations::section(tasks_root.as_deref()));
    // Sixth, and the first section that asks the RUNTIME a question rather than
    // analysing the files itself: will the gate accept what was declared? The
    // decomposition runs this lint over what it wrote, so a contract the
    // runtime refuses is caught here instead of hours into a run.
    out.push_str(&contracts::section(tasks_root.as_deref()));
    // Issue-117: a task body that obliges a file no task declares. Advisory,
    // like the sections above: it reads an obligation out of prose.
    out.push_str(&unowned_obligations::section(tasks_root.as_deref()));
    if let Some(root) = tasks_root.as_deref() {
        out.push_str(&task_set::inspect(cwd, root, mode)?.report);
    }
    Ok(out)
}

/// Findings the runtime is certain to refuse, for the command that gates on
/// them. Resolves the tasks root exactly as [`run_lint`] does, so the gate and
/// the report can never be looking at different files.
#[cfg(test)]
fn base_blocking_findings_with_mode(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
) -> Result<Vec<String>> {
    if let LintSource::TaskFile(path) = source {
        return Ok(task_file::inspect(cwd, path, mode).blockers);
    }
    let tasks_root = match source {
        LintSource::TaskFile(path) => absolute(cwd, path).parent().map(Path::to_path_buf),
        LintSource::Tasks(path) => Some(absolute(cwd, path)),
        LintSource::Spec(_) | LintSource::Graph(_) => None,
    };
    let root = tasks_root.as_deref();

    // Three facts, never a judgement. Each is derived from what the PRD and the
    // task files themselves declare, so this holds for any PRD in any domain,
    // and each one describes work that will silently not happen:
    //
    //   * a spec the runtime's own parser cannot read;
    //   * a requirement the PRD defines that no task claims — nobody builds it,
    //     and the run reports success without it;
    //   * a task with no runnable command — it can never prove what it claims,
    //     so accepting it means accepting a summary nobody can check.
    //
    // The ownership heuristic stays out: a guess that blocks a decomposition is
    // a guess the author cannot argue with, and the first time it is wrong the
    // whole gate gets switched off.
    let mut findings = contracts::blocking_findings(root);
    if let Some(root) = root {
        findings.extend(task_set::inspect(cwd, root, mode)?.blockers);
        findings.extend(
            tool_obligations::set_findings(cwd, root)
                .into_iter()
                .map(|f| f.text),
        );
    }
    findings.extend(
        declarations::tasks_without_a_runnable_test(root)
            .into_iter()
            .map(|task| declarations::missing_runnable_test_finding(&task)),
    );
    Ok(findings)
}

#[cfg(test)]
fn blocking_findings_with_mode(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
) -> Result<Vec<String>> {
    let mut findings = base_blocking_findings_with_mode(cwd, source, mode)?;
    let root = match source {
        LintSource::TaskFile(path) => absolute(cwd, path).parent().map(Path::to_path_buf),
        LintSource::Tasks(path) => Some(absolute(cwd, path)),
        LintSource::Spec(_) | LintSource::Graph(_) => None,
    };
    findings.extend(
        coverage::policy_findings(root.as_deref())
            .into_iter()
            .map(|finding| finding.text),
    );
    Ok(findings)
}

#[cfg(test)]
pub(crate) fn run_lint(cwd: &Path, source: &LintSource) -> Result<String> {
    run_lint_with_mode(cwd, source, archon_core::config::GateMode::Enforce)
}

#[cfg(test)]
pub(crate) fn blocking_findings(cwd: &Path, source: &LintSource) -> Vec<String> {
    blocking_findings_with_mode(cwd, source, archon_core::config::GateMode::Enforce)
        .unwrap_or_else(|error| vec![error.to_string()])
}

pub(crate) fn evaluate_lint(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    let _read = chain_read(cwd, source)?;
    preflight::operational_input(cwd, source)?;
    let graph_error = if matches!(source, LintSource::Tasks(_)) {
        load_graph(cwd, source).err().map(|error| {
            format!(
                "task graph could not be lowered: {error}; correct the named depends_on/blocks declaration or task status, then re-run `workflow lint --tasks <DIR>`"
            )
        })
    } else {
        None
    };
    let report = run_lint_with_mode(cwd, source, mode)?;
    let gate_id = match source {
        LintSource::TaskFile(_) => crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        _ => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
    };
    let source_path = match source {
        LintSource::TaskFile(path) | LintSource::Tasks(path) | LintSource::Spec(path) => {
            Some(absolute(cwd, path))
        }
        LintSource::Graph(_) => None,
    };
    let subject = describe(source);
    let mut deterministic = std::collections::BTreeMap::new();
    let mut contract_defects = Vec::new();
    let (base_findings, inherited_findings) = match source {
        LintSource::TaskFile(path) => {
            let lint = task_file::inspect(cwd, path, mode);
            deterministic = lint.deterministic;
            (lint.blockers, lint.inherited_blockers)
        }
        _ => {
            let root = match source {
                LintSource::Tasks(path) => Some(absolute(cwd, path)),
                LintSource::Spec(_) | LintSource::Graph(_) | LintSource::TaskFile(_) => None,
            };
            // Kept with their identities, never re-derived from message text.
            contract_defects = contracts::blocking_defects(root.as_deref());
            let mut blockers = Vec::new();
            let mut inherited = std::collections::BTreeSet::new();
            if let Some(root) = root.as_deref() {
                let lint = task_set::inspect(cwd, root, mode)?;
                blockers.extend(lint.blockers);
                deterministic = lint.deterministic;
                inherited = lint.inherited_blockers;
            }
            blockers.extend(
                declarations::tasks_without_a_runnable_test(root.as_deref())
                    .into_iter()
                    .map(|task| declarations::missing_runnable_test_finding(&task)),
            );
            (blockers, inherited)
        }
    };
    let default_scope = match source {
        LintSource::TaskFile(_) => archon_workflow::RemediationScope::Body,
        _ => archon_workflow::RemediationScope::Skeleton,
    };
    let mut findings = base_findings
        .into_iter()
        .map(|text| {
            let remediation_scope = if inherited_findings.contains(&text) {
                archon_workflow::RemediationScope::InheritedPredecessor
            } else {
                default_scope
            };
            let finding_subject = crate::command::workflow_gate::finding_subject(&text, &subject);
            let identity = deterministic
                .get_mut(&text)
                .and_then(|identities| identities.pop_front());
            let mut finding = crate::command::workflow_gate::GateFinding::new(
                gate_id,
                text,
                finding_subject,
                source_path.clone(),
                remediation_scope,
            );
            finding.deterministic_defect = identity;
            finding
        })
        .collect::<Vec<_>>();
    let path = source_path.as_deref();
    let gate = (gate_id, subject.as_str(), path, default_scope);
    findings.extend(contracts::gate_findings(contract_defects, gate));
    let coverage_root = match source {
        LintSource::TaskFile(_) => None,
        LintSource::Tasks(path) => Some(absolute(cwd, path)),
        LintSource::Spec(_) | LintSource::Graph(_) => None,
    };
    let mut repository_error = None;
    if let Some(root) = coverage_root.as_deref() {
        findings.extend(tool_obligations::set_findings(cwd, root));
        findings.extend(scope_declarations::set_findings(root));
        match repository_claims::set_findings(cwd, root) {
            Ok(claims) => findings.extend(claims),
            Err(error) => {
                repository_error = Some(format!("repository claim check failed: {error:#}"))
            }
        }
        match owner_coverage::set_findings(root) {
            Ok(owners) => findings.extend(owners),
            Err(error) => {
                let text = format!("repository owner coverage failed: {error:#}");
                repository_error = Some(match repository_error {
                    Some(existing) => format!("{existing}; {text}"),
                    None => text,
                });
            }
        }
        match focused_test_files::set_findings(root) {
            Ok(owned) => findings.extend(owned),
            Err(error) => {
                let text = format!("focused test file ownership failed: {error:#}");
                repository_error = Some(match repository_error {
                    Some(existing) => format!("{existing}; {text}"),
                    None => text,
                });
            }
        }
    }
    findings.extend(
        coverage::policy_findings(coverage_root.as_deref())
            .into_iter()
            .map(|finding| {
                crate::command::workflow_gate::GateFinding::new(
                    gate_id,
                    finding.text,
                    finding.subject,
                    Some(finding.source_path),
                    finding.remediation_scope,
                )
            }),
    );
    let evaluation = crate::command::workflow_gate::GateEvaluation::new(report, findings);
    let operational = match (graph_error, repository_error) {
        (Some(graph), Some(repository)) => Some(format!("{graph}; {repository}")),
        (graph, repository) => graph.or(repository),
    };
    Ok(match operational {
        Some(error) => evaluation.with_operational_error(error),
        None => evaluation,
    })
}

fn describe(source: &LintSource) -> String {
    match source {
        LintSource::TaskFile(path) => format!("task file {}", path.display()),
        LintSource::Tasks(path) => format!("task directory {}", path.display()),
        LintSource::Spec(path) => format!("workflow spec {}", path.display()),
        LintSource::Graph(id) => format!("recorded graph {id}"),
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

#[cfg(test)]
#[path = "topology_lint/tool_obligation_tests.rs"]
mod tool_obligation_tests;
