//! Prepared private mount seals. Everything owned by the hook is built before fork.
use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

#[derive(Debug)]
struct Seal {
    path: CString,
    display: PathBuf,
    readonly: bool,
    directory: bool,
    submounts: bool,
}

#[derive(Debug)]
pub(super) struct MountSeals {
    seals: Vec<Seal>,
    uid: Vec<u8>,
    gid: Vec<u8>,
}

#[path = "write_boundary_mounts_failure.rs"]
mod diagnostics;
use diagnostics::Failure;
#[cfg(test)]
use diagnostics::namespace_step;

impl MountSeals {
    pub(super) fn build(protected: &[PathBuf], writable: &[PathBuf]) -> Result<Self, String> {
        let mut seals = Vec::new();
        let mountpoints = topology::mountpoints()?;
        for path in protected {
            let real = super::spellings(path).pop().unwrap_or_else(|| path.clone());
            if !writable.iter().any(|root| real.starts_with(root)) {
                continue;
            }
            if let Some(link) = path.ancestors().find(|ancestor| replaceable_link(ancestor)) {
                return Err(format!(
                    "cannot protect symbolic link {}: its directory entry could be replaced independently of its target",
                    link.display()
                ));
            }
            let metadata = std::fs::metadata(&real).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    format!("Linux cannot write-seal {} inside a writable folder: the protected path does not exist", path.display())
                } else {
                    format!("Linux cannot inspect protected path {} inside a writable folder: {error}", path.display())
                }
            })?;
            // Mount ancestors first so their dentries cannot be renamed out from
            // under a protected leaf. Their contents retain their original access.
            let root = writable
                .iter()
                .filter(|root| real.starts_with(root))
                .min_by_key(|root| root.components().count())
                .expect("a containing grant was checked");
            let mut ancestors: Vec<_> = real
                .ancestors()
                .skip(1)
                .take_while(|ancestor| ancestor.starts_with(root) && *ancestor != root)
                .collect();
            ancestors.reverse();
            for ancestor in ancestors {
                let name = CString::new(ancestor.as_os_str().as_bytes())
                    .map_err(|_| "ancestor contains NUL".to_owned())?;
                if !seals.iter().any(|seal: &Seal| seal.path == name) {
                    seals.push(Seal {
                        path: name,
                        display: real.clone(),
                        readonly: false,
                        directory: true,
                        submounts: false,
                    });
                }
            }
            let name = CString::new(real.as_os_str().as_bytes())
                .map_err(|_| format!("protected path contains NUL: {}", path.display()))?;
            if let Some(seal) = seals.iter_mut().find(|seal| seal.path == name) {
                seal.readonly = true;
                seal.directory = metadata.is_dir();
                seal.submounts = topology::contains_mount(&real, &mountpoints);
            } else {
                seals.push(Seal {
                    path: name,
                    submounts: topology::contains_mount(&real, &mountpoints),
                    display: real,
                    readonly: true,
                    directory: metadata.is_dir(),
                });
            }
        }
        // Earlier seals can add nested mounts too; recursion must cover them.
        let planned: Vec<_> = seals
            .iter()
            .map(|seal| PathBuf::from(std::ffi::OsStr::from_bytes(seal.path.as_bytes())))
            .collect();
        for seal in &mut seals {
            let path = std::path::Path::new(std::ffi::OsStr::from_bytes(seal.path.as_bytes()));
            seal.submounts |= topology::contains_mount(path, &planned);
        }
        // SAFETY: identity queries have no preconditions.
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        let mounts = Self {
            seals,
            uid: format!("{uid} {uid} 1\n").into_bytes(),
            gid: format!("{gid} {gid} 1\n").into_bytes(),
        };
        if !mounts.seals.is_empty() {
            mounts.preflight()?;
        }
        Ok(mounts)
    }

    pub(super) fn build_available(protected: &[PathBuf]) -> Result<(Self, Vec<String>), String> {
        let writable = [PathBuf::from(std::path::MAIN_SEPARATOR_STR)];
        let mut combined = Self::build(&[], &writable)?;
        let mut notes = Vec::new();
        for path in protected {
            match Self::build(std::slice::from_ref(path), &writable) {
                Ok(prepared) => {
                    for seal in prepared.seals {
                        if let Some(existing) =
                            combined.seals.iter_mut().find(|s| s.path == seal.path)
                        {
                            existing.readonly |= seal.readonly;
                        } else {
                            combined.seals.push(seal);
                        }
                    }
                }
                Err(reason) => notes.push(format!("{} not sealed: {reason}", path.display())),
            }
        }
        // Merging independently prepared paths can introduce nested mounts.
        let planned: Vec<_> = combined
            .seals
            .iter()
            .map(|seal| PathBuf::from(std::ffi::OsStr::from_bytes(seal.path.as_bytes())))
            .collect();
        for seal in &mut combined.seals {
            let path = std::path::Path::new(std::ffi::OsStr::from_bytes(seal.path.as_bytes()));
            seal.submounts |= topology::contains_mount(path, &planned);
        }
        Ok((combined, notes))
    }

    pub(super) fn probe() -> Result<(), String> {
        let mounts = Self {
            seals: Vec::new(),
            uid: format!("{0} {0} 1\n", unsafe { libc::geteuid() }).into_bytes(),
            gid: format!("{0} {0} 1\n", unsafe { libc::getegid() }).into_bytes(),
        };
        mounts.preflight()
    }

    pub(super) fn packet(&self) -> [i32; 4] {
        let mut cwd_reentered = false;
        let result = self.apply(&mut cwd_reentered);
        let mut packet = match result {
            Ok(()) => [0; 4],
            Err(f) => [step_index(f.step), f.errno, f.seal as i32, 0],
        };
        packet[3] = i32::from(cwd_reentered);
        packet
    }

    /// Only libc syscalls and stack buffers: safe in a pre_exec hook.
    pub(super) fn apply(&self, cwd_reentered: &mut bool) -> Result<(), Failure> {
        self.apply_with_status(true, cwd_reentered, |_, syscall| syscall())
    }

    fn apply_with(
        &self,
        reenter: bool,
        result: impl FnMut(&'static str, &mut dyn FnMut() -> i32) -> i32,
    ) -> Result<(), Failure> {
        self.apply_with_status(reenter, &mut false, result)
    }

    fn apply_with_status(
        &self,
        reenter: bool,
        cwd_reentered: &mut bool,
        mut result: impl FnMut(&'static str, &mut dyn FnMut() -> i32) -> i32,
    ) -> Result<(), Failure> {
        if reenter && self.seals.is_empty() {
            return Ok(());
        }
        let mut check = |step, syscall: &mut dyn FnMut() -> i32, seal| {
            if result(step, syscall) == 0 {
                Ok(())
            } else {
                Err(Failure {
                    step,
                    errno: io::Error::last_os_error()
                        .raw_os_error()
                        .unwrap_or(libc::EIO),
                    seal,
                })
            }
        };
        unsafe {
            let mut before: libc::stat = std::mem::zeroed();
            if reenter {
                check(
                    "inspect working directory",
                    &mut || topology::cwd_stat(&mut before),
                    0,
                )?;
            }
            check(
                "unshare",
                &mut || libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS),
                0,
            )?;
            let sealing = (|| {
                for (step, path, bytes) in [
                    ("setgroups", c"/proc/self/setgroups", b"deny".as_slice()),
                    ("uid_map", c"/proc/self/uid_map", self.uid.as_slice()),
                    ("gid_map", c"/proc/self/gid_map", self.gid.as_slice()),
                ] {
                    check(step, &mut || write_map(path, bytes), 0)?;
                }
                check(
                    "mount-private",
                    &mut || {
                        libc::mount(
                            std::ptr::null(),
                            c"/".as_ptr(),
                            std::ptr::null(),
                            libc::MS_REC | libc::MS_PRIVATE,
                            std::ptr::null(),
                        )
                    },
                    0,
                )?;
                for (index, seal) in self.seals.iter().enumerate() {
                    let path = seal.path.as_ptr();
                    let mut stat: libc::statfs64 = std::mem::zeroed();
                    check(
                        "statfs",
                        &mut || {
                            #[cfg(target_pointer_width = "64")]
                            {
                                libc::syscall(
                                    libc::SYS_statfs,
                                    path,
                                    &mut stat as *mut libc::statfs64,
                                ) as i32
                            }
                            #[cfg(target_pointer_width = "32")]
                            {
                                libc::syscall(
                                    libc::SYS_statfs64,
                                    path,
                                    std::mem::size_of_val(&stat),
                                    &mut stat as *mut libc::statfs64,
                                ) as i32
                            }
                        },
                        index,
                    )?;
                    check(
                        "bind",
                        &mut || {
                            libc::mount(
                                path,
                                path,
                                std::ptr::null(),
                                libc::MS_BIND | libc::MS_REC,
                                std::ptr::null(),
                            )
                        },
                        index,
                    )?;
                    if !seal.readonly {
                        continue;
                    }
                    let flags = remount_flags(stat.f_flags as libc::c_ulong);
                    check(
                        "remount-readonly",
                        &mut || {
                            libc::mount(
                                std::ptr::null(),
                                path,
                                std::ptr::null(),
                                libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | flags,
                                std::ptr::null(),
                            )
                        },
                        index,
                    )?;
                    if seal.directory {
                        let attributes = [1u64, 0, 0, 0]; // read-only, clear, propagation, namespace
                        let recursive = check(
                            "recursive-readonly",
                            &mut || {
                                libc::syscall(
                                    libc::SYS_mount_setattr,
                                    libc::AT_FDCWD,
                                    path,
                                    libc::AT_RECURSIVE,
                                    attributes.as_ptr(),
                                    std::mem::size_of_val(&attributes),
                                ) as i32
                            },
                            index,
                        );
                        if let Err(failure) = recursive {
                            if failure.errno != libc::ENOSYS || seal.submounts {
                                return Err(failure);
                            }
                        }
                    }
                }
                Ok(())
            })();
            let mut first_failure = sealing.err();
            let mut retain = |result: Result<(), Failure>| {
                if first_failure.is_none() {
                    first_failure = result.err();
                }
            };
            // A mount-only boundary must survive execution even for an identity
            // whose numeric ID is zero. The bounding-set drop prevents a later
            // execution from restoring the capability needed to undo seals.
            retain(check(
                "drop mount capability",
                &mut || libc::prctl(libc::PR_CAPBSET_DROP, 21 as libc::c_ulong, 0, 0, 0),
                0,
            ));
            retain(check(
                "prevent privilege gain",
                &mut || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0),
                0,
            ));
            let header = [0x2008_0522u32, 0]; // capability interface version, current process
            let capabilities = [0u32; 6]; // effective, permitted and inheritable, both words
            retain(check(
                "clear process capabilities",
                &mut || {
                    libc::syscall(libc::SYS_capset, header.as_ptr(), capabilities.as_ptr()) as i32
                },
                0,
            ));
            if reenter {
                let reentry = check(
                    "re-enter working directory",
                    &mut || topology::reenter(&before),
                    0,
                );
                *cwd_reentered = reentry.is_ok();
                retain(reentry);
            }
            if let Some(failure) = first_failure {
                return Err(failure);
            }
        }
        Ok(())
    }
}

