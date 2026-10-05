//! The commands a tool builds in, as a check's site sees it (Issues 331,
//! 333).
//!
//! `tool --list` runs with the site's own environment -- the variables its
//! policy binds, such as where a toolchain proxy finds its toolchains --
//! but with an empty search path, so no `tool-*` program on a path is
//! listed, and with a fresh empty HOME and TMPDIR where the site gives none
//! of its own (a scratch site gives every check fresh ones). So a proxy
//! resolves the real toolchain binary as the site would, and never through
//! the operator's home unless the site itself runs there. A tool that does
//! not list there is never judged, and why is kept for the operator.
//!
//! A listing is given up only when it prints nothing for the site's stall
//! bound (a no-progress bound, never a total). Each tool is asked once per
//! process and environment, whatever it answered; only a tool that could
//! not be started is asked again. Many tools are asked at once
//! ([`prefetch`]), so a slow one costs its own wait once, not once per
//! command.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::super::Context;

/// The longest a listing may print nothing before it is stopped.
pub(crate) const LIST_STALL: Duration = Duration::from_secs(10);

/// What `tool --list` told of a tool's built-in commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Listing {
    Commands(BTreeSet<String>),
    /// It listed none; why, for an operator.
    Unknown(String),
}

type Key = (PathBuf, BTreeMap<String, String>);
static LISTED: LazyLock<Mutex<BTreeMap<Key, Listing>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// `program`'s built-in commands at `at`'s site (see the module docs).
pub(super) fn listing(program: &Path, at: &Context) -> Listing {
    let lock = || LISTED.lock().unwrap_or_else(|poison| poison.into_inner());
    let key = (program.to_path_buf(), at.environment.clone());
    if let Some(known) = lock().get(&key) {
        return known.clone();
    }
    match list(program, at) {
        Ok(listing) => {
            lock().insert(key, listing.clone());
            listing
        }
        Err(why) => Listing::Unknown(why),
    }
}

/// List every one of `programs` at once, so each later [`listing`] is known.
pub(super) fn prefetch(programs: &BTreeSet<PathBuf>, at: &Context) {
    std::thread::scope(|scope| {
        for program in programs {
            scope.spawn(move || listing(program, at));
        }
    });
}

/// `program`'s listing; `Err` when it could not be started (not kept).
fn list(program: &Path, at: &Context) -> Result<Listing, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let home = std::env::temp_dir().join(format!(
        "archon-command-list-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&home).map_err(|error| format!("no listing directory: {error}"))?;
    let listed = list_in(program, &home, at);
    let _ = std::fs::remove_dir_all(&home);
    listed
}

fn list_in(program: &Path, home: &Path, at: &Context) -> Result<Listing, String> {
    let (out, err) = (home.join(".list"), home.join(".list-errors"));
    let shown = program.display();
    let mut environment = at.environment.clone();
    environment.insert("PATH".into(), String::new());
    for name in ["HOME", "TMPDIR"] {
        let fresh = home.to_string_lossy().into_owned();
        environment.entry(name.into()).or_insert(fresh);
    }
    let spawn = || {
        Command::new(program)
            .arg("--list")
            .current_dir(home)
            .env_clear()
            .envs(&environment)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out)?)
            .stderr(std::fs::File::create(&err)?)
            .spawn()
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
    let printed = || [&out, &err].map(|file| std::fs::metadata(file).map_or(0, |m| m.len()));
    let (mut seen, mut since) = (printed(), Instant::now());
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        let now = printed();
        if now != seen {
            (seen, since) = (now, Instant::now());
        } else if since.elapsed() >= at.list_stall {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Listing::Unknown(format!(
                "`{shown} --list` printed nothing for {} ms, so it was stopped",
                at.list_stall.as_millis()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
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
    Ok(Listing::Unknown(format!(
        "`{shown} --list`, run with the site's environment and an empty search path, {}{}",
        match status.code() {
            Some(0) => "listed no commands".to_string(),
            Some(code) => format!("exited {code}"),
            None => "was killed".to_string(),
        },
        said.map(|line| format!(": {line}")).unwrap_or_default()
    )))
}
