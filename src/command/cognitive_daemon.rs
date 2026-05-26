use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use archon_cognitive::{CognitiveDaemon, DaemonState, DaemonStatus, PersistentCognitiveStore};
use archon_core::config::ArchonConfig;
use serde_json::json;

use crate::cli_args::CognitiveDaemonAction;

pub(crate) async fn handle_daemon_action(
    action: &CognitiveDaemonAction,
    config: &ArchonConfig,
    cwd: &Path,
) -> Result<()> {
    match action {
        CognitiveDaemonAction::Start { interval_ms, json } => {
            start(config, cwd, *interval_ms, *json)
        }
        CognitiveDaemonAction::Run { interval_ms, json } => run(config, cwd, *interval_ms, *json),
        CognitiveDaemonAction::RunOnce { json } => run_once(config, cwd, *json),
        CognitiveDaemonAction::Stop { json } => stop(config, cwd, *json),
        CognitiveDaemonAction::Status { json } => status(config, cwd, *json),
    }
}

fn start(config: &ArchonConfig, cwd: &Path, interval_ms: Option<u64>, as_json: bool) -> Result<()> {
    ensure_daemon_config(config)?;
    ensure_daemon_policy(cwd)?;
    let root = cognitive_root(cwd, config);
    let status =
        CognitiveDaemon::status(&root, config.learning.cognitive.daemon.stale_heartbeat_ms)?;
    if status.running {
        anyhow::bail!("cognitive daemon is already running");
    }
    let child = spawn_daemon_child(cwd, interval_ms)?;
    if as_json {
        println!(
            "{}",
            json!({
                "started": true,
                "pid": child.id(),
                "statePath": root.join("cognitive-daemon-state.json"),
            })
        );
    } else {
        println!("Cognitive daemon started (pid {}).", child.id());
        println!(
            "State: {}",
            root.join("cognitive-daemon-state.json").display()
        );
    }
    Ok(())
}

fn ensure_daemon_config(config: &ArchonConfig) -> Result<()> {
    if config.learning.cognitive.daemon.enabled {
        Ok(())
    } else {
        anyhow::bail!("learning.cognitive.daemon.enabled is false")
    }
}

fn run(config: &ArchonConfig, cwd: &Path, interval_ms: Option<u64>, as_json: bool) -> Result<()> {
    let store_root = cognitive_root(cwd, config);
    let store = PersistentCognitiveStore::open(&store_root)?;
    let policy = load_cognitive_policy(cwd)?;
    let mut daemon_config = config.learning.cognitive.daemon.clone();
    apply_interval_override(&mut daemon_config, interval_ms);
    let mut daemon = CognitiveDaemon::new(&store_root, daemon_config, store.db(), policy);
    let state = daemon.run_forever()?;
    print_state(&state, as_json)
}

fn run_once(config: &ArchonConfig, cwd: &Path, as_json: bool) -> Result<()> {
    let store_root = cognitive_root(cwd, config);
    let store = PersistentCognitiveStore::open(&store_root)?;
    let policy = load_cognitive_policy(cwd)?;
    let mut daemon = CognitiveDaemon::new(
        &store_root,
        config.learning.cognitive.daemon.clone(),
        store.db(),
        policy,
    );
    let state = daemon.run_once()?;
    print_state(&state, as_json)
}

fn stop(config: &ArchonConfig, cwd: &Path, as_json: bool) -> Result<()> {
    let root = cognitive_root(cwd, config);
    CognitiveDaemon::request_stop(&root)?;
    if as_json {
        println!("{}", json!({ "stop_requested": true }));
    } else {
        println!("Cognitive daemon stop requested.");
    }
    Ok(())
}

fn status(config: &ArchonConfig, cwd: &Path, as_json: bool) -> Result<()> {
    let daemon_config = &config.learning.cognitive.daemon;
    let root = cognitive_root(cwd, config);
    let status = CognitiveDaemon::status(&root, daemon_config.stale_heartbeat_ms)?;
    print_status(&status, as_json)
}

fn spawn_daemon_child(cwd: &Path, interval_ms: Option<u64>) -> Result<std::process::Child> {
    let exe = std::env::current_exe().context("resolve current archon executable")?;
    let mut command = Command::new(exe);
    command
        .arg("cognitive")
        .arg("daemon")
        .arg("run")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(interval_ms) = interval_ms {
        command.arg("--interval-ms").arg(interval_ms.to_string());
    }
    command.spawn().context("spawn cognitive daemon")
}

fn ensure_daemon_policy(cwd: &Path) -> Result<()> {
    let policy = load_cognitive_policy(cwd)?;
    if policy.can_run_daemon() {
        Ok(())
    } else {
        anyhow::bail!("background daemon denied by policy.cognitive")
    }
}

fn load_cognitive_policy(cwd: &Path) -> Result<archon_policy::CognitivePolicy> {
    Ok(archon_policy::load_effective_policy(cwd)
        .map(|policy| policy.cognitive)
        .unwrap_or_default())
}

fn apply_interval_override(
    config: &mut archon_cognitive::CognitiveDaemonConfig,
    interval_ms: Option<u64>,
) {
    if let Some(interval_ms) = interval_ms {
        config.interval_ms = interval_ms;
    }
    let mut warnings = Vec::new();
    config.validate_and_normalize(&mut warnings);
}

fn cognitive_root(cwd: &Path, config: &ArchonConfig) -> PathBuf {
    expand_path(cwd, &config.learning.cognitive.ledger_dir)
}

fn expand_path(cwd: &Path, raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn print_state(state: &DaemonState, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(state)?);
    } else {
        println!(
            "Cognitive daemon {} pid={} ticks={} last_tick={}",
            state.status,
            state.pid,
            state.ticks_run,
            state
                .last_tick_at
                .map(|time| time.to_rfc3339())
                .unwrap_or_else(|| "never".into())
        );
    }
    Ok(())
}

fn print_status(status: &DaemonStatus, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(status)?);
    } else {
        println!("Cognitive daemon");
        println!("Running: {}", status.running);
        println!("Stale: {}", status.stale);
        println!("Stop requested: {}", status.stop_requested);
        println!("State: {}", status.state_path.display());
        println!("Lock: {}", status.lock_path.display());
        if let Some(state) = &status.state {
            println!("PID: {}", state.pid);
            println!("Ticks run: {}", state.ticks_run);
            println!("Last heartbeat: {}", state.last_heartbeat_at);
        }
    }
    Ok(())
}
