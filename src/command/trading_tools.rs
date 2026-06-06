use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct ToolCommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) fn render_tools_status(target: Option<&PathBuf>) -> Result<String> {
    let root = project_root(target)?;
    let tv_dir = tradingview_dir(&root);
    let venv_dir = openbb_venv_dir(&root);
    let mcp_config = root.join(".mcp.json");
    let openbb_api = openbb_bin(&root, "openbb-api");

    let mut lines = vec![
        "Trading Lab external tools".to_string(),
        format!("  project: {}", root.display()),
        format!("  tools dir: {}", root.join(".archon/tools").display()),
        format!("  node: {}", binary_state("node")),
        format!("  npm: {}", binary_state("npm")),
        format!("  python3: {}", binary_state("python3")),
        format!("  git: {}", binary_state("git")),
        format!("  TradingView MCP: {}", path_state(&tv_dir)),
        format!(
            "  TradingView MCP server: {}",
            path_state(&tv_server(&root))
        ),
        format!("  TradingView CLI: {}", path_state(&tv_cli(&root))),
        format!("  project .mcp.json: {}", path_state(&mcp_config)),
        format!(
            "  .mcp.json tradingview entry: {}",
            tradingview_mcp_state(&mcp_config)
        ),
        format!("  OpenBB venv: {}", path_state(&venv_dir)),
        format!("  openbb-api: {}", path_state(&openbb_api)),
        format!(
            "  OpenBB API URL: {}",
            std::env::var("OPENBB_API_URL").unwrap_or_else(|_| "http://127.0.0.1:6900".into())
        ),
    ];

    lines.push("".into());
    lines.push("Setup command:".into());
    lines.push(format!(
        "  scripts/setup-trading-tools.sh --target {}",
        shell_path(&root)
    ));
    Ok(lines.join("\n"))
}

pub(crate) fn run_setup_script(
    target: Option<&PathBuf>,
    check: bool,
    skip_tradingview: bool,
    skip_openbb: bool,
) -> Result<String> {
    let root = project_root(target)?;
    let script = root.join("scripts/setup-trading-tools.sh");
    if !script.is_file() {
        return Err(anyhow!(
            "Trading setup script not found at {}; copy scripts/setup-trading-tools.sh into this project or run it from the archon-cli repo",
            script.display()
        ));
    }
    let mut args = vec!["--target".to_string(), root.display().to_string()];
    if check {
        args.push("--check".into());
    }
    if skip_tradingview {
        args.push("--skip-tradingview".into());
    }
    if skip_openbb {
        args.push("--skip-openbb".into());
    }
    let output = run_command(script.as_path(), &args, Some(&root))?;
    checked_text(output, "setup-trading-tools")
}

pub(crate) fn project_root(target: Option<&PathBuf>) -> Result<PathBuf> {
    let root = match target {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("failed to resolve current directory")?,
    };
    Ok(root)
}

pub(crate) fn tv_cli(project_root: &Path) -> PathBuf {
    tradingview_dir(project_root).join("src/cli/index.js")
}

pub(crate) fn tv_server(project_root: &Path) -> PathBuf {
    tradingview_dir(project_root).join("src/server.js")
}

pub(crate) fn tradingview_dir(project_root: &Path) -> PathBuf {
    project_root.join(".archon/tools/tradingview-mcp")
}

pub(crate) fn openbb_bin(project_root: &Path, name: &str) -> PathBuf {
    let bin_dir = if cfg!(windows) { "Scripts" } else { "bin" };
    openbb_venv_dir(project_root).join(bin_dir).join(name)
}

pub(crate) fn openbb_venv_dir(project_root: &Path) -> PathBuf {
    project_root.join(".archon/tools/openbb-venv")
}

pub(crate) fn run_node_script(
    project_root: &Path,
    script: &Path,
    args: &[String],
) -> Result<ToolCommandOutput> {
    let mut full_args = vec![script.display().to_string()];
    full_args.extend(args.iter().cloned());
    run_command("node", &full_args, Some(project_root))
}

pub(crate) fn run_command<P: AsRef<std::ffi::OsStr>>(
    program: P,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<ToolCommandOutput> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().context("failed to run external tool")?;
    Ok(ToolCommandOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub(crate) fn checked_text(output: ToolCommandOutput, label: &str) -> Result<String> {
    let text = join_output(&output);
    if output.status == 0 {
        Ok(text)
    } else {
        Err(anyhow!(
            "{label} failed with exit {}:\n{text}",
            output.status
        ))
    }
}

pub(crate) fn join_output(output: &ToolCommandOutput) -> String {
    match (output.stdout.trim(), output.stderr.trim()) {
        ("", "") => String::new(),
        (stdout, "") => stdout.to_string(),
        ("", stderr) => stderr.to_string(),
        (stdout, stderr) => format!("{stdout}\n{stderr}"),
    }
}

fn binary_state(binary: &str) -> String {
    match Command::new(binary).arg("--version").output() {
        Ok(output) if output.status.success() => first_line(&output.stdout, &output.stderr),
        Ok(_) => "present but version check failed".into(),
        Err(_) => "missing".into(),
    }
}

fn first_line(stdout: &[u8], stderr: &[u8]) -> String {
    let text = if stdout.is_empty() { stderr } else { stdout };
    String::from_utf8_lossy(text)
        .lines()
        .next()
        .unwrap_or("present")
        .to_string()
}

fn path_state(path: &Path) -> String {
    if path.exists() {
        format!("present ({})", path.display())
    } else {
        format!("missing ({})", path.display())
    }
}

fn tradingview_mcp_state(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) if text.contains("\"tradingview\"") && text.contains("src/server.js") => {
            "configured".into()
        }
        Ok(_) => "present but no tradingview server entry".into(),
        Err(_) => "missing".into(),
    }
}

fn shell_path(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') {
        format!("'{text}'")
    } else {
        text
    }
}
