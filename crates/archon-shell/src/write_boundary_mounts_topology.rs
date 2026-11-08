//! Inspect mount topology before fork; re-enter the same directory after sealing.
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

pub(super) fn mountpoints() -> Result<Vec<PathBuf>, String> {
    let bytes = std::fs::read("/proc/self/mountinfo")
        .map_err(|error| format!("cannot inspect mount topology: {error}"))?;
    Ok(parse_mountpoints(&bytes))
}

fn parse_mountpoints(bytes: &[u8]) -> Vec<PathBuf> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let field = line.split(|byte| *byte == b' ').nth(4)?;
            let mut decoded = Vec::new();
            let mut index = 0;
            while index < field.len() {
                if field[index] == b'\\'
                    && index + 3 < field.len()
                    && field[index + 1..index + 4]
                        .iter()
                        .all(|byte| (b'0'..=b'7').contains(byte))
                {
                    let value = ((field[index + 1] - b'0') as u16 * 64
                        + (field[index + 2] - b'0') as u16 * 8
                        + (field[index + 3] - b'0') as u16) as u8;
                    decoded.push(value);
                    index += 4;
                } else {
                    decoded.push(field[index]);
                    index += 1;
                }
            }
            Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
        })
        .collect()
}

pub(super) fn contains_mount(path: &Path, mounts: &[PathBuf]) -> bool {
    mounts
        .iter()
        .any(|mount| mount != path && mount.starts_with(path))
}

pub(super) unsafe fn cwd_stat(stat: &mut libc::stat) -> i32 {
    unsafe { libc::fstatat(libc::AT_FDCWD, c".".as_ptr(), stat, 0) }
}

pub(super) unsafe fn reenter(before: &libc::stat) -> i32 {
    unsafe {
        let mut cwd = [0u8; libc::PATH_MAX as usize];
        if libc::syscall(libc::SYS_getcwd, cwd.as_mut_ptr(), cwd.len()) < 0
            || libc::syscall(libc::SYS_chdir, cwd.as_ptr()) < 0
        {
            return -1;
        }
        let mut after: libc::stat = std::mem::zeroed();
        if cwd_stat(&mut after) != 0 {
            return -1;
        }
        if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
            *libc::__errno_location() = libc::ESTALE;
            return -1;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_decodes_escaped_mountpoints_and_checks_descendants() {
        let paths = parse_mountpoints(
            br"1 2 3:4 / /sealed rw - tmpfs none rw
2 1 3:4 / /sealed/with\040space rw - tmpfs none rw
3 1 3:4 / /sealed/with\134slash rw - tmpfs none rw
4 1 3:4 / /sealed-other rw - tmpfs none rw
",
        );
        assert_eq!(paths[1], PathBuf::from("/sealed/with space"));
        assert_eq!(paths[2], PathBuf::from(r"/sealed/with\slash"));
        assert!(contains_mount(Path::new("/sealed"), &paths));
        assert!(!contains_mount(Path::new("/sealed-other"), &paths));
        assert!(!contains_mount(Path::new("/seal"), &paths));
    }
}
