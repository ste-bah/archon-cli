//! Issue-227: the Linux write boundary, a Landlock ruleset applied to the
//! child between `fork` and `exec`.
//!
//! # Mapping the macOS profile onto Landlock
//!
//! The `sandbox-exec` profile allows every write, denies writes under the
//! sealed roots and re-opens the writable entries inside them. Landlock
//! cannot deny: a ruleset handles a set of access rights and refuses each of
//! them everywhere except beneath a path a rule allows. So the same boundary
//! is drawn as its complement ([`plan`]): for every ancestor of a sealed
//! root, a rule on each entry beside the path down to the root (never the
//! root, never another ancestor, never a symlink), plus a rule on each
//! writable entry that does not contain a sealed root. Writes everywhere a
//! toolchain writes stay allowed; under a sealed root only the re-opened
//! entries are.
//!
//! Only the write rights are handled (`WRITE_FILE`, `TRUNCATE`, the `MAKE_*`
//! and `REMOVE_*` rights, `REFER`), so reading and executing are untouched.
//! Resolution is the kernel's: a symlink, a hard link, `..` and a rename are
//! all judged on the real hierarchy.
//!
//! # Where it is stricter than the macOS profile
//!
//! Landlock grants a directory's rights to everything beneath it, so it
//! cannot let a command create an entry DIRECTLY in an ancestor of a sealed
//! root without also opening the root. On Linux the command therefore cannot
//! create, remove or rename entries directly in such an ancestor (existing
//! entries beside the root stay writable), and a declared artifact file
//! inside a sealed root is writable in place only: it cannot be created, nor
//! replaced through a sibling, which the profile's `regex` rule allows. A
//! host temp directory that contains a sealed root (a project under `/tmp`)
//! is replaced, for the command, by a private directory inside it
//! ([`LandlockSandbox::private_temp`]), so `mktemp` keeps working.
//!
//! # What it does not cover
//!
//! Metadata changes Landlock does not mediate (`chmod`, `utimes`, extended
//! attributes) and a second mount of a sealed tree reached through a granted
//! path. `PR_SET_NO_NEW_PRIVS` is set on the child, which Landlock requires:
//! a set-uid program does not gain privileges inside a bounded command.
//!
//! ABI 3 (Linux 6.2) is the floor: before it `truncate(2)` is not mediated
//! and a sealed file could be emptied. The probe refuses an older kernel
//! rather than applying a partial ruleset (no best-effort compatibility).

use std::path::{Component, Path, PathBuf};

/// Every access right this boundary handles.
pub const HANDLED_RIGHTS: u64 = WRITE_FILE
    | REMOVE_DIR
    | REMOVE_FILE
    | MAKE_CHAR
    | MAKE_DIR
    | MAKE_REG
    | MAKE_SOCK
    | MAKE_FIFO
    | MAKE_BLOCK
    | MAKE_SYM
    | REFER
    | TRUNCATE;
/// The rights a rule on a non-directory may carry.
pub const FILE_RIGHTS: u64 = WRITE_FILE | TRUNCATE;
/// The ABI floor; see the module docs.
pub const MIN_ABI: u32 = 3;

const WRITE_FILE: u64 = 1 << 1;
const REMOVE_DIR: u64 = 1 << 4;
const REMOVE_FILE: u64 = 1 << 5;
const MAKE_CHAR: u64 = 1 << 6;
const MAKE_DIR: u64 = 1 << 7;
const MAKE_REG: u64 = 1 << 8;
const MAKE_SOCK: u64 = 1 << 9;
const MAKE_FIFO: u64 = 1 << 10;
const MAKE_BLOCK: u64 = 1 << 11;
const MAKE_SYM: u64 = 1 << 12;
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;

