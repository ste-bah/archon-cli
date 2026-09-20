//! `archon requirements trace` — requirement→code traceability with a proof
//! ladder.
//!
//! # What it reports and what it refuses to report
//!
//! Per requirement: its proof level, its anchors, and — for anything below
//! `Exercised` — exactly what is missing. An unproven requirement reads as a
//! **declared residual gap** with fail-closed behaviour, per PRD §32. It is not
//! an acceptance failure and emphatically not a pass, because calling an
//! unproven edge satisfied is the whole of finding F1. Deterministic input and
//! set-coverage defects are different: malformed bindings, empty populations,
//! phantom citations, and unclaimed obligations return non-zero after the
//! report is rendered.
//!
//! # Read-only, and never mid-workflow
//!
//! Three inputs, all read: the PRD, the task directory, and — optionally — an
//! already-built code index, a run's verifier evidence and that run's ambient
//! trace. Nothing here indexes; see [`leann_source`] for why that is enforced at
//! the point of construction rather than by convention.
//!
//! `--persist` is the only write in the default configuration, and it writes to
//! a knowledge store the caller names, never to the code index.
//!
//! # The one exception, and why it is a flag
//!
//! `--falsify` executes the falsification plans: it mutates an anchored file,
//! runs the verifier the task declared, and restores. That is the opposite of
//! read-only, which is why it is off unless a person types it. Without the flag
//! nothing in [`falsify`] runs and the output — text and JSON alike — is
//! byte-identical to what it was before the flag existed. The module documents
//! what it refuses to do (a dirty file, a workspace-wide command) and what
//! happens on every path out of a mutation.

#[path = "requirement_trace/evaluation.rs"]
mod evaluation;
mod evidence;
mod falsify;
mod leann_source;
mod persist;
mod render;
mod slash;
mod staged;
mod verdict;
pub(crate) use evaluation::{evaluate_trace, evaluate_trace_for_published_bodies};
use persist::persist;
#[cfg(test)]
use persist::write_cli_report;

pub(crate) use slash::RequirementsHandler;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
use anyhow::anyhow;
use anyhow::{Context, Result};
use archon_knowledge::traceability::anchors::{AnchorGap, check_freshness};
use archon_knowledge::traceability::report::{AnchorVerdict, find_shared_anchors, strongest_level};
use archon_knowledge::traceability::{
    Anchor, AnchorFreshness, CodeSearch, CommandEvidence, ProofLevel, ReadEvidence, Requirement,
    RequirementRow, TaskBinding, TraceReport, coverage, falsification, ladder, requirements, tasks,
};
#[cfg(test)]
use verdict::TraceVerdict;

/// Everything the command was told to look at.
#[derive(Debug, Clone)]
pub(crate) struct TraceOptions {
    /// The PRD to extract requirements from.
    pub(crate) prd: PathBuf,
    /// Directory of decomposed-PRD `TASK-*.md` files.
    pub(crate) tasks: PathBuf,
    /// Recorded graph id under `.archon/topology`, for `FileRead` evidence.
    pub(crate) graph: Option<String>,
    /// A run's final report, for `commands_run` evidence.
    pub(crate) evidence: Option<PathBuf>,
    /// The code index. `None` skips anchoring and says so.
    pub(crate) leann_db: Option<PathBuf>,
    /// Knowledge store to persist entities and anchors into.
    pub(crate) persist: Option<PathBuf>,
    /// Execute the falsification plans instead of only printing them.
    ///
    /// The one option here that writes to the working tree, and the only reason
    /// the rest of this struct can still be described as read-only. Off unless
    /// a person typed `--falsify`; see [`falsify`] for what "off" has to mean.
    pub(crate) falsify: bool,
    /// Emit the report model as JSON rather than text.
    pub(crate) json: bool,
    /// Hits requested per declared path scope.
    pub(crate) limit_per_scope: usize,
    /// Declared path scopes searched per task, capping the query budget.
    pub(crate) max_scopes: usize,
    /// How to embed the queries put to the code index.
    ///
    /// Carried here rather than threaded as an `&ArchonConfig` because
    /// `build_report` has eight test call sites that have no config and want
    /// none. It MUST match whatever built the index: cosine similarity between
    /// vectors from two different models is not a weaker signal, it is an
    /// unrelated number, and this command's whole job is saying whether a
    /// requirement is proven. See #148.
    pub(crate) embedding: archon_memory::embedding::EmbeddingConfig,
}

