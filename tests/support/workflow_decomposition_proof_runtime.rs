#![allow(dead_code)]
use super::*;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

pub fn standard_runtime_binary(source_project: &Path) -> PathBuf {
    source_project.join("archon")
}

/// The `archon` a shell would run, if there is one.
///
/// Deployment here is project-scoped: the only binary that has to exist is the
/// one inside the project. A copy elsewhere is something that *may* exist, not
/// something that should, so this resolves whatever `PATH` actually points at
/// rather than naming a directory. Hardcoding one made the proof demand an
/// install this repository does not use, and the two drifted apart -- so the
/// binary check would have refused the proof even after the preflight and
/// provider configuration were fixed.
///
/// `None` is a correct and expected result. If a copy does exist it still has
/// to match, so a stale build cannot be mistaken for the one under test.
pub fn standard_deployed_peer() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("archon"))
        .find(|candidate| candidate.is_file())
}

/// True when no peer exists, or one exists and matches.
pub fn deployed_binaries_agree(runtime: &Path, peer: Option<&Path>) -> bool {
    peer.is_none_or(|peer| require_matching_binaries(runtime, peer).is_ok())
}

pub fn source_revision(source_project: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(source_project)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("reading source revision: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if revision.is_empty() {
        return Err("source revision is empty".into());
    }
    Ok(revision)
}

pub fn deployed_runtime_identity(
    source_revision: String,
    binary: &Path,
) -> Result<RuntimeIdentity, String> {
    let bytes = std::fs::read(binary)
        .map_err(|error| format!("reading runtime binary {}: {error}", binary.display()))?;
    let version = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| format!("reading runtime version: {error}"))?;
    if !version.status.success() {
        return Err("runtime --version returned nonzero".into());
    }
    let text = String::from_utf8_lossy(&version.stdout);
    let binary_revision = text
        .split_once('(')
        .and_then(|(_, tail)| tail.split_once(')'))
        .map(|(value, _)| value.trim().to_string())
        .ok_or_else(|| format!("runtime version carried no revision: {text}"))?;
    if !source_revision.starts_with(&binary_revision) {
        return Err(format!(
            "source revision {source_revision} does not match deployed binary revision {binary_revision}"
        ));
    }
    let identity = Command::new(binary)
        .args(["workflow", "decomposition-identity"])
        .env_remove("ANTHROPIC_BASE_URL")
        .output()
        .map_err(|error| format!("reading decomposition identity: {error}"))?;
    if !identity.status.success() {
        return Err(format!(
            "decomposition identity returned nonzero: {}",
            String::from_utf8_lossy(&identity.stderr)
        ));
    }
    let fixed: serde_json::Value = serde_json::from_slice(&identity.stdout)
        .map_err(|error| format!("parsing decomposition identity: {error}"))?;
    let embedded_revision = required_json_text(&fixed, &["binary_revision"])?;
    if embedded_revision != binary_revision {
        return Err("runtime version and decomposition identity revisions differ".into());
    }
    Ok(RuntimeIdentity {
        source_revision,
        binary_sha256: sha256(&bytes),
        binary_revision,
        script_digest: required_json_text(&fixed, &["script_digest"])?,
        catalog_digest: required_json_text(&fixed, &["catalog_digest"])?,
    })
}

pub fn runtime_matches_clearance(
    clearance: &SyntheticClearance,
    source_revision: &str,
    binary: &Path,
) -> Result<(), String> {
    let current = deployed_runtime_identity(source_revision.to_string(), binary)?;
    require_clearance_identity(clearance, &current)
}

pub fn command_output(binary: &Path, cwd: &Path, action: &ProofAction) -> Result<Output, String> {
    let mut command = Command::new(binary);
    command
        .current_dir(cwd)
        .args(action.args())
        .env_remove("ANTHROPIC_BASE_URL")
        .env(PROOF_ENV_CANARY_NAME, PROOF_ENV_CANARY_VALUE)
        .env(PROOF_SECRET_CANARY_NAME, PROOF_SECRET_CANARY_VALUE);
    command
        .output()
        .map_err(|error| format!("spawning {}: {error}", binary.display()))
}

