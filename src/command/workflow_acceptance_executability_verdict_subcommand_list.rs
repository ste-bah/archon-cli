//! The commands a tool builds in, as a check's site sees it (Issues 331,
//! 333).
//!
//! `tool --list` runs with the site's own environment -- its search path
//! and the variables its policy binds, such as where a toolchain proxy
//! finds its toolchains -- so a program the tool starts by name (an
//! `#!/usr/bin/env` shim's interpreter) resolves as at the site. It runs
//! with a fresh empty HOME and TMPDIR where the site gives none of its own
//! (a scratch site gives every check fresh ones), in a fresh empty
//! directory: never the live tree, which a `--list` could write to. So it
//! never sees the configuration of the tree a check runs on -- its aliases,
//! the toolchain it pins -- and its answer is used only to withhold a
//! verdict, never to give one (`verdict_subcommand`).
//!
//! A listing is stopped when it prints no new line for the site's stall
//! bound (a no-progress bound: a line printed again is not progress),
//! prints more than [`MOST_OUTPUT`] (a tool that never stops: its output is
//! read from pipes, never written to disk), or writes more than
//! [`MOST_SCRATCH`] to its own directories. Each of those is no answer, and
//! so is a tool that could not run (exit 126 or 127, a shim that found no
//! interpreter) or was killed by a signal. The tool leads its own process
//! group, and the whole group is killed and the tool reaped on every way
//! out; the listing's directory is removed on every way out too, and a
//! directory that could not be removed is reported, and is no answer. A
//! tool is asked once per program (its resolved path, size and change time)
//! and environment, whatever it answered; one that gave no answer is asked
//! again next time.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::super::Context;

/// The longest a listing may print nothing before it is stopped.
pub(crate) const LIST_STALL: Duration = Duration::from_secs(10);
/// The most a listing may print before it is stopped.
const MOST_OUTPUT: u64 = 1 << 20;

/// What `tool --list` told of a tool's built-in commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Listing {
    Commands(BTreeSet<String>),
    /// It answered, but listed none; why, for an operator.
    Unlisted(String),
    /// It gave no answer: it could not start, stalled or never stopped.
    NoAnswer(String),
}

type Identity = (PathBuf, u64, Option<SystemTime>);
type Key = (Identity, BTreeMap<String, String>);
static LISTED: LazyLock<Mutex<BTreeMap<Key, Listing>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// `program`'s built-in commands at `at`'s site (see the module docs).
pub(super) fn listing(program: &Path, at: &Context) -> Listing {
    let lock = || LISTED.lock().unwrap_or_else(|poison| poison.into_inner());
    let Ok(resolved) = std::fs::canonicalize(program) else {
        return Listing::NoAnswer(format!("`{}` could not be resolved", program.display()));
    };
    let meta = std::fs::metadata(&resolved).ok();
    let identity = (
        resolved,
        meta.as_ref().map_or(0, std::fs::Metadata::len),
        meta.and_then(|meta| meta.modified().ok()),
    );
    let key = (identity, at.environment.clone());
    if let Some(known) = lock().get(&key) {
        return known.clone();
    }
    let listing = list(program, at).unwrap_or_else(Listing::NoAnswer);
    if !matches!(listing, Listing::NoAnswer(_)) {
        lock().insert(key, listing.clone());
    }
    listing
}

/// List every one of `programs` at once; each one's listing, by program.
pub(super) fn prefetch(programs: &BTreeSet<PathBuf>, at: &Context) -> BTreeMap<PathBuf, Listing> {
    std::thread::scope(|scope| {
        let listing = |program: &PathBuf| (program.clone(), listing(program, at));
        let running: Vec<_> = (programs.iter())
            .map(|program| scope.spawn(move || listing(program)))
            .collect();
        (running.into_iter())
            .filter_map(|thread| thread.join().ok())
            .collect()
    })
}

