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
    let mut trial = std::process::Command::new("/bin/sh");
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
mod sys {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    const CREATE_RULESET_VERSION: libc::c_uint = 1;
    const RULE_PATH_BENEATH: libc::c_int = 1;

    /// `struct landlock_ruleset_attr`, first field only: the kernel accepts
    /// a shorter struct and treats the rest as zero.
    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    /// `struct landlock_path_beneath_attr` (packed in the uapi header).
    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    pub(super) fn abi() -> Result<u32, String> {
        // SAFETY: the documented version query: no attribute, size 0.
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                CREATE_RULESET_VERSION,
            )
        };
        if abi >= 0 {
            return Ok(abi as u32);
        }
        let error = io::Error::last_os_error();
        Err(match error.raw_os_error() {
            Some(libc::ENOSYS) => "the kernel has no Landlock (Linux 5.13+ built with \
                                   CONFIG_SECURITY_LANDLOCK)"
                .to_string(),
            Some(libc::EOPNOTSUPP) => "Landlock is built in but disabled at boot (it is \
                                       missing from the kernel's lsm= list)"
                .to_string(),
            _ => format!("the Landlock ABI cannot be queried: {error}"),
        })
    }

    /// A ruleset handling the write rights, allowing them beneath `grants`.
    pub(super) fn ruleset(grants: &[PathBuf]) -> Result<OwnedFd, String> {
        let attr = RulesetAttr {
            handled_access_fs: super::HANDLED_RIGHTS,
        };
        // SAFETY: a valid attribute of the size passed.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(format!(
                "a Landlock ruleset cannot be created: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: a fresh descriptor the kernel just returned (close-on-exec).
        let ruleset = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        for path in grants {
            allow(&ruleset, path)?;
        }
        Ok(ruleset)
    }

    fn allow(ruleset: &OwnedFd, path: &std::path::Path) -> Result<(), String> {
        let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return Ok(());
        };
        // `O_NOFOLLOW`: every planned path is real, so a final link here was
        // swapped in after planning and is never followed into a rule.
        let flags = libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: a NUL-terminated path; the result is checked.
        let raw = unsafe { libc::open(name.as_ptr(), flags) };
        if raw < 0 {
            // Gone since it was listed, or not ours to open: not granted,
            // which only makes the boundary stricter.
            return Ok(());
        }
        // SAFETY: a fresh descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: `stat` is plain data, filled in by `fstat`.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: a valid descriptor and buffer.
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0 {
            return Ok(());
        }
        let kind = stat.st_mode & libc::S_IFMT;
        if kind == libc::S_IFLNK {
            return Ok(());
        }
        let directory = kind == libc::S_IFDIR;
        let rule = PathBeneathAttr {
            allowed_access: if directory {
                super::HANDLED_RIGHTS
            } else {
                super::FILE_RIGHTS
            },
            parent_fd: fd.as_raw_fd(),
        };
        // SAFETY: a valid ruleset descriptor and rule.
        let added = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset.as_raw_fd(),
                RULE_PATH_BENEATH,
                &rule as *const PathBeneathAttr,
                0 as libc::c_uint,
            )
        };
        if added != 0 {
            return Err(format!(
                "a Landlock rule for {} cannot be added: {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    /// In the child, between `fork` and `exec`: two raw syscalls, no
    /// allocation, no lock.
    pub(super) fn restrict(ruleset: &OwnedFd) -> io::Result<()> {
        // SAFETY: plain syscalls on this (single-threaded) child.
        unsafe {
            let (one, zero): (libc::c_ulong, libc::c_ulong) = (1, 0);
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, one, zero, zero, zero) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::syscall(
                libc::SYS_landlock_restrict_self,
                ruleset.as_raw_fd(),
                0 as libc::c_uint,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "write_boundary_landlock_tests.rs"]
mod tests;
