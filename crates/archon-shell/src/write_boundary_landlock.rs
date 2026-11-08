//! Write boundaries applied to a child between fork and exec.
//!
//! # Complement mapping
//!
//! A deny profile permits ordinary writes, seals protected roots and reopens
//! declared writable entries. An allow-only ruleset represents that boundary
//! with its complement: [`plan`] grants siblings beside each ancestor of a
//! protected root, excluding roots, ancestors and symbolic links. Explicit
//! writable entries are granted only when they contain no protected root.
//! File roots and directory roots use the same complement planner.
//!
//! Reads and execution are untouched by the write ruleset. Resolution follows
//! the real hierarchy, including symbolic links, parent traversal and renames.
//!
//! # Stricter ancestor handling
//!
//! New entries directly in an ancestor cannot be granted without opening its
//! protected descendants. Existing siblings remain writable. Explicit file
//! exceptions permit in-place writes, but not creation or atomic replacement.
//! A temporary root containing a protected directory receives a private
//! temporary child so temporary creation remains possible.
//!
//! # Private mount seals
//!
//! [`LandlockSandbox::build_sealed`] uses private recursive binds for existing
//! protected paths and rename guards for their ancestors. It re-enters the
//! working directory after mounting. Without a usable ruleset, other processes
//! can expose unsealed mounts; failed preparation retains the uncontained fallback.
//!
//! # What it does not cover
//!
//! Complement rules do not mediate metadata changes or alternate mounts of a
//! sealed tree reached through a grant. Mount seals do not protect absent
//! startup files or replace process isolation. No-new-privileges prevents
//! privilege gain across execution. ABI 3 is required for rulesets because
//! earlier interfaces do not mediate truncation. Read rulesets are separate.

use std::path::Component;
use std::path::{Path, PathBuf};

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
/// Require mediated truncation before installing a write boundary.
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
/// contain one.
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
    ruleset: Option<std::sync::Arc<std::os::fd::OwnedFd>>,
    #[cfg(target_os = "linux")]
    mounts: Option<std::sync::Arc<mounts::MountSeals>>,
    private_temp: Option<PrivateTemp>,
    grants: usize,
}

impl LandlockSandbox {
    /// Preserve writes outside sealed directories and reopen the supplied exceptions.
    /// File and directory roots both use complement grants.
    pub fn build(
        protected: &[PathBuf],
        writable: &[PathBuf],
        temp_dirs: &[PathBuf],
    ) -> Result<Self, String> {
        Self::build_inner(protected, writable, temp_dirs)
    }

    fn build_inner(
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
        let sealed: Vec<_> = protected
            .iter()
            .filter(|path| !path.is_file())
            .flat_map(|root| spellings(root))
            .collect();
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
            ruleset: Some(std::sync::Arc::new(sys::ruleset(&grants)?)),
            #[cfg(target_os = "linux")]
            mounts: None,
            private_temp,
            grants: grants.len(),
        })
    }

    /// Build a second write layer. Mount operations cannot follow an earlier restriction.
    pub fn build_layered(
        protected: &[PathBuf],
        writable: &[PathBuf],
        temp_dirs: &[PathBuf],
    ) -> Result<Self, String> {
        Self::build_inner(protected, writable, temp_dirs)
    }

    /// Probe namespace setup before preparing persistent paths.
    #[cfg(target_os = "linux")]
    pub fn probe_seals() -> Result<(), String> {
        mounts::MountSeals::probe()
    }

    /// Seal existing paths with private mounts while allowing ordinary writes elsewhere.
    #[cfg(target_os = "linux")]
    pub fn build_sealed(protected: &[PathBuf]) -> Result<Self, String> {
        let grants = vec![PathBuf::from(std::path::MAIN_SEPARATOR_STR)];
        Ok(Self {
            ruleset: None,
            mounts: Some(std::sync::Arc::new(mounts::MountSeals::build(
                protected, &grants,
            )?)),
            private_temp: None,
            grants: 0,
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
                ruleset: Some(std::sync::Arc::new(sys::ruleset_rights(
                    &grants,
                    // REFER is denied even when omitted from handled rights.
                    // Preserve renames/hard links within readable trees; the
                    // separate write ruleset still judges every write.
                    READ_FILE | READ_DIR | REFER,
                    READ_FILE,
                )?)),
                mounts: None,
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

    /// A private temporary directory when a supplied temporary root contains a seal.
    pub fn private_temp(&self) -> Option<&Path> {
        self.private_temp.as_ref().map(|temp| temp.0.as_path())
    }

    /// How many path rules the ruleset holds.
    pub fn grants(&self) -> usize {
        self.grants
    }

    /// Install mount seals before restricting the child. The hook uses raw
    /// syscalls and prepared buffers, with no allocation or host-state locks.
    #[cfg(target_os = "linux")]
    pub fn restrict_hook(&self) -> impl FnMut() -> std::io::Result<()> + Send + Sync + 'static {
        let ruleset = self.ruleset.clone();
        let status = self
            .mounts
            .as_ref()
            .map(|mounts| SealStatus::new(mounts.clone()));
        move || {
            if let Some(status) = &status {
                status.apply();
            }
            match &ruleset {
                Some(ruleset) => sys::restrict(ruleset),
                None => Ok(()),
            }
        }
    }

    /// Apply the prepared restriction and retain a parent-side outcome reader.
    #[cfg(target_os = "linux")]
    pub fn install_std(&self, command: &mut std::process::Command) {
        let _ = self.install_with_report(command);
    }

    /// Retain optional operation outcomes for queries made after spawn.
    #[cfg(target_os = "linux")]
    pub fn install_with_report(&self, command: &mut std::process::Command) -> Option<SealStatus> {
        let status = self
            .mounts
            .as_ref()
            .map(|mounts| SealStatus::new(mounts.clone()));
        let ruleset = self.ruleset.clone();
        let child_status = status.clone();
        // SAFETY: only prepared buffers and raw calls are used in the child.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(command, move || {
                if let Some(status) = &child_status {
                    status.apply();
                }
                match &ruleset {
                    Some(ruleset) => sys::restrict(ruleset),
                    None => Ok(()),
                }
            });
        }
        status
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
    let ruleset = sys::ruleset(&[])?;
    sys::probe_restriction(&ruleset)?;
    Ok(abi)
}

#[cfg(target_os = "linux")]
#[path = "write_boundary_landlock_sys.rs"]
mod sys;

#[cfg(test)]
#[path = "write_boundary_landlock_tests.rs"]
mod tests;

#[cfg(target_os = "linux")]
#[path = "write_boundary_mounts.rs"]
mod mounts;

#[cfg(target_os = "linux")]
#[path = "write_boundary_seal_status.rs"]
mod seal_status;
#[cfg(target_os = "linux")]
pub use seal_status::SealStatus;

#[cfg(all(test, target_os = "linux"))]
#[path = "write_boundary_seal_kernel_tests.rs"]
mod seal_kernel_tests;
