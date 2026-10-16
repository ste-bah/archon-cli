use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

pub(crate) use super::trading_venv_health::openbb_health;

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
    let openbb_health = openbb_health(&venv_dir);

    let mut lines = vec![
        "Trading Lab external tools".to_string(),
        format!("  project: {}", root.display()),
        format!("  tools dir: {}", root.join(".archon/tools").display()),
        format!("  node: {}", binary_state("node")),
        format!("  npm: {}", binary_state("npm")),
        format!(
            "  {}: {}",
            python_command_name(),
            binary_state(python_command_name())
        ),
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
        format!("  OpenBB venv: {openbb_health}"),
        format!(
            "  openbb-api: {}",
            if openbb_health.starts_with("healthy") {
                path_state(&openbb_api)
            } else {
                openbb_health.clone()
            }
        ),
        format!(
            "  OpenBB API URL: {}",
            std::env::var("OPENBB_API_URL").unwrap_or_else(|_| "http://127.0.0.1:6900".into())
        ),
    ];

    lines.push("".into());
    lines.push("Setup command:".into());
    lines.push(format!("  {}", setup_command(&root)));
    Ok(lines.join("\n"))
}

pub(crate) fn run_setup_script(
    target: Option<&PathBuf>,
    check: bool,
    dry_run: bool,
    skip_tradingview: bool,
    skip_openbb: bool,
) -> Result<String> {
    let root = project_root(target)?;
    let script = root.join("scripts").join(if cfg!(windows) {
        "setup-trading-tools.ps1"
    } else {
        "setup-trading-tools.sh"
    });
    if !script.is_file() {
        return Err(anyhow!(
            "Trading setup script not found at {}; copy the platform trading setup script into this project or run it from the archon-cli repo",
            script.display()
        ));
    }
    let mut args = vec!["--target".to_string(), root.display().to_string()];
    if check {
        args.push("--check".into());
    }
    if dry_run {
        args.push("--dry-run".into());
    }
    if skip_tradingview {
        args.push("--skip-tradingview".into());
    }
    if skip_openbb {
        args.push("--skip-openbb".into());
    }
    #[cfg(windows)]
    let output = {
        let mut powershell_args = vec![
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            script.display().to_string(),
        ];
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--target" => {
                    powershell_args.push("-Target".into());
                    if let Some(value) = iter.next() {
                        powershell_args.push(value.clone());
                    }
                }
                "--check" => powershell_args.push("-Check".into()),
                "--dry-run" => powershell_args.push("-DryRun".into()),
                "--skip-tradingview" => powershell_args.push("-SkipTradingView".into()),
                "--skip-openbb" => powershell_args.push("-SkipOpenBB".into()),
                other => powershell_args.push(other.into()),
            }
        }
        run_command("powershell.exe", &powershell_args, Some(&root))?
    };
    #[cfg(not(windows))]
    let output = run_command(script.as_path(), &args, Some(&root))?;
    checked_text(output, "setup-trading-tools")
}

pub(crate) fn setup_command(root: &Path) -> String {
    if cfg!(windows) {
        format!(
            "powershell.exe -NoProfile -File scripts/setup-trading-tools.ps1 -Target {}",
            shell_path(root)
        )
    } else {
        format!(
            "scripts/setup-trading-tools.sh --target {}",
            shell_path(root)
        )
    }
}

/// Resolve the project root that owns `.archon/trading-lab`.
///
/// The starting point is taken literally when it already looks like a project
/// root; otherwise this walks up until it finds one. The walk exists because
/// workflow branches execute inside `…/v2/worktrees/<branch>/`, and an agent
/// following a task contract that says `--target .` resolved that to the
/// worktree — which has no `trading-lab`. A live verification reported "the
/// required Polygon dataset/version is absent under target '.'" for a dataset
/// that was present, and the task was partly failed over a file that existed.
///
/// If no ancestor qualifies, the original path is returned unchanged so the
/// caller still fails with a message about the path the operator actually gave,
/// rather than about some ancestor they never mentioned.
pub(crate) fn project_root(target: Option<&PathBuf>) -> Result<PathBuf> {
    let start = match target {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("failed to resolve current directory")?,
    };
    Ok(resolve_trading_lab_root(&start))
}

/// True when `dir` is a project root — it owns the trading-lab data tree.
fn owns_trading_lab(dir: &Path) -> bool {
    dir.join(".archon/trading-lab").is_dir()
}

fn resolve_trading_lab_root(start: &Path) -> PathBuf {
    if owns_trading_lab(start) {
        return start.to_path_buf();
    }
    // `canonicalize` so a relative `.` inside a worktree still has ancestors to
    // walk; fall back to the literal path when it does not yet exist on disk.
    let absolute = start
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| start.to_path_buf());
    absolute
        .ancestors()
        .find(|ancestor| owns_trading_lab(ancestor))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start.to_path_buf())
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
    let executable = if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    openbb_venv_dir(project_root).join(bin_dir).join(executable)
}

fn python_command_name() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
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
    let mut command = archon_shell::spawn::command(program);
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
    match archon_shell::spawn::command(binary)
        .arg("--version")
        .output()
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openbb_python_path_uses_platform_layout_and_name() {
        let root = Path::new("project with spaces");
        let path = openbb_bin(root, "python");
        if cfg!(windows) {
            assert!(path.ends_with("Scripts/python.exe"));
            assert!(openbb_bin(root, "openbb-api").ends_with("Scripts/openbb-api.exe"));
        } else {
            assert!(path.ends_with("bin/python"));
        }
    }

    #[test]
    fn setup_command_selects_the_platform_installer() {
        let command = setup_command(Path::new("project"));
        if cfg!(windows) {
            assert!(command.contains("setup-trading-tools.ps1"));
            assert!(command.contains("-Target project"));
        } else {
            assert!(command.contains("setup-trading-tools.sh"));
            assert!(command.contains("--target project"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn openbb_health_distinguishes_missing_and_broken_interpreters() {
        let temp = tempfile::tempdir().unwrap();
        let venv = temp.path().join("venv space");
        let bin = venv.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let python = bin.join("python");
        std::fs::write(&python, "#!/bin/sh\nexit 17\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(openbb_health(&venv).starts_with("broken"));
        std::fs::remove_file(&python).unwrap();
        assert!(openbb_health(&venv).starts_with("missing"));
        std::os::unix::fs::symlink(bin.join("removed-python"), &python).unwrap();
        assert!(openbb_health(&venv).starts_with("broken"));
    }

    #[cfg(unix)]
    #[test]
    fn openbb_health_accepts_a_working_interpreter_import() {
        let temp = tempfile::tempdir().unwrap();
        let venv = temp.path().join("venv with spaces");
        let bin = venv.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let python = bin.join("python");
        std::fs::write(
            &python,
            "#!/bin/sh\n[ \"$1\" = -c ] && [ \"$2\" = 'import openbb' ]\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(openbb_health(&venv).starts_with("healthy"));
    }
}
