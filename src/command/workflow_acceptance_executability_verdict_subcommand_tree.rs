//! The directory a tool lists its commands in: the configuration of the
//! tree a check runs on (Issue 333).
//!
//! A tool reads configuration from the directory it runs in: cargo reads
//! `.cargo/config.toml` (its aliases) and a rustup proxy reads
//! `rust-toolchain.toml` (its toolchain). A check runs in a fresh clone of
//! the recorded commit, gone by the time its result is read, and the host
//! never runs a command in the live working tree, which a tool's `--list`
//! could write to. So the listing runs in a fresh directory holding what
//! the commit has at its root: every root file, and everything under each
//! root directory whose name starts with a dot -- where tools keep their
//! configuration. Only committed content is used, as the site's clone has
//! only that. A file larger than [`LARGEST`] is left out: configuration is
//! small.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The largest file copied.
const LARGEST: u64 = 1 << 20;

/// A tree a check runs on: a repository, at a commit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SiteTree {
    pub(crate) repository: PathBuf,
    pub(crate) commit: String,
}

/// Write `tree`'s root configuration (see the module docs) into `dir`.
pub(super) fn materialize(tree: &SiteTree, dir: &Path) -> Result<(), String> {
    let git = || {
        let mut command = Command::new("git");
        command.arg("-C").arg(&tree.repository);
        command
    };
    let listed = git()
        .args(["ls-tree", "-r", "-z", "-l", "--full-tree", &tree.commit])
        .output()
        .map_err(|error| format!("git ls-tree could not run: {error}"))?;
    if !listed.status.success() {
        return Err(format!(
            "git ls-tree {} failed: {}",
            tree.commit,
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    let wanted: Vec<(String, String)> = (listed.stdout.split(|b| *b == 0))
        .filter_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (meta, path) = entry.split_once('\t')?;
            let mut meta = meta.split_whitespace();
            let (mode, kind, object, size) =
                (meta.next()?, meta.next()?, meta.next()?, meta.next()?);
            let root = !path.contains('/') || path.starts_with('.');
            let small = size.parse::<u64>().is_ok_and(|size| size <= LARGEST);
            let plain = kind == "blob" && matches!(mode, "100644" | "100755");
            let safe = !path.split('/').any(|part| part == ".." || part.is_empty());
            (root && small && plain && safe).then(|| (object.to_string(), path.to_string()))
        })
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut cat = git()
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("git cat-file could not run: {error}"))?;
    let mut input = cat.stdin.take().expect("piped stdin");
    let objects: String = wanted
        .iter()
        .map(|(object, _)| format!("{object}\n"))
        .collect();
    let writer = std::thread::spawn(move || input.write_all(objects.as_bytes()));
    let mut output = BufReader::new(cat.stdout.take().expect("piped stdout"));
    let copied = (wanted.iter()).try_for_each(|(_, path)| {
        let mut header = String::new();
        output.read_line(&mut header).map_err(|e| e.to_string())?;
        let size: usize = (header.split_whitespace().nth(2))
            .and_then(|size| size.parse().ok())
            .ok_or_else(|| format!("git cat-file answered `{}`", header.trim()))?;
        let mut content = vec![0; size + 1];
        output.read_exact(&mut content).map_err(|e| e.to_string())?;
        content.pop();
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, content).map_err(|e| e.to_string())
    });
    let _ = writer.join();
    let _ = cat.kill();
    let _ = cat.wait();
    copied.map_err(|error| format!("copying {}'s root configuration: {error}", tree.commit))
}
