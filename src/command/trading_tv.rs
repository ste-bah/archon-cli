use anyhow::{Result, anyhow};
use std::path::PathBuf;

use crate::cli_args::TradingCliTvAction;

use super::trading_tools::{checked_text, project_root, run_node_script, tv_cli};

pub(crate) fn render_tv(action: &TradingCliTvAction) -> Result<String> {
    match action {
        TradingCliTvAction::Status { target } => run_tv(target.as_ref(), &["status".into()]),
        TradingCliTvAction::Launch { target, port } => run_tv(
            target.as_ref(),
            &["launch".into(), "--port".into(), port.to_string()],
        ),
        TradingCliTvAction::Cli { target, args } => {
            if args.is_empty() {
                return Err(anyhow!("tv cli requires arguments, for example: status"));
            }
            run_tv(target.as_ref(), args)
        }
    }
}

fn run_tv(target: Option<&PathBuf>, args: &[String]) -> Result<String> {
    let root = project_root(target)?;
    let cli = tv_cli(&root);
    if !cli.is_file() {
        return Err(anyhow!(
            "TradingView MCP CLI missing at {}; run scripts/setup-trading-tools.sh --target {}",
            cli.display(),
            root.display()
        ));
    }
    let output = run_node_script(&root, &cli, args)?;
    checked_text(output, "TradingView MCP CLI")
}
