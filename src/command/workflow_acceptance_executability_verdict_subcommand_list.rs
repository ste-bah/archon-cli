//! The commands a tool builds in, as a check's site sees it (Issues 331,
//! 333).
//!
//! `tool --list` runs with the site's own environment -- the variables its
//! policy binds, such as where a toolchain proxy finds its toolchains --
//! but with an empty search path, so no `tool-*` program on a path is
//! listed, and with a fresh empty HOME and TMPDIR where the site gives none
//! of its own (a scratch site gives every check fresh ones). It runs in a
//! fresh directory holding the configuration of the tree the check runs on
//! (`verdict_subcommand_tree`), so a proxy resolves the toolchain that tree
//! pins and the tree's own aliases are listed, as at the site.
//!
//! A listing is stopped when it prints nothing for the site's stall bound
//! (a no-progress bound), prints more than [`MOST_OUTPUT`] (a tool that
//! never stops: its output is read from pipes, never written to disk), or
//! writes more than [`MOST_SCRATCH`] to its own directories. Each of those
//! is no answer. The tool leads its own process group, and the whole group
//! is killed and the tool reaped on every way out; the listing's directory
//! is removed on every way out too. A tool is asked once per
//! program (its resolved path, size and change time), environment and tree,
//! whatever it answered; one that gave no answer is asked again next time.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
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
type Key = (Identity, BTreeMap<String, String>, Option<super::SiteTree>);
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
    let key = (identity, at.environment.clone(), at.tree.clone());
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
/// however the listing ends.
fn list(program: &Path, at: &Context) -> Result<Listing, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = Scratch(std::env::temp_dir().join(format!(
        "{SCRATCH_PREFIX}{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )));
    let (home, tree) = (root.0.join("home"), root.0.join("tree"));
    for dir in [&home, &tree] {
        std::fs::create_dir_all(dir).map_err(|error| format!("no listing directory: {error}"))?;
    }
    if let Some(site) = &at.tree {
        super::tree::materialize(site, &tree)?;
    }
    list_in(program, &root.0, &home, &tree, at)
}

/// The name every listing directory starts with.
pub(super) const SCRATCH_PREFIX: &str = "archon-command-list-";
/// The most a listing's HOME, TMPDIR and directory may hold.
const MOST_SCRATCH: u64 = 16 << 20;

/// A listing's own directory, removed when it is dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

fn list_in(
    program: &Path,
    root: &Path,
    home: &Path,
    tree: &Path,
    at: &Context,
) -> Result<Listing, String> {
    let shown = program.display();
    let mut environment = at.environment.clone();
    environment.insert("PATH".into(), String::new());
    for name in ["HOME", "TMPDIR"] {
        let fresh = home.to_string_lossy().into_owned();
        environment.entry(name.into()).or_insert(fresh);
    }
    let spawn = || {
        let mut command = Command::new(program);
        command
            .arg("--list")
            .current_dir(tree)
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
    let (mut output, mut status) = ([Vec::new(), Vec::new()], None);
    let (mut since, mut polls) = (Instant::now(), 0u64);
    loop {
        match received.recv_timeout(Duration::from_millis(20)) {
            Ok((stream, chunk)) => {
                output[stream].extend_from_slice(&chunk);
                since = Instant::now();
                let printed = output.iter().map(|o| o.len() as u64).sum::<u64>();
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
                "`{shown} --list` printed nothing for {} ms, so it was stopped",
                at.list_stall.as_millis()
            ));
        }
    }
    let status = status.expect("the loop ends only once the tool is reaped");
    let [out, err] = output.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let commands: BTreeSet<String> = (out.lines())
        .filter(|line| line.starts_with(char::is_whitespace))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| super::named(word))
        .map(str::to_string)
        .collect();
    if status.success() && !commands.is_empty() {
        return Ok(Listing::Commands(commands));
    }
    let said = (err.lines().map(str::trim)).find(|line| !line.is_empty());
    Ok(Listing::Unlisted(format!(
        "`{shown} --list`, run with the site's environment and an empty search path, {}{}",
        match status.code() {
            Some(0) => "listed no commands".to_string(),
            Some(code) => format!("exited {code}"),
            None => "was killed".to_string(),
        },
        said.map(|line| format!(": {line}")).unwrap_or_default()
    )))
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