/// `program`'s listing; `Err` when it gave no answer. Its directory goes
/// however the listing ends; one that could not be removed is reported.
fn list(program: &Path, at: &Context) -> Result<Listing, String> {
    let root = Scratch::make()?;
    let listed = root.path().and_then(|root| list_in(program, root, at));
    match (listed, root.remove()) {
        (listed, Ok(())) => listed,
        (Ok(_), Err(left)) => Err(left),
        (Err(why), Err(left)) => Err(format!("{why}; {left}")),
    }
}

/// The name every listing directory starts with.
pub(super) const SCRATCH_PREFIX: &str = "archon-command-list-";
/// The most a listing's HOME, TMPDIR and directory may hold.
const MOST_SCRATCH: u64 = 16 << 20;

/// A listing's own directory, removed however the listing ends.
struct Scratch(Option<PathBuf>);

/// The number of the next listing directory.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The names the next `count` listing directories would try first.
#[cfg(test)]
pub(super) fn next_scratch(count: u64) -> Vec<PathBuf> {
    let next = NEXT.load(Ordering::SeqCst);
    (next..next + count)
        .map(|n| std::env::temp_dir().join(format!("{SCRATCH_PREFIX}{}-{n}", std::process::id())))
        .collect()
}

impl Scratch {
    /// A new directory of this listing's own: one that did not exist, so a
    /// directory left behind is never reused, nor removed as this one.
    fn make() -> Result<Self, String> {
        for _ in 0..64 {
            let root = std::env::temp_dir().join(format!(
                "{SCRATCH_PREFIX}{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            match std::fs::create_dir(&root) {
                Ok(()) => return Ok(Self(Some(root))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!("no listing directory {}: {error}", root.display()));
                }
            }
        }
        Err("no listing directory: every name tried was taken".to_string())
    }

    /// The directory, with its fresh `home` and `work` directories.
    fn path(&self) -> Result<&Path, String> {
        let root = self.0.as_deref().expect("removed only by `remove`");
        for dir in [root.join("home"), root.join("work")] {
            std::fs::create_dir(&dir)
                .map_err(|error| format!("no listing directory {}: {error}", dir.display()))?;
        }
        Ok(root)
    }

    /// Remove the directory; why it could not be, when it is still there.
    fn remove(mut self) -> Result<(), String> {
        let root = self.0.take().expect("removed once");
        match std::fs::remove_dir_all(&root) {
            Err(error) if root.exists() => {
                let left = format!(
                    "the listing's directory {} could not be removed: {error}",
                    root.display()
                );
                tracing::warn!("{left}");
                Err(left)
            }
            _ => Ok(()),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let Some(root) = self.0.take() else {
            return;
        };
        if let Err(error) = std::fs::remove_dir_all(&root)
            && root.exists()
        {
            tracing::warn!(
                "the listing's directory {} could not be removed: {error}",
                root.display()
            );
        }
    }
}

/// What a stream printed, and whether more of it is progress: a line
/// unlike the one before it. A line printed again and again is not.
#[derive(Default)]
struct Printed {
    bytes: Vec<u8>,
    line_start: usize,
    last: Option<std::ops::Range<usize>>,
}

impl Printed {
    /// Add `chunk`; whether it completed a line unlike the one before.
    fn push(&mut self, chunk: &[u8]) -> bool {
        let from = self.bytes.len();
        self.bytes.extend_from_slice(chunk);
        let ends: Vec<usize> = (from..self.bytes.len())
            .filter(|&at| self.bytes[at] == b'\n')
            .collect();
        let mut progress = false;
        for end in ends {
            let line = self.line_start..end;
            let again = (self.last.clone())
                .is_some_and(|last| self.bytes[last] == self.bytes[line.clone()]);
            progress |= !again;
            self.last = Some(line);
            self.line_start = end + 1;
        }
        progress
    }
}

/// The bytes the files under `dir` hold.
fn bytes_under(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    (entries.flatten())
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => bytes_under(&entry.path()),
            Ok(_) => entry.metadata().map_or(0, |meta| meta.len()),
            Err(_) => 0,
        })
        .sum()
}

