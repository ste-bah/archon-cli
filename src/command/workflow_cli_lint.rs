//! Exact parser shared by the slash workflow-lint surface and its tests.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_workflow::{WorkflowLlmClientFactory, WorkflowLlmClientRequest};

use crate::command::topology_lint::LintSource;

/// `/workflow lint --tasks <DIR>` and friends, parsed by hand.
///
/// The slash surface hands over raw tokens rather than a clap-parsed struct, so
/// the three flags are read directly. An unrecognised token is an error naming
/// the accepted flags: silently ignoring it would produce a report of something
/// other than what was asked for, which for a lint is worse than no report.
pub(super) fn lint_source_from_slash_args(
    args: &[String],
) -> Result<crate::command::topology_lint::LintSource> {
    let mut task_file: Option<PathBuf> = None;
    let mut tasks: Option<PathBuf> = None;
    let mut spec_file: Option<PathBuf> = None;
    let mut graph: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let value = args.get(index + 1).cloned();
        let missing = |flag: &str| anyhow!("workflow lint {flag} needs a value");
        match args[index].as_str() {
            "--task-file" => {
                task_file = Some(PathBuf::from(value.ok_or_else(|| missing("--task-file"))?))
            }
            "--tasks" => tasks = Some(PathBuf::from(value.ok_or_else(|| missing("--tasks"))?)),
            "--spec-file" => {
                spec_file = Some(PathBuf::from(value.ok_or_else(|| missing("--spec-file"))?));
            }
            "--graph" => graph = Some(value.ok_or_else(|| missing("--graph"))?),
            // The slash surface runs on the session's synchronous command path
            // and cannot await a critic call; the CLI owns fidelity and waivers.
            flag @ ("--fidelity" | "--waive-obligation" | "--waive-reason") => {
                return Err(anyhow!(
                    "/workflow lint {flag} is CLI-only; run `archon workflow lint --tasks <DIR> --fidelity` in a terminal"
                ));
            }
            other => {
                return Err(anyhow!(
                    "workflow lint does not accept '{other}'; use --task-file <PATH>, --tasks <DIR>, --spec-file <PATH>, or --graph <ID>"
                ));
            }
        }
        index += 2;
    }
    let source = crate::command::topology_lint::LintSource::from_flags(
        task_file.as_deref(),
        tasks.as_deref(),
        spec_file.as_deref(),
        graph.as_deref(),
    )?;
    Ok(source)
}

#[cfg(test)]
pub(crate) fn lint_from_slash_args(cwd: &Path, args: &[String]) -> Result<String> {
    let source = lint_source_from_slash_args(args)?;
    crate::command::topology_lint::run_lint(cwd, &source)
}

/// `--fidelity`, `--waive-obligation`, `--waive-reason` as the CLI parsed them.
pub(super) struct FidelityFlags {
    pub(super) fidelity: bool,
    pub(super) waive_obligation: Vec<String>,
    pub(super) waive_reason: Option<String>,
}

/// The operator-facing lint: evaluate, then dispose through the sync gate.
///
/// Fidelity is opt-in here because it spends tokens on every claimed
/// obligation; a waiver implies it, since waiving a finding one has not seen
/// is not a decision. Waivers are recorded before the audit runs, so the
/// record exists even if the provider then fails — the operator's decision is
/// not lost to an outage, and the next freeze honours it.
pub(super) async fn run_cli_lint(
    cwd: &Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    source: LintSource,
    flags: FidelityFlags,
) -> Result<()> {
    let mode = config.workflow.gate_mode;
    let gate_id = match source {
        LintSource::TaskFile(_) => crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        _ => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
    };
    let waivers = crate::command::topology_lint::waivers_from_flags(
        &flags.waive_obligation,
        flags.waive_reason.as_deref(),
    )?;
    let with_fidelity = flags.fidelity || !waivers.is_empty();
    let evaluation = if with_fidelity && mode != archon_core::config::GateMode::Off {
        let LintSource::Tasks(tasks) = &source else {
            return Err(anyhow!(
                "--fidelity and --waive-obligation need --tasks <DIR>: the audit reads every task body that claims an obligation"
            ));
        };
        let tasks_root = if tasks.is_absolute() {
            tasks.clone()
        } else {
            cwd.join(tasks)
        };
        if !waivers.is_empty() {
            let pin = crate::command::topology_lint::record_waivers(cwd, &tasks_root, &waivers)?;
            eprintln!(
                "recorded {} obligation waiver(s) verbatim in {}",
                waivers.len(),
                pin.display()
            );
        }
        let waivers = crate::command::topology_lint::recorded_waivers(cwd, &tasks_root);
        let factory = crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::new(
            config, env_vars,
        );
        let client = factory
            .build_client(WorkflowLlmClientRequest {
                cwd: cwd.to_path_buf(),
                origin: "workflow-lint-fidelity".into(),
                session_id: format!("lint-fidelity-{}", uuid::Uuid::new_v4()),
                read_roots: Vec::new(),
            })
            .await
            .map_err(anyhow::Error::new)
            .context("building the obligation fidelity critic client");
        Some(
            crate::command::topology_lint::evaluate_lint_with_fidelity(
                cwd, &source, mode, client, &waivers,
            )
            .await?,
        )
    } else {
        None
    };
    let disposition =
        crate::command::workflow_gate::run_sync_gate(cwd, mode, gate_id, || match evaluation {
            Some(evaluation) => Ok(evaluation),
            None => crate::command::topology_lint::evaluate_lint(cwd, &source, mode),
        })?;
    print!("{}", disposition.report());
    for diagnostic in disposition.diagnostics() {
        eprintln!("{diagnostic}");
    }
    disposition.require_allowed()
}