/// The paths a ruleset sealing `protected` allows writes beneath: the
/// complement of the sealed roots plus the `writable` entries that do not
/// contain one. See the module docs.
pub fn plan(protected: &[PathBuf], writable: &[PathBuf]) -> Vec<PathBuf> {
    let mut sealed = Vec::new();
    for root in protected.iter().filter(|root| root.is_absolute()) {
        for spelling in spellings(root) {
            push_unique(&mut sealed, spelling);
        }
    }
    let inside = |path: &Path| sealed.iter().any(|root| path.starts_with(root));
    let holds = |path: &Path| sealed.iter().any(|root| root.starts_with(path));
    // The ancestors of each root's REAL path only: every prefix of a real
    // path is real, so they cover the hierarchy the kernel judges, and an
    // entry listed in one (not a symlink) is itself a real path. An ancestor
    // under another spelling adds nothing a rule would need.
    let mut ancestors = Vec::new();
    for root in protected.iter().filter(|root| root.is_absolute()) {
        if let Some(real) = spellings(root).pop() {
            for ancestor in real.ancestors().skip(1) {
                push_unique(&mut ancestors, ancestor.to_path_buf());
            }
        }
    }
    let mut grants = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in ancestors.iter().filter(|dir| !inside(dir)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().map_or(true, |kind| kind.is_symlink()) {
                continue;
            }
            let path = entry.path();
            if !inside(&path) && !holds(&path) && seen.insert(path.clone()) {
                grants.push(path);
            }
        }
    }
    for entry in writable.iter().filter(|entry| entry.is_absolute()) {
        let named = spellings(entry);
        if named.iter().any(|spelling| holds(spelling)) {
            continue;
        }
        if let Some(real) = named.last().filter(|real| real.exists())
            && seen.insert(real.clone())
        {
            grants.push(real.clone());
        }
    }
    grants
}

/// `path` with `.` and `..` folded, and that with its longest existing
/// prefix resolved: the kernel judges the real path.
fn spellings(path: &Path) -> Vec<PathBuf> {
    let mut given = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                given.pop();
            }
            Component::CurDir => {}
            other => given.push(other),
        }
    }
    let mut out = vec![given.clone()];
    let mut existing = given.as_path();
    let mut rest = Vec::new();
    while existing.symlink_metadata().is_err() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return out;
        };
        rest.push(name.to_os_string());
        existing = parent;
    }
    if let Ok(mut real) = existing.canonicalize() {
        real.extend(rest.iter().rev());
        push_unique(&mut out, real);
    }
    out
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.contains(&candidate) {
        paths.push(candidate);
    }
}

/// A private temp directory for one bounded command; removed on drop.
#[derive(Debug)]
struct PrivateTemp(PathBuf);

impl PrivateTemp {
    fn create_in(parent: &Path) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        #[cfg(unix)]
        let builder = {
            let mut builder = std::fs::DirBuilder::new();
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder
        };
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        let mut last = None;
        for _ in 0..8 {
            let name = format!(
                "archon-bounded-tmp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let dir = parent.join(name);
            match builder.create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(error) => last = Some(error),
            }
        }
        Err(format!(
            "a private temp directory could not be created in {}: {}",
            parent.display(),
            last.map(|e| e.to_string()).unwrap_or_default()
        ))
    }
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A Landlock ruleset built in the host, ready to be applied to a child.
#[derive(Debug)]
pub struct LandlockSandbox {
    #[cfg(target_os = "linux")]
    ruleset: std::sync::Arc<std::os::fd::OwnedFd>,
    private_temp: Option<PrivateTemp>,
    grants: usize,
}

