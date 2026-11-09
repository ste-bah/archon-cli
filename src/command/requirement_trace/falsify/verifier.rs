//! Vet and run a declared verifier without changing its meaning.
//!
//! A broad run once exhausted a 466 GB disk twice. Configured refusal tokens
//! bound breadth; time and free-space budgets bound every admitted command.
//! Pipes are drained concurrently so output cannot deadlock the deadline.
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::limits::{DiskFloor, VerifyLimits, disk_floor, watched};
use super::process_tree::Tree;
use archon_knowledge::traceability::RefusedToRun;

const SHELL_METACHARACTERS: &[char] = &['&', '|', ';', '<', '>', '$', '`', '\n', '\r'];

pub(super) fn vet(command: &str, limits: &VerifyLimits) -> Result<Vec<String>, RefusedToRun> {
    if let Some(found) = command.chars().find(|c| SHELL_METACHARACTERS.contains(c)) {
        return Err(RefusedToRun::NotDirectlyExecutable {
            command: command.to_string(),
            reason: format!("it contains `{found}`"),
        });
    }
    let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        return Err(RefusedToRun::NotDirectlyExecutable {
            command: command.to_string(),
            reason: "it is empty".into(),
        });
    }
    if let Some(token) = limits.refuse_tokens.iter().find(|token| {
        argv.iter().any(|word| {
            word == *token
                || word
                    .strip_prefix(token.as_str())
                    .is_some_and(|rest| rest.starts_with('='))
        })
    }) {
        return Err(RefusedToRun::WorkspaceWideCommand {
            command: command.into(),
            token: token.clone(),
        });
    }
    Ok(argv)
}

#[derive(Debug)]
pub(super) enum Ran {
    Finished {
        code: Option<i32>,
        success: bool,
        output: String,
        note: Option<String>,
    },
    Interrupted {
        signal: i32,
    },
    TimedOut {
        seconds: u64,
    },
    DiskFloor(DiskFloor),
    DiskFloorBeforeLaunch(DiskFloor),
    NotLaunchable {
        reason: String,
    },
}

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const DISK_INTERVAL: Duration = Duration::from_secs(5);

#[cfg(all(test, unix))]
pub(super) fn run(cwd: &Path, argv: &[String], timeout: Duration) -> Ran {
    run_with_policy(cwd, argv, timeout, None)
}

#[cfg(all(test, unix))]
pub(super) fn run_with_policy(
    cwd: &Path,
    argv: &[String],
    timeout: Duration,
    policy: Option<&archon_workflow::acceptance_check_environment::CheckPolicy>,
) -> Ran {
    let environment = match archon_workflow::acceptance_check_environment::CommandEnvironment::capture(policy) {
        Ok(environment) => environment.with_remedy("Name the needed variables in [workflow.acceptance_execution] environment_allowlist in the operator configuration consumed by requirements trace, then retry --falsify"),
        Err(reason) => return Ran::NotLaunchable { reason: format!("`{}`: {reason}", argv.join(" ")) },
    };
    let limits = VerifyLimits {
        timeout,
        ..VerifyLimits::for_test()
    };
    run_in_environment(cwd, argv, &limits, &environment)
}

pub(super) fn run_in_environment(
    cwd: &Path,
    argv: &[String],
    limits: &VerifyLimits,
    environment: &archon_workflow::acceptance_check_environment::CommandEnvironment,
) -> Ran {
    let result = invoke(cwd, argv, limits, environment);
    result.unwrap_or_else(|error| Ran::NotLaunchable {
        reason: format!("`{}`: {error}", argv.join(" ")),
    })
}

