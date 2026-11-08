use super::*;

impl MountSeals {
    /// Check in a disposable child so the parent can report the actual failing
    /// step and errno before returning a prepared command. Never changes the host.
    pub(super) fn preflight(&self) -> Result<(), String> {
        let mut pipe = [-1; 2];
        // SAFETY: valid output buffer; pipe descriptors are closed on every path.
        if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(format!(
                "mount seal diagnostic pipe: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: child only calls the raw-syscall hook, writes a fixed packet, exits.
        let pid = unsafe { crate::spawn::fork_probe() };
        if pid == 0 {
            unsafe {
                libc::close(pipe[0]);
            }
            let packet = match self.apply_with(false, |_, syscall| syscall()) {
                Ok(()) => [0i32; 3],
                Err(failure) => [step_index(failure.step), failure.errno, failure.seal as i32],
            };
            unsafe {
                libc::write(
                    pipe[1],
                    packet.as_ptr().cast(),
                    std::mem::size_of_val(&packet),
                );
                libc::_exit(0);
            }
        }
        unsafe {
            libc::close(pipe[1]);
        }
        if pid < 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(pipe[0]);
            }
            return Err(format!("mount seal preflight fork: {error}"));
        }
        let mut packet = [0i32; 3];
        let mut read = 0;
        while read < std::mem::size_of_val(&packet) {
            let n = unsafe {
                libc::read(
                    pipe[0],
                    packet.as_mut_ptr().cast::<u8>().add(read).cast(),
                    std::mem::size_of_val(&packet) - read,
                )
            };
            if n > 0 {
                read += n as usize;
            } else if n < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            } else {
                break;
            }
        }
        unsafe {
            libc::close(pipe[0]);
            while libc::waitpid(pid, std::ptr::null_mut(), 0) < 0 {
                if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                    break;
                }
            }
        }
        if read != std::mem::size_of_val(&packet) {
            return Err("mount seal preflight child did not report a result".into());
        }
        if packet[0] != 0 {
            return Err(Failure {
                step: STEPS[(packet[0] - 1) as usize],
                errno: packet[1],
                seal: packet[2] as usize,
            }
            .message(self));
        }
        Ok(())
    }
}