impl TraceOptions {
    /// Defaults chosen so a bare `--prd/--tasks` run costs no queries at all and
    /// still answers the coverage question.
    pub(crate) fn new(prd: PathBuf, tasks: PathBuf) -> Self {
        Self {
            prd,
            tasks,
            graph: None,
            evidence: None,
            leann_db: None,
            persist: None,
            falsify: false,
            json: false,
            limit_per_scope: 3,
            max_scopes: 8,
            // Both entry points overwrite this from `[memory]`; what is left
            // reaching the default is a test that built its own index with the
            // same default, which is consistent by construction.
            embedding: archon_memory::embedding::EmbeddingConfig::default(),
        }
    }
}

/// CLI entry point for `archon requirements <action>`.
///
/// set-coverage defects. Missing optional proof evidence remains in the report.
pub(crate) fn handle_requirements_command(
    action: &crate::cli_args::RequirementsAction,
    cwd: &Path,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let crate::cli_args::RequirementsAction::Trace {
        prd,
        tasks,
        graph,
        evidence,
        leann_db,
        persist,
        falsify,
        json,
        limit_per_scope,
        max_scopes,
        gate_envelope,
        call_id,
    } = action;
    let options = TraceOptions {
        prd: prd.clone(),
        tasks: tasks.clone(),
        graph: graph.clone(),
        evidence: evidence.clone(),
        leann_db: leann_db.clone(),
        persist: persist.clone(),
        falsify: *falsify,
        json: *json,
        limit_per_scope: *limit_per_scope,
        max_scopes: *max_scopes,
        // `[memory] embedding_*` through the same mapping every other opener
        // uses, so the query embedder cannot drift from the index's.
        embedding: config.memory.open_spec().embedding,
    };
    let mode = config.workflow.gate_mode;
    if gate_envelope.is_some() || call_id.is_some() {
        return staged::handle(
            cwd,
            &options,
            gate_envelope.as_deref(),
            call_id.as_deref(),
            mode,
        );
    }
    let disposition = crate::command::workflow_gate::run_sync_gate(
        cwd,
        mode,
        crate::command::workflow_gate::GateId::RequirementsTrace,
        || evaluate_trace(cwd, &options),
    )?;
    let stdout = std::io::stdout();
    persist::write_cli_report(&mut stdout.lock(), disposition.report())?;
    for diagnostic in disposition.diagnostics() {
        eprintln!("{diagnostic}");
    }
    disposition.require_allowed()
}

/// Build and render the report together with deterministic gate findings.
#[cfg(test)]
pub(crate) fn run_trace(cwd: &Path, options: &TraceOptions) -> Result<TraceVerdict> {
    let (mut report, input_findings, prd_findings) =
        build_report_with_input_findings(cwd, options)?;
    let task_population_complete = input_findings.is_empty();
    let mut blocking_findings = verdict::blocking_findings(&report, input_findings);
    blocking_findings.extend(prd_findings);
    blocking_findings.sort();
    blocking_findings.dedup();

    // Never mutate or persist evidence from malformed or incomplete inputs.
    // Before `--persist`, so a clean store records the level the experiment
    // established rather than the one it started from.
    if task_population_complete {
        if options.falsify {
            falsify::execute_plans(cwd, &mut report);
        }
        if let Some(store_path) = &options.persist {
            persist(cwd, store_path, &report)?;
        }
    }
    verdict::render_verdict(
        &report,
        options.json,
        task_population_complete,
        blocking_findings,
    )
}

