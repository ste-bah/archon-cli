use anyhow::{Context, Result, anyhow};
use archon_trading::pine_lab::{ScriptVariant, generate_pine_scripts};
use archon_trading::spec_registry::parse_strategy_spec_json;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::cli_args::TradingCliPineAction;

use super::trading_tools::{checked_text, project_root, run_node_script, tv_cli};

pub(crate) fn render_pine(action: &TradingCliPineAction) -> Result<String> {
    match action {
        TradingCliPineAction::Generate {
            strategy_id,
            spec,
            out,
        } => generate(strategy_id, spec, out),
        TradingCliPineAction::Analyze { target, source } => {
            run_pine_tool(target.as_ref(), "analyze", source)
        }
        TradingCliPineAction::Check { target, source } => {
            run_pine_tool(target.as_ref(), "check", source)
        }
    }
}

fn generate(strategy_id: &str, spec_path: &Path, out_dir: &Path) -> Result<String> {
    let text = std::fs::read_to_string(spec_path)
        .with_context(|| format!("failed to read StrategySpec {}", spec_path.display()))?;
    let spec = parse_strategy_spec_json(&text)
        .map_err(|err| anyhow!("invalid StrategySpec JSON: {err:?}"))?;
    let report = generate_pine_scripts(strategy_id, &spec)
        .map_err(|err| anyhow!("Pine generation failed: {err:?}"))?;

    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir {}", out_dir.display()))?;
    let mut manifest_scripts = Vec::new();
    for script in &report.scripts {
        let file_name = format!(
            "{}-{}-{}.pine",
            strategy_id,
            clean_file_token(&script.symbol),
            variant_token(script.variant)
        );
        let path = out_dir.join(file_name);
        std::fs::write(&path, &script.source)
            .with_context(|| format!("failed to write {}", path.display()))?;
        manifest_scripts.push(json!({
            "symbol": script.symbol,
            "variant": variant_token(script.variant),
            "path": path.display().to_string(),
            "alert_handoff": script.alert_handoff,
        }));
    }

    let manifest = json!({
        "strategy_id": strategy_id,
        "spec": spec_path.display().to_string(),
        "script_count": report.scripts.len(),
        "portfolio_record": report.portfolio_record,
        "audit_events": report.audit_events,
        "scripts": manifest_scripts,
    });
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(format!(
        "Generated {} Pine script(s)\n  output: {}\n  manifest: {}",
        report.scripts.len(),
        out_dir.display(),
        manifest_path.display()
    ))
}

fn run_pine_tool(target: Option<&PathBuf>, subcommand: &str, source: &Path) -> Result<String> {
    let root = project_root(target)?;
    let cli = tv_cli(&root);
    if !cli.is_file() {
        return Err(anyhow!(
            "TradingView MCP CLI missing at {}; run scripts/setup-trading-tools.sh --target {}",
            cli.display(),
            root.display()
        ));
    }
    let args = vec![
        "pine".to_string(),
        subcommand.to_string(),
        "--file".to_string(),
        source.display().to_string(),
    ];
    let output = run_node_script(&root, &cli, &args)?;
    checked_text(output, &format!("tv pine {subcommand}"))
}

fn clean_file_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

const fn variant_token(variant: ScriptVariant) -> &'static str {
    match variant {
        ScriptVariant::Indicator => "indicator",
        ScriptVariant::Strategy => "strategy",
    }
}
