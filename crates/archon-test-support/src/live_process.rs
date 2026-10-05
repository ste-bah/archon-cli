//! Issue 339: a real process that runs until the test ends it.
//!
//! A liveness test needs a pid that is running on every platform. Pid 1 is
//! not one on Windows, so tests spawn this child instead, read its pid while
//! it runs, and end it to get a pid whose process is gone.

use std::process::{Child, Command, Stdio};

/// A child process that sleeps until [`LiveChild::end`] or drop ends it.
pub struct LiveChild {
    child: Child,
    ended: bool,
}

impl LiveChild {
    /// Spawns a child that runs for about a minute without any input.
    ///
    /// # Panics
    /// When the platform sleeper cannot be spawned.
    pub fn spawn() -> Self {
        let child = sleeper()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a sleeping child process");
        Self {
            child,
            ended: false,
        }
    }

    /// The child's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Kills the child and waits for it, so its process is gone. The child
    /// stays owned here, so on Windows its handle still pins the pid and no
    /// new process can reuse it while the test asserts on it.
    ///
    /// # Panics
    /// When the child cannot be waited for.
    pub fn end(&mut self) {
        if self.ended {
            return;
        }
        // An error here means it already exited; the wait below still reaps it.
        let _ = self.child.kill();
        self.child.wait().expect("wait for the killed child");
        self.ended = true;
    }
}

impl Drop for LiveChild {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(windows)]
fn sleeper() -> Command {
    // ping.exe is one process with no console input, unlike `timeout`, and
    // unlike `cmd /C` it leaves no grandchild behind when killed.
    let mut command = Command::new("ping");
    command.args(["-n", "60", "127.0.0.1"]);
    command
}

#[cfg(not(windows))]
fn sleeper() -> Command {
    let mut command = Command::new("sleep");
    command.arg("60");
    command
}
