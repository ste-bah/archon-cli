// Syscall-only child setup, and ownership of the first child's wait status.
unsafe fn worker_detach_step(
    session: unsafe extern "C" fn() -> libc::pid_t,
    fork: unsafe extern "C" fn() -> libc::pid_t,
) -> std::io::Result<libc::pid_t> {
    // SAFETY: supplied functions are the POSIX syscalls (or test syscall substitutes).
    if unsafe { session() } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let pid = unsafe { fork() };
    if pid == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(pid)
}

fn worker_wait(pid: libc::pid_t) -> std::io::Result<()> {
    if pid <= 0 {
        return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
    }
    let mut status = 0;
    loop {
        // SAFETY: reaps only the first child this caller owns.
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        if waited == pid {
            break;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
        Ok(())
    } else {
        Err(std::io::Error::other("background worker detachment failed"))
    }
}

#[cfg(test)]
#[path = "eval_worker_detach_tests.rs"]
mod worker_detach_tests;