pub fn require_success(output: &Output, label: &str) -> Result<String, String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "{label} failed with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        ));
    }
    Ok(format!("{stdout}{stderr}"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProofProcessInventory {
    pub conflicts: Vec<String>,
    pub idle_tui_count: usize,
}

#[derive(Debug)]
struct ProcessRow {
    pid: u32,
    ppid: u32,
    comm: String,
    args: String,
}

pub fn proof_process_inventory() -> Result<ProofProcessInventory, String> {
    let output = Command::new("ps")
        .args(["-Ao", "pid=,ppid=,comm=,args="])
        .output()
        .map_err(|error| format!("process inventory failed: {error}"))?;
    if !output.status.success() {
        return Err("process inventory returned nonzero".into());
    }
    classify_process_inventory(&String::from_utf8_lossy(&output.stdout), std::process::id())
}

pub(crate) fn classify_process_inventory(
    text: &str,
    current_pid: u32,
) -> Result<ProofProcessInventory, String> {
    let mut inventory = ProofProcessInventory::default();
    let rows: Vec<_> = text.lines().filter_map(parse_process_row).collect();
    // The invocation running this proof is not competing work. Excluding only
    // our own pid left the `cargo` that spawned the test binary in the
    // inventory, so the preflight refused every run of the proof it exists to
    // guard -- the check could never pass when invoked as documented.
    let mut ancestors = std::collections::BTreeSet::from([current_pid]);
    let parents: std::collections::BTreeMap<u32, u32> =
        rows.iter().map(|row| (row.pid, row.ppid)).collect();
    let mut cursor = current_pid;
    while let Some(&parent) = parents.get(&cursor) {
        if parent == 0 || !ancestors.insert(parent) {
            break;
        }
        cursor = parent;
    }
    for row in rows {
        if ancestors.contains(&row.pid) {
            continue;
        }
        let executable = std::path::Path::new(&row.comm)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(row.comm.as_str())
            .to_ascii_lowercase();
        let argv = row.args.split_whitespace().collect::<Vec<_>>();
        if executable == "archon" {
            let trailing = argv.iter().skip(1).copied().collect::<Vec<_>>();
            let flag_only = trailing.iter().all(|value| value.starts_with('-'));
            if flag_only {
                inventory.idle_tui_count += 1;
            } else {
                inventory.conflicts.push(format!(
                    "pid={} ppid={} executable={} argv={}",
                    row.pid, row.ppid, row.comm, row.args
                ));
            }
            continue;
        }
        let compiler_or_linker = matches!(
            executable.as_str(),
            "cargo" | "rustc" | "rustdoc" | "clippy-driver" | "ld" | "ld64" | "clang" | "cc"
        );
        let rust_test_harness = (row.args.contains("/target/")
            || row.args.contains("archon-r2a-target/debug/deps"))
            && (row.args.contains("--test-threads")
                || executable.starts_with("workflow_")
                || executable.starts_with("archon-"));
        if compiler_or_linker || rust_test_harness {
            inventory.conflicts.push(format!(
                "pid={} ppid={} executable={} argv={}",
                row.pid, row.ppid, row.comm, row.args
            ));
        }
    }
    Ok(inventory)
}

fn parse_process_row(line: &str) -> Option<ProcessRow> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    let ppid = fields.next()?.parse().ok()?;
    let comm = fields.next()?.to_string();
    let args = fields.collect::<Vec<_>>().join(" ");
    Some(ProcessRow {
        pid,
        ppid,
        comm,
        args,
    })
}

pub fn require_process_guard_zero() -> Result<(), String> {
    let inventory = proof_process_inventory()?;
    if inventory.conflicts.is_empty() && inventory.idle_tui_count == 0 {
        return Ok(());
    }
    Err(format!(
        "proof preflight found conflicting work or idle TUI: conflicts={} idle_tuis={}",
        inventory.conflicts.join(" | "),
        inventory.idle_tui_count
    ))
}

pub fn require_observe_config(project: &Path) -> Result<(), String> {
    for path in [
        project.join(".archon/config.local.toml"),
        project.join(".archon/config.toml"),
    ] {
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("reading {}: {error}", path.display()))?;
        let value: toml::Value = text
            .parse()
            .map_err(|error| format!("parsing {}: {error}", path.display()))?;
        if let Some(mode) = value
            .get("workflow")
            .and_then(|value| value.get("gate_mode"))
            .and_then(toml::Value::as_str)
        {
            return if mode == "observe" {
                Ok(())
            } else {
                Err(format!(
                    "effective proof workflow.gate_mode is '{mode}' from {}",
                    path.display()
                ))
            };
        }
    }
    Err("proof requires project workflow.gate_mode = \"observe\"".into())
}

