use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use archon_cognitive::{
    CognitiveDaemon, CognitiveError, DaemonJob, DaemonJobReport, DaemonState, DaemonStatus,
    PersistentCognitiveStore,
};
use archon_core::config::ArchonConfig;
use serde_json::json;

use crate::cli_args::CognitiveDaemonAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DaemonStartOutcome {
    Disabled,
    PolicyDenied(String),
    AlreadyRunning { state_path: PathBuf },
    Started { pid: u32, state_path: PathBuf },
}

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

pub(crate) fn ensure_daemon_started(
    config: &ArchonConfig,
    cwd: &Path,
) -> Result<DaemonStartOutcome> {
    if !config.learning.cognitive.daemon.enabled {
        return Ok(DaemonStartOutcome::Disabled);
    }
    if let Err(error) = ensure_daemon_policy(cwd) {
        return Ok(DaemonStartOutcome::PolicyDenied(error.to_string()));
    }
    let root = cognitive_root(cwd, config);
    let status =
        CognitiveDaemon::status(&root, config.learning.cognitive.daemon.stale_heartbeat_ms)?;
    if status.running {
        return Ok(DaemonStartOutcome::AlreadyRunning {
            state_path: status.state_path,
        });
    }
    let mut child = spawn_daemon_child(cwd, None)?;
    let status = wait_for_daemon_running(
        &root,
        config.learning.cognitive.daemon.stale_heartbeat_ms,
        &mut child,
    )?;
    Ok(DaemonStartOutcome::Started {
        pid: child.id(),
        state_path: status.state_path,
    })
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
    let mut child = spawn_daemon_child(cwd, interval_ms)?;
    let status = wait_for_daemon_running(
        &root,
        config.learning.cognitive.daemon.stale_heartbeat_ms,
        &mut child,
    )?;
    if as_json {
        println!(
            "{}",
            json!({
                "started": true,
                "pid": child.id(),
                "statePath": status.state_path,
            })
        );
    } else {
        println!("Cognitive daemon started (pid {}).", child.id());
        println!("State: {}", status.state_path.display());
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
    add_deferred_retry_jobs(&mut daemon);
    crate::command::cognitive_daemon_learning::add_learning_jobs(
        &mut daemon,
        config.clone(),
        cwd.to_path_buf(),
        store_root.clone(),
    );
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
    add_deferred_retry_jobs(&mut daemon);
    crate::command::cognitive_daemon_learning::add_learning_jobs(
        &mut daemon,
        config.clone(),
        cwd.to_path_buf(),
        store_root.clone(),
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
    print_status(&status, as_json, &root)
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
    detach_daemon_child(&mut command);
    if let Some(interval_ms) = interval_ms {
        command.arg("--interval-ms").arg(interval_ms.to_string());
    }
    command.spawn().context("spawn cognitive daemon")
}

#[cfg(unix)]
fn detach_daemon_child(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: pre_exec runs after fork and before exec. setsid is async-signal-safe
    // and detaches the daemon from the launcher's process group/session.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_daemon_child(_command: &mut Command) {}

fn wait_for_daemon_running(root: &Path, stale_ms: u64, child: &mut Child) -> Result<DaemonStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = CognitiveDaemon::status(root, stale_ms)?;
        if status.running
            && status
                .state
                .as_ref()
                .is_some_and(|state| state.pid == child.id())
        {
            return Ok(status);
        }
        if let Some(exit) = child.try_wait().context("poll cognitive daemon startup")? {
            anyhow::bail!("cognitive daemon exited during startup with {exit}");
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "cognitive daemon did not become ready within 5s; state path {}",
                status.state_path.display()
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
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

fn add_deferred_retry_jobs<'a>(daemon: &mut CognitiveDaemon<'a>) {
    if let Ok(root) = world_model_root() {
        daemon.add_job(WorldModelShadowRetryJob { root });
    }
}

fn world_model_root() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?;
    Ok(home.join(".archon").join("world-model"))
}

struct WorldModelShadowRetryJob {
    root: PathBuf,
}

impl DaemonJob for WorldModelShadowRetryJob {
    fn name(&self) -> &'static str {
        "world_model_shadow_retry"
    }

    fn run(&mut self) -> Result<DaemonJobReport, CognitiveError> {
        use archon_world_model::storage::deferred_retry::ShadowEvidenceRetryOutcome;

        let outcome =
            archon_world_model::storage::deferred_retry::process_shadow_evidence_retry(&self.root)
                .map_err(|error| CognitiveError::Store(format!("world-model retry: {error}")))?;
        let summary = match outcome {
            ShadowEvidenceRetryOutcome::NoPending => "no pending world-model retry".to_owned(),
            ShadowEvidenceRetryOutcome::Resolved {
                attempts,
                rows_loaded,
                ..
            } => {
                format!("resolved after {attempts} attempt(s); rows_loaded={rows_loaded}")
            }
            ShadowEvidenceRetryOutcome::StillPending {
                attempts,
                last_error,
                ..
            } => {
                format!("still pending after {attempts} attempt(s): {last_error}")
            }
        };
        Ok(DaemonJobReport {
            name: self.name().into(),
            ok: true,
            summary,
        })
    }
}

fn expand_path(cwd: &Path, raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(stripped);
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

fn print_status(status: &DaemonStatus, as_json: bool, root: &Path) -> Result<()> {
    if as_json {
        let mut value = serde_json::to_value(status)?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "learningJobs".into(),
                serde_json::to_value(crate::command::cognitive_daemon_learning_ledger::latest(
                    root, 10,
                ))?,
            );
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
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
            if let Some(job) = &state.current_job {
                println!("Current job: {job}");
            }
            if let Some(started) = state.tick_started_at {
                println!("Tick started: {started}");
            }
            println!("Last heartbeat: {}", state.last_heartbeat_at);
        }
        println!(
            "{}",
            crate::command::cognitive_daemon_learning::render_recent_summary(root)
        );
    }
    Ok(())
}