fn list_in(program: &Path, root: &Path, at: &Context) -> Result<Listing, String> {
    let (home, work) = (root.join("home"), root.join("work"));
    let shown = program.display();
    // Only the site's own context (see `Context`), and never a credential
    // whatever that context holds (Issue 282): a listing needs none.
    let mut environment = at.environment.clone();
    environment.retain(|name, _| !credential(name));
    for name in ["HOME", "TMPDIR"] {
        let fresh = home.to_string_lossy().into_owned();
        environment.entry(name.into()).or_insert(fresh);
    }
    let spawn = || {
        let mut command = archon_shell::spawn::command(program);
        command
            .arg("--list")
            .current_dir(&work)
            .env_clear()
            .envs(&environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command.spawn()
    };
    // A program written a moment ago can be briefly unable to start while
    // another thread's child still holds it open.
    let mut failure = String::new();
    let mut child = (0..3)
        .find_map(|attempt| {
            std::thread::sleep(Duration::from_millis(50 * attempt));
            spawn().map_err(|error| failure = error.to_string()).ok()
        })
        .ok_or_else(|| format!("`{shown} --list` could not be started: {failure}"))?;
    // The output is read from pipes, never kept on disk, and at most
    // [`MOST_OUTPUT`] of it: past that the group is killed.
    let (sender, received) = std::sync::mpsc::sync_channel::<(usize, Vec<u8>)>(16);
    let pipes: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        child.stdout.take().map(|pipe| Box::new(pipe) as _),
        child.stderr.take().map(|pipe| Box::new(pipe) as _),
    ];
    for (stream, pipe) in pipes.into_iter().enumerate() {
        let (sender, Some(mut pipe)) = (sender.clone(), pipe) else {
            continue;
        };
        std::thread::spawn(move || {
            let mut chunk = vec![0; 64 << 10];
            while let Ok(read @ 1..) = pipe.read(&mut chunk) {
                if sender.send((stream, chunk[..read].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    drop(sender);
    let mut group = Group(Some(child));
    let (mut output, mut status) = ([Printed::default(), Printed::default()], None);
    let (mut since, mut polls) = (Instant::now(), 0u64);
    loop {
        match received.recv_timeout(Duration::from_millis(20)) {
            Ok((stream, chunk)) => {
                if output[stream].push(&chunk) {
                    since = Instant::now();
                }
                let printed = (output.iter()).map(|o| o.bytes.len() as u64).sum::<u64>();
                if printed > MOST_OUTPUT {
                    return Err(format!(
                        "`{shown} --list` output exceeded {MOST_OUTPUT} bytes, so it was stopped"
                    ));
                }
            }
            // Every writer of the pipes is gone: the listing is complete.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) if status.is_some() => break,
            // It closed its output but runs on.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        polls += 1;
        if polls % 10 == 0 && bytes_under(root) > MOST_SCRATCH {
            return Err(format!(
                "`{shown} --list` wrote more than {MOST_SCRATCH} bytes to its own directories, so it was stopped"
            ));
        }
        if status.is_none()
            && group
                .exited()
                .map_err(|e| format!("`{shown} --list`: {e}"))?
        {
            // What it left behind in its group goes now, so the pipes close.
            status = Some(
                group
                    .end()
                    .ok_or_else(|| format!("`{shown} --list` was not reaped"))?,
            );
            since = Instant::now();
        }
        if since.elapsed() >= at.list_stall {
            if status.is_some() {
                break;
            }
            return Err(format!(
                "`{shown} --list` printed nothing new for {} ms, so it was stopped",
                at.list_stall.as_millis()
            ));
        }
    }
    let status = status.expect("the loop ends only once the tool is reaped");
    let [out, err] = output.map(|printed| String::from_utf8_lossy(&printed.bytes).into_owned());
    let said = (err.lines().map(str::trim)).find(|line| !line.is_empty());
    let said = said.map(|line| format!(": {line}")).unwrap_or_default();
    // It never ran: a shim found no interpreter, or it was not executable;
    // or a signal ended it. Neither is an answer.
    match status.code() {
        Some(code @ (126 | 127)) => {
            return Err(format!(
                "`{shown} --list` could not run (exit {code}){said}"
            ));
        }
        None => {
            return Err(format!(
                "`{shown} --list` was killed{}{said}",
                signal(&status)
            ));
        }
        Some(_) => {}
    }
    let commands: BTreeSet<String> = (out.lines())
        .filter(|line| line.starts_with(char::is_whitespace))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| super::named(word))
        .map(str::to_string)
        .collect();
    if status.success() && !commands.is_empty() {
        return Ok(Listing::Commands(commands));
    }
    // It has no listing only when it says so: it listed nothing and
    // succeeded, or it rejected `--list` itself. Anything else -- commands
    // and then a failure, a toolchain proxy that could not choose a
    // toolchain -- is no answer.
    if status.success() && commands.is_empty() {
        return Ok(Listing::Unlisted(format!(
            "`{shown} --list`, run with the site's environment, listed no commands{said}"
        )));
    }
    let code = status.code().unwrap_or(-1);
    match (out.lines().chain(err.lines())).find(|line| NO_LIST.is_match(line)) {
        Some(line) => Ok(Listing::Unlisted(format!(
            "`{shown} --list`, run with the site's environment, exited {code}: {}",
            line.trim()
        ))),
        None => Err(format!(
            "`{shown} --list` exited {code} without listing its commands{said}"
        )),
    }
}

/// A line that rejects the `--list` option itself: `unknown option:
/// --list`, `unexpected argument '--list' found`.
static NO_LIST: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^(?:.*\b(?:unknown|unrecogni[sz]ed|invalid|unexpected|illegal|unsupported|bad)\s+(?:option|argument|flag|switch|subcommand|command)\b.*--list\b|.*--list\b.*\b(?:unknown|unrecogni[sz]ed|invalid|unexpected|illegal|unsupported|bad)\s+(?:option|argument|flag|switch|subcommand|command)\b|.*\bno such (?:option|flag)\b.*--list\b|.*--list\b.*(?:wasn't|was not) expected)",
    )
    .expect("static pattern")
});

/// Whether `name` is a variable a listing never gets: the engine's own
/// credentials, and any key, token, secret or password a site forwards.
fn credential(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    (archon_tools::bash::ENGINE_CREDENTIAL_VARS.iter())
        .any(|owned| owned.eq_ignore_ascii_case(name))
        || ["_API_KEY", "_TOKEN", "_SECRET", "_PASSWORD"]
            .iter()
            .any(|suffix| upper.ends_with(suffix))
        || ["API_KEY", "TOKEN", "SECRET", "PASSWORD"].contains(&upper.as_str())
}

/// The signal that ended `status`, for an operator.
fn signal(status: &ExitStatus) -> String {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(status) {
        return format!(" by signal {signal}");
    }
    let _ = status;
    String::new()
}

/// A listing tool, the leader of its own process group: the group is
/// killed and the tool reaped however the listing ends.
struct Group(Option<Child>);

impl Group {
    /// Whether the tool has exited; on Unix it is not reaped yet, so no
    /// other process can hold its group id while the group is killed.
    fn exited(&mut self) -> std::io::Result<bool> {
        let Some(child) = self.0.as_mut() else {
            return Ok(true);
        };
        #[cfg(unix)]
        return archon_shell::process_tree::exited(child.id());
        #[cfg(not(unix))]
        return child.try_wait().map(|status| status.is_some());
    }

    /// Kill what is left of the group, then reap the tool.
    fn end(&mut self) -> Option<ExitStatus> {
        let mut child = self.0.take()?;
        #[cfg(unix)]
        if let Ok(group) = libc::pid_t::try_from(child.id()) {
            // SAFETY: plain integers; the unreaped leader keeps the group id
            // ours, and ESRCH (nothing left) is the expected answer.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
        let _ = child.kill();
        child.wait().ok()
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.end();
    }
}