fn write_map(path: &std::ffi::CStr, bytes: &[u8]) -> i32 {
    // SAFETY: static path, prepared bytes, and the fresh descriptor is closed.
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return -1;
        }
        let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
        let errno = *libc::__errno_location();
        libc::close(fd);
        if written == bytes.len() as isize {
            return 0;
        }
        *libc::__errno_location() = if written >= 0 { libc::EIO } else { errno };
        -1
    }
}

fn remount_flags(source: libc::c_ulong) -> libc::c_ulong {
    let mut flags = 0;
    for (stat, mount) in [
        (libc::ST_NOSUID, libc::MS_NOSUID),
        (libc::ST_NODEV, libc::MS_NODEV),
        (libc::ST_NOEXEC, libc::MS_NOEXEC),
        (libc::ST_NOATIME, libc::MS_NOATIME),
        (libc::ST_NODIRATIME, libc::MS_NODIRATIME),
        (libc::ST_RELATIME, libc::MS_RELATIME),
    ] {
        if source & stat != 0 {
            flags |= mount;
        }
    }
    if source & (libc::ST_NOATIME | libc::ST_RELATIME) == 0 {
        flags |= libc::MS_STRICTATIME;
    }
    flags
}

const STEPS: &[&str] = &[
    "inspect working directory",
    "unshare",
    "setgroups",
    "uid_map",
    "gid_map",
    "mount-private",
    "statfs",
    "bind",
    "remount-readonly",
    "recursive-readonly",
    "drop mount capability",
    "prevent privilege gain",
    "clear process capabilities",
    "re-enter working directory",
];
fn step_index(step: &str) -> i32 {
    STEPS
        .iter()
        .position(|candidate| *candidate == step)
        .unwrap_or(0) as i32
        + 1
}

#[cfg(test)]
#[path = "write_boundary_mounts_tests.rs"]
mod tests;

fn replaceable_link(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    if !path.is_symlink() {
        return false;
    }
    let Some(parent) = path.parent() else {
        return true;
    };
    let Ok(metadata) = std::fs::metadata(parent) else {
        return true;
    };
    // Ownership permits chmod even when the current mode disallows writes.
    if metadata.uid() == unsafe { libc::geteuid() } {
        return true;
    }
    let Ok(parent) = CString::new(parent.as_os_str().as_bytes()) else {
        return true;
    };
    unsafe {
        libc::faccessat(
            libc::AT_FDCWD,
            parent.as_ptr(),
            libc::W_OK,
            libc::AT_EACCESS,
        ) == 0
    }
}

#[cfg(test)]
#[path = "write_boundary_nested_mount_tests.rs"]
mod nested_tests;

#[path = "write_boundary_mounts_topology.rs"]
mod topology;

#[path = "write_boundary_mounts_preflight.rs"]
mod preflight;

#[path = "write_boundary_mounts_outcome.rs"]
mod outcome;

#[cfg(test)]
#[path = "write_boundary_mounts_cleanup_tests.rs"]
mod cleanup_tests;
