//! Exact parser shared by the slash workflow-lint surface and its tests.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

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

pub(crate) fn lint_from_slash_args(cwd: &Path, args: &[String]) -> Result<String> {
    let source = lint_source_from_slash_args(args)?;
    crate::command::topology_lint::run_lint(cwd, &source)
}
