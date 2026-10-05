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
//! (a no-progress bound) or prints more than [`MOST_OUTPUT`] (a tool that
//! never stops). The tool leads its own process group, and the whole group
//! is killed and the tool reaped on every way out. A tool is asked once per
//! program (its resolved path, size and change time), environment and tree,
//! whatever it answered; one that gave no answer is asked again next time.

use std::collections::{BTreeMap, BTreeSet};
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

/// `program`'s listing; `Err` when it gave no answer.
fn list(program: &Path, at: &Context) -> Result<Listing, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "archon-command-list-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let listed = (|| {
        let (home, tree) = (root.join("home"), root.join("tree"));
        for dir in [&home, &tree] {
            std::fs::create_dir_all(dir)
                .map_err(|error| format!("no listing directory: {error}"))?;
        }
        if let Some(site) = &at.tree {
            super::tree::materialize(site, &tree)?;
        }
        list_in(program, &home, &tree, at)
    })();
    let _ = std::fs::remove_dir_all(&root);
    listed
}

fn list_in(program: &Path, home: &Path, tree: &Path, at: &Context) -> Result<Listing, String> {
    let (out, err) = (home.join(".list"), home.join(".list-errors"));
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
            .stdout(std::fs::File::create(&out)?)
            .stderr(std::fs::File::create(&err)?);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command.spawn()
    };
    // A program written a moment ago can be briefly unable to start while
    // another thread's child still holds it open.
    let mut failure = String::new();
    let child = (0..3)
        .find_map(|attempt| {
            std::thread::sleep(Duration::from_millis(50 * attempt));
            spawn().map_err(|error| failure = error.to_string()).ok()
        })
        .ok_or_else(|| format!("`{shown} --list` could not be started: {failure}"))?;
    let mut group = Group(Some(child));
    let printed = || [&out, &err].map(|file| std::fs::metadata(file).map_or(0, |m| m.len()));
    let (mut seen, mut since) = (printed(), Instant::now());
    loop {
        if group
            .exited()
            .map_err(|error| format!("`{shown} --list`: {error}"))?
        {
            break;
        }
        let now = printed();
        if now.iter().sum::<u64>() > MOST_OUTPUT {
            return Err(format!(
                "`{shown} --list` printed more than {MOST_OUTPUT} bytes, so it was stopped"
            ));
        }
        if now != seen {
            (seen, since) = (now, Instant::now());
        } else if since.elapsed() >= at.list_stall {
            return Err(format!(
                "`{shown} --list` printed nothing for {} ms, so it was stopped",
                at.list_stall.as_millis()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = group
        .end()
        .ok_or_else(|| format!("`{shown} --list` could not be reaped"))?;
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let commands: BTreeSet<String> = (text.lines())
        .filter(|line| line.starts_with(char::is_whitespace))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| super::named(word))
        .map(str::to_string)
        .collect();
    if status.success() && !commands.is_empty() {
        return Ok(Listing::Commands(commands));
    }
    let errors = std::fs::read_to_string(&err).unwrap_or_default();
    let said = (errors.lines().map(str::trim)).find(|line| !line.is_empty());
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