pub fn wait_for_new_fixed_run(
    project: &Path,
    existing: &BTreeSet<String>,
    timeout: Duration,
) -> Result<String, String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let deadline = Instant::now() + timeout;
    loop {
        for run in store.list_runs().map_err(|error| error.to_string())? {
            if existing.contains(&run.id) {
                continue;
            }
            let path = store.run_dir(&run.id).join("decomposition/state.json");
            if path.exists() {
                return Ok(run.id);
            }
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for a newly persisted fixed decomposition run".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn wait_for_event(
    project: &Path,
    run_id: &str,
    event_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_typed_event(project, run_id, event_name, &[], timeout)
}

pub fn wait_for_event_line(
    project: &Path,
    run_id: &str,
    detail_markers: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let (kind, detail) = detail_markers
        .split_first()
        .ok_or_else(|| "typed event wait requires a kind".to_string())?;
    wait_for_typed_event(project, run_id, kind, detail, timeout)
}

fn wait_for_typed_event(
    project: &Path,
    run_id: &str,
    kind: &str,
    detail_markers: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let deadline = Instant::now() + timeout;
    loop {
        let events = parse_json_lines(&store.events_path(run_id)).unwrap_or_default();
        if events.iter().any(|event| {
            event["kind"] == kind
                && detail_markers
                    .iter()
                    .all(|marker| event["detail"].to_string().contains(marker))
        }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for typed durable event {kind} {detail_markers:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn spawn_action(
    binary: &Path,
    cwd: &Path,
    action: &ProofAction,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<std::process::Child, String> {
    for path in [stdout_path, stderr_path] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    let stdout = std::fs::File::create(stdout_path).map_err(|error| error.to_string())?;
    let stderr = std::fs::File::create(stderr_path).map_err(|error| error.to_string())?;
    Command::new(binary)
        .current_dir(cwd)
        .args(action.args())
        .env_remove("ANTHROPIC_BASE_URL")
        .env(PROOF_ENV_CANARY_NAME, PROOF_ENV_CANARY_VALUE)
        .env(PROOF_SECRET_CANARY_NAME, PROOF_SECRET_CANARY_VALUE)
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| format!("spawning {}: {error}", binary.display()))
}

pub fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let id = child.id();
            child
                .kill()
                .map_err(|error| format!("terminating timed-out child {id}: {error}"))?;
            child
                .wait()
                .map_err(|error| format!("reaping timed-out child {id}: {error}"))?;
            return Err(format!("child {id} exceeded proof watchdog and was reaped"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn wait_for_terminal_run(
    project: &Path,
    run_id: &str,
    timeout: Duration,
) -> Result<archon_workflow::WorkflowRun, String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let deadline = Instant::now() + timeout;
    loop {
        let run = store
            .load_state(run_id)
            .map_err(|error| error.to_string())?;
        if matches!(
            run.status,
            archon_workflow::RunStatus::Completed
                | archon_workflow::RunStatus::NeedsReview
                | archon_workflow::RunStatus::Blocked
                | archon_workflow::RunStatus::Failed
                | archon_workflow::RunStatus::Cancelled
        ) {
            return Ok(run);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for terminal run {run_id}"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub fn runtime_identity_from_fixed_run(
    source_revision: String,
    binary: &Path,
    project: &Path,
    run_id: &str,
) -> Result<RuntimeIdentity, String> {
    let current = deployed_runtime_identity(source_revision, binary)?;
    let store = archon_workflow::WorkflowStore::project(project);
    let fixed: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json"))
            .map_err(|error| format!("reading fixed identity: {error}"))?,
    )
    .map_err(|error| format!("parsing fixed identity: {error}"))?;
    let persisted_revision = required_json_text(&fixed, &["identity", "starting_binary_revision"])?;
    let persisted_script = required_json_text(&fixed, &["identity", "script_digest"])?;
    let persisted_catalog = required_json_text(&fixed, &["identity", "catalog_digest"])?;
    if persisted_revision != current.binary_revision
        || persisted_script != current.script_digest
        || persisted_catalog != current.catalog_digest
    {
        return Err("persisted fixed-run identity differs from deployed runtime".into());
    }
    Ok(current)
}

pub fn require_matching_binaries(first: &Path, second: &Path) -> Result<(), String> {
    let left =
        std::fs::read(first).map_err(|error| format!("reading {}: {error}", first.display()))?;
    let right =
        std::fs::read(second).map_err(|error| format!("reading {}: {error}", second.display()))?;
    if sha256(&left) != sha256(&right) {
        return Err("deployed Archon binaries differ".into());
    }
    Ok(())
}

pub fn current_run_ids(project: &Path) -> Result<BTreeSet<String>, String> {
    archon_workflow::WorkflowStore::project(project)
        .list_runs()
        .map(|runs| runs.into_iter().map(|run| run.id).collect())
        .map_err(|error| error.to_string())
}

pub fn parse_json_lines(path: &Path) -> Result<Vec<serde_json::Value>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect()
}

fn required_json_text(value: &serde_json::Value, path: &[&str]) -> Result<String, String> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| format!("missing JSON path {}", path.join(".")))?;
    }
    current
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("JSON path {} is not nonempty text", path.join(".")))
}