/// Assemble a complete report, refusing any malformed TASK binding.
#[cfg(test)]
pub(crate) fn build_report(cwd: &Path, options: &TraceOptions) -> Result<TraceReport> {
    let (report, input_findings, _prd_findings) = build_report_with_input_findings(cwd, options)?;
    if input_findings.is_empty() {
        return Ok(report);
    }
    Err(anyhow!(
        "traceability input error:\n  {}",
        input_findings.join("\n  ")
    ))
}

fn build_report_with_input_findings(
    cwd: &Path,
    options: &TraceOptions,
) -> Result<(TraceReport, Vec<String>, Vec<String>)> {
    let prd_path = absolute(cwd, &options.prd);
    let prd = std::fs::read_to_string(&prd_path)
        .with_context(|| format!("reading PRD at {}", prd_path.display()))?;
    let requirements = requirements::extract_requirements(&prd);
    let obligations = archon_workflow::obligation_ids::obligation_ids(&prd);
    let mut prd_findings = archon_workflow::obligation_ids::malformed_obligation_ids(&prd)
        .into_iter()
        .map(|id| archon_workflow::obligation_ids::malformed_obligation_finding(&id))
        .collect::<Vec<_>>();
    prd_findings.extend(
        archon_workflow::obligation_ids::duplicate_obligation_ids(&prd)
            .into_iter()
            .map(|id| archon_workflow::obligation_ids::duplicate_obligation_finding(&id)),
    );
    prd_findings.sort();
    prd_findings.dedup();

    let task_dir = absolute(cwd, &options.tasks);
    let (bindings, input_findings) = load_bindings_with_findings(&task_dir)?;
    let coverage = coverage::check_coverage(&obligations, &bindings);
    if !input_findings.is_empty() {
        return Ok((
            TraceReport {
                prd_path: prd_path.display().to_string(),
                task_dir: task_dir.display().to_string(),
                coverage,
                rows: Vec::new(),
                shared_anchors: Vec::new(),
                stale_anchors: 0,
                index_consulted: false,
            },
            input_findings,
            prd_findings,
        ));
    }

    let commands = match &options.evidence {
        Some(path) => evidence::load_commands(&absolute(cwd, path))?,
        None => Vec::new(),
    };
    let reads = match &options.graph {
        Some(graph_id) => evidence::load_reads(cwd, graph_id)?,
        None => Vec::new(),
    };

    let index = match &options.leann_db {
        Some(path) => Some(leann_source::LeannCodeSearch::open(
            &absolute(cwd, path),
            options.embedding.clone(),
        )?),
        None => None,
    };

    let by_task: BTreeMap<&str, &TaskBinding> = bindings
        .iter()
        .map(|binding| (binding.task_id.as_str(), binding))
        .collect();

    let mut rows = Vec::with_capacity(requirements.len());
    let mut stale_anchors = 0usize;
    for requirement in &requirements {
        let row = build_row(
            cwd,
            requirement,
            &coverage,
            &by_task,
            index.as_ref().map(|i| i as &dyn CodeSearch),
            &commands,
            &reads,
            options,
        )?;
        stale_anchors += row
            .anchors
            .iter()
            .filter(|verdict| !verdict.freshness.is_fresh())
            .count();
        rows.push(row);
    }

    let shared_anchors = find_shared_anchors(&rows);
    Ok((
        TraceReport {
            prd_path: prd_path.display().to_string(),
            task_dir: task_dir.display().to_string(),
            coverage,
            rows,
            shared_anchors,
            stale_anchors,
            index_consulted: index.is_some(),
        },
        input_findings,
        prd_findings,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_row(
    cwd: &Path,
    requirement: &Requirement,
    coverage: &coverage::CoverageReport,
    by_task: &BTreeMap<&str, &TaskBinding>,
    index: Option<&dyn CodeSearch>,
    commands: &[CommandEvidence],
    reads: &[ReadEvidence],
    options: &TraceOptions,
) -> Result<RequirementRow> {
    let claimed_by = coverage
        .claimed_by
        .get(&requirement.id)
        .cloned()
        .unwrap_or_default();

    let mut row = RequirementRow {
        requirement_id: requirement.id.clone(),
        prd_line: requirement.line,
        severity: requirement.severity,
        severity_evidence: requirement.severity_evidence.clone(),
        claimed_by: claimed_by.clone(),
        anchors: Vec::new(),
        anchor_gap: None,
        level: ProofLevel::Unproven,
    };

    if claimed_by.is_empty() {
        row.anchor_gap = Some(AnchorGap::Unclaimed);
        return Ok(row);
    }
    let Some(index) = index else {
        // "We did not look" is reported as such, never as "we looked and found
        // nothing" — understating the code without evidence is the same error
        // as overstating it.
        row.anchor_gap = Some(AnchorGap::IndexNotConsulted);
        return Ok(row);
    };

    let mut anchors: Vec<Anchor> = Vec::new();
    let mut gap: Option<AnchorGap> = None;
    for task_id in &claimed_by {
        let Some(binding) = by_task.get(task_id.as_str()) else {
            continue;
        };
        match archon_knowledge::traceability::anchors::anchor_requirement(
            index,
            requirement,
            binding,
            cwd,
            options.limit_per_scope,
            options.max_scopes,
        )? {
            Ok(found) => anchors.extend(found),
            Err(found_gap) => {
                gap.get_or_insert(found_gap);
            }
        }
    }

    if anchors.is_empty() {
        row.anchor_gap = gap;
        return Ok(row);
    }

    for anchor in anchors {
        let binding = by_task
            .get(anchor.task_id.as_str())
            .copied()
            .expect("anchor carries the task that produced it");
        let freshness = check_freshness(&anchor, cwd);
        let (level, proof, missing) = match freshness {
            // A stale anchor names a line range in a file that has since
            // changed. Promoting it would be asserting something about code
            // that no longer exists in that form.
            AnchorFreshness::Fresh => ladder::promote(&anchor, binding, commands, reads),
            _ => (ProofLevel::Unproven, None, None),
        };
        let falsification = falsification::plan(requirement, &anchor, level, proof.as_ref());
        row.anchors.push(AnchorVerdict {
            anchor,
            freshness,
            level,
            proof,
            missing,
            falsification,
            // Populated only by `--falsify`, and only after the whole report
            // exists: a plan is decided by running it, not by building a row.
            falsification_outcome: None,
        });
    }
    row.level = strongest_level(&row.anchors);
    Ok(row)
}

/// Read every `TASK-*.md` in a directory.
///
/// A file that fails to parse aborts with its own name in the message. A task
/// set that cannot be read in full cannot answer "is every requirement
/// claimed", and answering it anyway from a partial read is how an unclaimed
/// requirement disappears.
#[cfg(test)]
fn load_bindings(dir: &Path) -> Result<Vec<TaskBinding>> {
    let (bindings, findings) = load_bindings_with_findings(dir)?;
    if findings.is_empty() {
        return Ok(bindings);
    }
    Err(anyhow!(
        "traceability input error:\n  {}",
        findings.join("\n  ")
    ))
}

fn load_bindings_with_findings(dir: &Path) -> Result<(Vec<TaskBinding>, Vec<String>)> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading task directory {}", dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let is_task = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("TASK-") && name.ends_with(".md"));
        if is_task {
            paths.push(path);
        }
    }
    paths.sort();

    if paths.is_empty() {
        return Ok((
            Vec::new(),
            vec![format!(
                "task directory {} contains zero TASK-*.md files; add the decomposed TASK files or correct --tasks to the directory that contains them",
                dir.display()
            )],
        ));
    }

    let mut bindings = Vec::with_capacity(paths.len());
    let mut findings = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading task file {}", path.display()))?;
        let source = path.display().to_string().replace('\\', "/");
        match tasks::parse_task_binding(&raw, &source) {
            Ok(binding) => bindings.push(binding),
            Err(error) => findings.push(format!(
                "{error}; rewrite the named YAML field exactly as shown, then re-run the same trace command"
            )),
        }
    }
    findings.sort();
    findings.dedup();
    Ok((bindings, findings))
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod tests;
