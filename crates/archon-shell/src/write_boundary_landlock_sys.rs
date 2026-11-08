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
    ruleset_rights(grants, super::HANDLED_RIGHTS, super::FILE_RIGHTS)
}

pub(super) fn ruleset_rights(
    grants: &[PathBuf],
    rights: u64,
    file_rights: u64,
) -> Result<OwnedFd, String> {
    let attr = RulesetAttr {
        handled_access_fs: rights,
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
        allow(&ruleset, path, rights, file_rights)?;
    }
    Ok(ruleset)
}

fn allow(
    ruleset: &OwnedFd,
    path: &std::path::Path,
    rights: u64,
    file_rights: u64,
) -> Result<(), String> {
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
        allowed_access: if directory { rights } else { file_rights },
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

/// Test restriction in a disposable child without resolving any host executable.
pub(super) fn probe_restriction(ruleset: &OwnedFd) -> Result<(), String> {
    // SAFETY: the child only restricts itself with raw syscalls and exits.
    let pid = unsafe { crate::spawn::fork_probe() };
    if pid < 0 {
        return Err(format!(
            "Landlock trial fork failed: {}",
            io::Error::last_os_error()
        ));
    }
    if pid == 0 {
        let code = match restrict(ruleset) {
            Ok(()) => 0,
            Err(error) => error.raw_os_error().unwrap_or(libc::EIO),
        };
        // Linux errno values fit in the exit status byte.
        unsafe {
            libc::_exit(code);
        }
    }
    let mut status = 0;
    loop {
        // SAFETY: wait for exactly the child created above, into a valid buffer.
        if unsafe { libc::waitpid(pid, &mut status, 0) } >= 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(format!("Landlock trial wait failed: {error}"));
        }
    }
    if !libc::WIFEXITED(status) {
        return Err(format!(
            "Landlock trial child did not exit normally: wait status {status}"
        ));
    }
    let errno = libc::WEXITSTATUS(status);
    if errno != 0 {
        return Err(format!(
            "a Landlock ruleset cannot be applied to a child of this process: {}",
            io::Error::from_raw_os_error(errno)
        ));
    }
    Ok(())
}