fn invoke(
    cwd: &Path,
    argv: &[String],
    limits: &VerifyLimits,
    environment: &archon_workflow::acceptance_check_environment::CommandEnvironment,
) -> std::io::Result<Ran> {
    let program = argv
        .first()
        .ok_or_else(|| std::io::Error::other("empty verifier argument list"))?;
    let mut process = environment.command(program);
    process
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let paths = watched(cwd);
    if let Some(floor) = disk_floor(&paths, limits)? {
        return Ok(Ran::DiskFloorBeforeLaunch(floor));
    }
    let mut signals = super::signals::Monitor::new()?;
    Tree::prepare(&mut process)?;
    archon_shell::read_boundary::prepare_spawn(&process)?;
    let mut child = process.spawn()?;
    let mut tree = match Tree::adopt(&mut child) {
        Ok(tree) => tree,
        Err(error) => {
            Tree::abort(&mut child);
            let _ = child.wait();
            return Err(error);
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || drain(stdout));
    let err_reader = std::thread::spawn(move || drain(stderr));
    let started = Instant::now();
    let mut disk_checked = started;
    let stopped = (|| -> std::io::Result<Option<Ran>> {
        loop {
            if let Some(signal) = signals.pending() {
                return Ok(Some(Ran::Interrupted { signal }));
            }
            if tree.exited(&mut child)? {
                return Ok(None);
            }
            if started.elapsed() >= limits.timeout {
                return Ok(Some(Ran::TimedOut {
                    seconds: limits.timeout.as_secs(),
                }));
            }
            if disk_checked.elapsed() >= DISK_INTERVAL {
                if let Some(floor) = disk_floor(&paths, limits)? {
                    return Ok(Some(Ran::DiskFloor(floor)));
                }
                disk_checked = Instant::now();
            }
            tree.scan()?;
            std::thread::sleep(POLL_INTERVAL);
        }
    })();
    // Tear down descendants even when the leader finished normally, before
    // reaping releases its identity and before joining the pipe readers.
    let cleanup = tree.kill();
    if cleanup.is_err() {
        Tree::abort(&mut child);
    }
    let status = child.wait();
    tree.reaped();
    let restored = signals.restore();
    // Include signals delivered after leader exit and during restoration.
    if let Some(signal) = signals.pending() {
        if let Err(error) = cleanup {
            eprintln!("interrupted verifier teardown failed: {error}");
        }
        if let Err(error) = restored {
            eprintln!("interrupted verifier handler restoration failed: {error}");
        }
        if !out_reader.is_finished() || !err_reader.is_finished() {
            eprintln!("interrupted verifier output readers still running; detaching");
        }
        return Ok(Ran::Interrupted { signal });
    }
    cleanup?;
    restored?;
    let status = status?;
    let stdout = out_reader
        .join()
        .map_err(|_| std::io::Error::other("output reader thread failed"))??;
    let stderr = err_reader
        .join()
        .map_err(|_| std::io::Error::other("error reader thread failed"))??;
    if let Some(stopped) = stopped? {
        return Ok(stopped);
    }
    let note = (!status.success())
        .then(|| environment.note(&[stdout.as_bytes(), stderr.as_bytes()]))
        .flatten();
    let success = matches!(
        archon_workflow::acceptance_check_crash::classify_execution(
            &argv.join(" "),
            status.code(),
            &stderr,
            &[]
        ),
        archon_workflow::acceptance_check_crash::CheckRunClass::Passed
    );
    Ok(Ran::Finished {
        code: status.code(),
        success,
        output: stdout + stderr.as_str(),
        note,
    })
}

fn drain<R: std::io::Read>(pipe: Option<R>) -> std::io::Result<String> {
    let mut raw = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut raw)?;
    }
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

#[cfg(test)]
#[path = "verifier_agnostic_proof_tests.rs"]
mod agnostic_proof_tests;
#[cfg(test)]
#[path = "verifier_environment_tests.rs"]
mod environment_tests;
#[cfg(test)]
#[path = "verifier_limits_tests.rs"]
mod limits_tests;

#[cfg(all(test, unix))]
#[path = "verifier_signal_tests.rs"]
mod signal_tests;
