//! `archon requirements trace` — requirement→code traceability with a proof
//! ladder.
//!
//! # What it reports and what it refuses to report
//!
//! Per requirement: its proof level, its anchors, and — for anything below
//! `Exercised` — exactly what is missing. An unproven requirement reads as a
//! **declared residual gap** with fail-closed behaviour, per PRD §32. It is not
//! a failure, because a traceability report that failed CI would be muted within
//! a week; and it is emphatically not a pass, because calling an unproven edge
//! satisfied is the whole of finding F1.
//!
//! The process exit status is therefore success whichever way the report comes
//! out, exactly as `archon workflow lint` behaves. The gate is
//! `ProofLevel::satisfies_promotion_gate`, and it lives in the graph, not in an
//! exit code.
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

mod evidence;
mod falsify;
mod leann_source;
mod render;
mod slash;

pub(crate) use slash::RequirementsHandler;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use archon_knowledge::traceability::anchors::{AnchorGap, anchor_relation, check_freshness};
use archon_knowledge::traceability::report::{AnchorVerdict, find_shared_anchors, strongest_level};
use archon_knowledge::traceability::requirements::requirement_entity_for;
use archon_knowledge::traceability::store::AnchorRecord;
use archon_knowledge::traceability::{
    Anchor, AnchorFreshness, CodeSearch, CommandEvidence, ProofLevel, ReadEvidence, Requirement,
    RequirementRow, TaskBinding, TraceReport, coverage, falsification, ladder, requirements, tasks,
};

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
        }
    }
}

/// CLI entry point for `archon requirements <action>`.
///
/// Prints and returns success whatever the report says. The verdict is in the
/// report, not the exit code: a traceability report that failed the build would
/// be muted, and one that passed would be F1.
pub(crate) fn handle_requirements_command(
    action: &crate::cli_args::RequirementsAction,
    cwd: &Path,
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
    };
    println!("{}", run_trace(cwd, &options)?);
    Ok(())
}

/// Build and render the report.
pub(crate) fn run_trace(cwd: &Path, options: &TraceOptions) -> Result<String> {
    let mut report = build_report(cwd, options)?;
    // Before `--persist`, so a store written in the same invocation records the
    // level the experiment established rather than the one it started from.
    if options.falsify {
        falsify::execute_plans(cwd, &mut report);
    }
    if let Some(store_path) = &options.persist {
        persist(cwd, store_path, &report)?;
    }
    if options.json {
        return Ok(serde_json::to_string_pretty(&report)?);
    }
    Ok(render::report(&report))
}

/// Assemble the report from its three read-only inputs.
pub(crate) fn build_report(cwd: &Path, options: &TraceOptions) -> Result<TraceReport> {
    let prd_path = absolute(cwd, &options.prd);
    let prd = std::fs::read_to_string(&prd_path)
        .with_context(|| format!("reading PRD at {}", prd_path.display()))?;
    let requirements = requirements::extract_requirements(&prd);

    let bindings = load_bindings(&absolute(cwd, &options.tasks))?;
    let coverage = coverage::check_coverage(&requirements, &bindings);

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
            Default::default(),
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
    Ok(TraceReport {
        prd_path: prd_path.display().to_string(),
        task_dir: absolute(cwd, &options.tasks).display().to_string(),
        coverage,
        rows,
        shared_anchors,
        stale_anchors,
        index_consulted: index.is_some(),
    })
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
fn load_bindings(dir: &Path) -> Result<Vec<TaskBinding>> {
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

    let mut bindings = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading task file {}", path.display()))?;
        let source = path.display().to_string().replace('\\', "/");
        bindings.push(tasks::parse_task_binding(&raw, &source)?);
    }
    Ok(bindings)
}

/// Write requirement entities and anchored edges into a knowledge store.
fn persist(cwd: &Path, store_path: &Path, report: &TraceReport) -> Result<()> {
    let path = absolute(cwd, store_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = archon_cozo::open_sqlite_guarded(
        path.to_string_lossy().as_ref(),
        "open knowledge store for requirement trace persistence",
        &archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .map_err(|e| anyhow::anyhow!("opening knowledge store at {}: {e}", path.display()))?;
    archon_knowledge::schema::ensure_knowledge_schema(&db)?;
    archon_knowledge::traceability::store::ensure_traceability_schema(&db)?;

    let now = chrono::Utc::now().to_rfc3339();
    for row in &report.rows {
        let entity = requirement_entity_for(&row.requirement_id, row.prd_line, &report.prd_path);
        archon_knowledge::store::insert_entity(&db, &entity)?;
        for verdict in &row.anchors {
            archon_knowledge::traceability::store::insert_anchor(
                &db,
                &anchor_relation(&verdict.anchor, &entity.entity_id),
                &AnchorRecord::from_anchor(&verdict.anchor, verdict.level, &now),
            )?;
        }
    }
    Ok(())
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