impl LandlockSandbox {
    /// Build the ruleset sealing `protected` and re-opening `writable`.
    /// `temp_dirs` are the temp directories the command will be pointed at;
    /// when one contains a sealed root, a private directory is created in it
    /// and granted (see [`Self::private_temp`]). `Err` when any rule cannot
    /// be added: the caller then runs nothing.
    pub fn build(
        protected: &[PathBuf],
        writable: &[PathBuf],
        temp_dirs: &[PathBuf],
    ) -> Result<Self, String> {
        if !cfg!(target_os = "linux") {
            return Err(format!(
                "Landlock is Linux-only, not {}",
                std::env::consts::OS
            ));
        }
        let sealed: Vec<PathBuf> = protected.iter().flat_map(|root| spellings(root)).collect();
        let crowded = temp_dirs.iter().find(|dir| {
            dir.is_absolute()
                && spellings(dir)
                    .iter()
                    .any(|dir| sealed.iter().any(|root| root.starts_with(dir)))
        });
        let private_temp = crowded.map(|dir| PrivateTemp::create_in(dir)).transpose()?;
        let mut writable = writable.to_vec();
        if let Some(temp) = &private_temp {
            writable.push(temp.0.clone());
        }
        let grants = plan(protected, &writable);
        Ok(Self {
            #[cfg(target_os = "linux")]
            ruleset: std::sync::Arc::new(sys::ruleset(&grants)?),
            private_temp,
            grants: grants.len(),
        })
    }

    /// A read ruleset, separate from the existing write ruleset. Landlock
    /// combines restrictions when both are installed on the same child.
    pub fn build_read(denied: &[PathBuf]) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        {
            const READ_FILE: u64 = 1 << 2;
            const READ_DIR: u64 = 1 << 3;
            let grants = plan(denied, &[]);
            return Ok(Self {
                ruleset: std::sync::Arc::new(sys::ruleset_rights(
                    &grants,
                    // REFER is denied even when omitted from handled rights.
                    // Preserve renames/hard links within readable trees; the
                    // separate write ruleset still judges every write.
                    READ_FILE | READ_DIR | REFER,
                    READ_FILE,
                )?),
                private_temp: None,
                grants: grants.len(),
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = denied;
            Err("Landlock read protection requires Linux".into())
        }
    }

    /// The directory to point `TMPDIR`, `TMP` and `TEMP` at, when the host's
    /// own temp directory contains a sealed root.
    pub fn private_temp(&self) -> Option<&Path> {
        self.private_temp.as_ref().map(|temp| temp.0.as_path())
    }

    /// How many path rules the ruleset holds.
    pub fn grants(&self) -> usize {
        self.grants
    }

    /// The hook that restricts the child, for a `pre_exec` (std's or
    /// tokio's): it makes two raw syscalls and allocates nothing, so it is
    /// safe between `fork` and `exec` of a multi-threaded host.
    #[cfg(target_os = "linux")]
    pub fn restrict_hook(&self) -> impl FnMut() -> std::io::Result<()> + Send + Sync + 'static {
        let ruleset = std::sync::Arc::clone(&self.ruleset);
        move || sys::restrict(&ruleset)
    }

    /// Apply the ruleset to the child `command` starts, before it execs.
    #[cfg(target_os = "linux")]
    pub fn install_std(&self, command: &mut std::process::Command) {
        // SAFETY: see `restrict_hook`.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(command, self.restrict_hook());
        }
    }
}

/// The running kernel's Landlock ABI, when it is at least [`MIN_ABI`] and a
/// child can actually be restricted from this process.
#[cfg(target_os = "linux")]
pub fn probe() -> Result<u32, String> {
    let abi = sys::abi()?;
    if abi < MIN_ABI {
        return Err(format!(
            "the kernel's Landlock ABI is {abi}; {MIN_ABI} (Linux 6.2) is required, since \
             truncate(2) is not restricted before it"
        ));
    }
    let sandbox = LandlockSandbox::build(&[], &[], &[])?;
    let mut trial = crate::spawn::command("/bin/sh");
    trial
        .args(["-c", ":"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    sandbox.install_std(&mut trial);
    match trial.status() {
        Ok(status) if status.success() => Ok(abi),
        Ok(status) => Err(format!(
            "a Landlock-restricted trial child failed: {status}"
        )),
        Err(error) => Err(format!(
            "a Landlock ruleset cannot be applied to a child of this process: {error}"
        )),
    }
}

#[cfg(target_os = "linux")]
#[path = "write_boundary_landlock_sys.rs"]
mod sys;

#[cfg(test)]
#[path = "write_boundary_landlock_tests.rs"]
mod tests;
