//! `/requirements trace --prd <PATH> --tasks <DIR> …` — the TUI surface.
//!
//! The slash registry hands over raw tokens rather than a clap-parsed struct,
//! so the flags are read by hand, exactly as `/workflow lint` does. An
//! unrecognised token is an error naming the accepted flags: silently ignoring
//! one would produce a report of something other than what was asked for, and a
//! traceability report of the wrong thing is worse than none.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

use super::{TraceOptions, run_trace};

pub(crate) struct RequirementsHandler;

impl CommandHandler for RequirementsHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let cwd = ctx
            .working_dir
            .clone()
            .ok_or_else(|| anyhow!("requirements command requires working directory context"))?;
        let rest = match args.first().map(String::as_str) {
            Some("trace") => &args[1..],
            Some(other) => {
                return Err(anyhow!(
                    "unknown requirements action '{other}'; the only action is `trace`"
                ));
            }
            None => {
                return Err(anyhow!(
                    "requirements needs an action: `trace --prd <PATH> --tasks <DIR>`"
                ));
            }
        };
        let mut options = options_from_slash_args(rest)?;
        // Same embedder the index was built with (#148). `CommandContext`
        // carries the config *path* rather than the config, so this re-reads it
        // — one file read on a command that then walks a PRD and a task
        // directory, and the alternative is querying an index with vectors from
        // a different model, which produces confident nonsense rather than a
        // visible error. A context with no path (test fixtures) keeps the
        // default.
        if let Some(path) = ctx.config_path.clone()
            && let Ok(config) = archon_core::config::load_config_from(path)
        {
            options.embedding = config.memory.open_spec().embedding;
        }
        let output = run_trace(&cwd, &options)?;
        ctx.emit(TuiEvent::TextDelta(output));
        ctx.emit(TuiEvent::SlashCommandComplete);
        Ok(())
    }

    fn description(&self) -> &str {
        "Trace PRD requirements to code with a proof ladder"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["reqs"]
    }
}

/// Parse the flag pairs. `--prd` and `--tasks` are required, because there is no
/// sensible default for either and guessing would report on the wrong PRD.
pub(crate) fn options_from_slash_args(args: &[String]) -> Result<TraceOptions> {
    let mut prd: Option<PathBuf> = None;
    let mut tasks: Option<PathBuf> = None;
    let mut graph: Option<String> = None;
    let mut evidence: Option<PathBuf> = None;
    let mut leann_db: Option<PathBuf> = None;
    let mut persist: Option<PathBuf> = None;
    let mut json = false;
    let mut falsify = false;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        // Spelled out here as well as in clap, because the TUI surface reads
        // tokens by hand: a `--falsify` that fell through to the unknown-flag
        // arm would be an error, and one that was silently ignored would be a
        // user who asked to mutate their tree and got a read-only report.
        if flag == "--falsify" {
            falsify = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .cloned()
            .ok_or_else(|| anyhow!("requirements trace {flag} needs a value"))?;
        match flag {
            "--prd" => prd = Some(PathBuf::from(value)),
            "--tasks" => tasks = Some(PathBuf::from(value)),
            "--graph" => graph = Some(value),
            "--evidence" => evidence = Some(PathBuf::from(value)),
            "--leann-db" => leann_db = Some(PathBuf::from(value)),
            "--persist" => persist = Some(PathBuf::from(value)),
            other => {
                return Err(anyhow!(
                    "requirements trace does not accept '{other}'; use --prd <PATH>, \
                     --tasks <DIR>, --graph <ID>, --evidence <PATH>, --leann-db <PATH>, \
                     --persist <PATH>, --falsify, or --json"
                ));
            }
        }
        index += 2;
    }

    let prd = prd.ok_or_else(|| anyhow!("requirements trace needs --prd <PATH>"))?;
    let tasks = tasks.ok_or_else(|| anyhow!("requirements trace needs --tasks <DIR>"))?;
    Ok(TraceOptions {
        graph,
        evidence,
        leann_db,
        persist,
        falsify,
        json,
        ..TraceOptions::new(prd, tasks)
    })
}
