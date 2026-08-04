//! Vetting a declared verifier, and running it without a shell.
//!
//! # What this refuses to run, and why the list is short
//!
//! NFR-004 forbids whole-workspace test runs; the one that prompted the rule
//! exhausted a 466 GB disk twice. So a declared command that is workspace-wide
//! is refused rather than trimmed — rewriting someone's `cargo test --workspace`
//! into `cargo test -p something` would run a *different* verifier from the one
//! the plan names, and the plan's criterion is about the command it names.
//!
//! The second refusal is quieter and matters as much: commands are run as an
//! argv, never through a shell. A declared verifier containing a pipe, a
//! redirect or an `&&` does not mean what it says when its tokens are handed
//! straight to `CreateProcess`, so it is refused instead of being run with its
//! meaning silently changed.
//!
//! # Why the child is polled rather than waited on
//!
//! `std::process` has no timeout. A verifier that hangs while a mutation is in
//! the working tree is the worst state this code can reach, so the child is
//! polled to a deadline and killed. Its pipes are drained by two threads for the
//! whole time: with `Stdio::piped()` and no reader, a chatty build fills the
//! pipe buffer and blocks forever, which is a deadlock the deadline would never
//! see because the deadline is not what the child is waiting on.
//!
//! Killing the child does not kill its grandchildren. `cargo` spawns `rustc` and
//! test binaries, and on Windows those survive their parent. The restore still
//! runs; if a surviving compiler holds the file open, the write fails and
//! [`super::guard::MutationGuard`] says so rather than reporting a clean run.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use archon_knowledge::traceability::RefusedToRun;

/// Tokens that make a command workspace-wide whatever else it says.
const WORKSPACE_WIDE_TOKENS: &[&str] = &["--workspace", "--all"];

/// Flags that scope a cargo test invocation to something smaller than
/// everything. An explicit list rather than a heuristic, for the same reason
/// `KNOWN_RUNNERS` is one: extend it when a real task declares a real narrowing
/// flag that is missing, do not replace it with a guess.
const NARROWING_FLAGS: &[&str] = &[
    "-p",
    "--package",
    "--bin",
    "--lib",
    "--test",
    "--bench",
    "--example",
    "--doc",
];

/// Cargo subcommands that build and run the world when nothing narrows them.
const UNSCOPED_CARGO_SUBCOMMANDS: &[&str] = &["test", "bench", "nextest"];

/// Characters that only mean anything to a shell.
const SHELL_METACHARACTERS: &[char] = &['&', '|', ';', '<', '>', '$', '`', '\n', '\r'];

/// Split a declared command into an argv, or refuse it.
pub(super) fn vet(command: &str) -> std::result::Result<Vec<String>, RefusedToRun> {
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
            reason: "it is empty".to_string(),
        });
    }

    if let Some(token) = argv
        .iter()
        .find(|token| WORKSPACE_WIDE_TOKENS.contains(&token.as_str()))
    {
        return Err(RefusedToRun::WorkspaceWideCommand {
            command: command.to_string(),
            token: token.clone(),
        });
    }

    // `cargo test` with nothing narrowing it is workspace-wide in effect even
    // though it never says so, which is exactly the shape NFR-004 is about.
    let unscoped_cargo = argv[0] == "cargo"
        && argv
            .get(1)
            .is_some_and(|sub| UNSCOPED_CARGO_SUBCOMMANDS.contains(&sub.as_str()))
        && !argv
            .iter()
            .any(|token| NARROWING_FLAGS.contains(&token.as_str()));
    if unscoped_cargo {
        return Err(RefusedToRun::WorkspaceWideCommand {
            command: command.to_string(),
            token: "no --package/--bin/--test scope".to_string(),
        });
    }

    Ok(argv)
}

/// What one verifier invocation did.
pub(super) enum Ran {
    Finished {
        code: Option<i32>,
        success: bool,
        output: String,
    },
    /// Killed at the deadline. Not a failure — an absence of an answer.
    TimedOut {
        seconds: u64,
    },
    NotLaunchable {
        reason: String,
    },
}

/// How often the child is checked. Short enough that the deadline is honoured
/// promptly, long enough that polling costs nothing next to a compile.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub(super) fn run(cwd: &Path, argv: &[String], timeout: Duration) -> Ran {
    let spawned = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            return Ran::NotLaunchable {
                reason: format!("could not start `{}`: {err}", argv.join(" ")),
            };
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || drain(stdout));
    let err_reader = std::thread::spawn(move || drain(stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Err(err) => {
                let _ = child.kill();
                return Ran::NotLaunchable {
                    reason: format!("waiting on `{}`: {err}", argv[0]),
                };
            }
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let mut output = out_reader.join().unwrap_or_default();
    output.push_str(&err_reader.join().unwrap_or_default());

    match status {
        None => Ran::TimedOut {
            seconds: timeout.as_secs(),
        },
        Some(status) => Ran::Finished {
            code: status.code(),
            success: status.success(),
            output,
        },
    }
}

/// Read a pipe to EOF. Lossy on purpose: compiler output is not guaranteed
/// UTF-8, and the only thing done with it is a substring search for a build
/// marker, which a replacement character cannot fake.
fn drain<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut raw = Vec::new();
    let _ = pipe.read_to_end(&mut raw);
    String::from_utf8_lossy(&raw).into_owned()
}
