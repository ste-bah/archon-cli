//! Process ownership using the same tracker and job primitives as other checks.
use std::io;
use std::process::{Child, Command};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

pub(super) struct Tree {
    #[cfg(unix)]
    tracker: archon_shell::process_tree::Tracker,
    #[cfg(windows)]
    job: archon_shell::job_object::Job,
}

impl Tree {
    pub(super) fn prepare(command: &mut Command) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            // SAFETY: the child hook uses only an async-signal-safe syscall.
            unsafe {
                command.pre_exec(archon_shell::process_tree::become_subreaper);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(archon_shell::job_object::CREATE_SUSPENDED_FLAG);
        }
        Ok(())
    }

    pub(super) fn abort(child: &mut Child) {
        #[cfg(unix)]
        // SAFETY: the unreaped child's process group was created at spawn.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.kill();
    }

    pub(super) fn adopt(child: &mut Child) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let pid = child.id();
            let start = match archon_shell::process_tree::identity_of(pid)? {
                Some(start) => start,
                // A proven exited, unreaped leader still reserves its group
                // identity. The tracker can collect that group's descendants.
                None if archon_shell::process_tree::exited(pid)? => 0,
                None => {
                    return Err(io::Error::other(
                        "child identity unavailable before reaping",
                    ));
                }
            };
            Ok(Self {
                tracker: archon_shell::process_tree::Tracker::new(
                    archon_shell::process_tree::Pinned { pid, start },
                    vec![pid],
                    Vec::new(),
                ),
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let job = archon_shell::job_object::Job::create(None)?;
            job.adopt_suspended(child.as_raw_handle(), child.id())?;
            Ok(Self { job })
        }
    }

    pub(super) fn scan(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        self.tracker
            .refresh(Instant::now() + Duration::from_millis(500))?;
        Ok(())
    }

    pub(super) fn exited(&self, child: &mut Child) -> io::Result<bool> {
        #[cfg(unix)]
        {
            archon_shell::process_tree::exited(child.id())
        }
        #[cfg(windows)]
        {
            Ok(child.try_wait()?.is_some())
        }
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let survivors = self.tracker.kill(Duration::from_secs(3))?;
            if !survivors.is_empty() {
                return Err(io::Error::other(format!(
                    "{} process(es) survived teardown",
                    survivors.len()
                )));
            }
        }
        #[cfg(windows)]
        {
            let active = self.job.kill_and_confirm(Duration::from_secs(3))?;
            if active != 0 {
                return Err(io::Error::other(format!(
                    "{active} process(es) survived teardown"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn reaped(&self) {
        #[cfg(unix)]
        self.tracker.leader_reaped();
    }
}
