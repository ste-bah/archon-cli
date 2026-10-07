//! Synchronous filesystem/git phases under a renewable no-progress window.
//!
//! Every [`check`] is reached only after a unit of real work (a file copied
//! or hashed, a directory listed, a git call answered): it renews the
//! window and reports coalesced activity, so the parent that watches this
//! process (the guardian's launcher, a host-command supervisor) sees a
//! progressing phase as alive however long it runs. A phase never has a
//! total. Only a wait on something outside this process (a lock another
//! holder keeps, a git child with no output and no CPU) is judged by
//! [`poll`]: no progress for the window is [`OBSERVATION_STALLED`], a
//! resumable pause, never a failure. A syscall that never returns cannot be
//! seen here at all; the watching parent's own window ends it.
//! Scope never crosses an await, so a runtime worker cannot inherit another task's control.
use super::*;
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// The marker of a no-progress stall of an observation phase or of a git
/// child: an operational, resumable pause (the host may be at fault).
pub const OBSERVATION_STALLED: &str = "native observation made no progress";
/// The window of a git child outside any phase control.
const DEFAULT_WINDOW_SECS: u64 = 300;

#[derive(Clone)]
pub(super) struct Control {
    window: Duration,
    last: Arc<Mutex<Instant>>,
    cancel: Arc<AtomicBool>,
}
thread_local! { static ACTIVE:RefCell<Option<Control>>=const { RefCell::new(None) }; }
impl Control {
    pub fn new(seconds: u64, cancel: Arc<AtomicBool>) -> Self {
        Self {
            window: Duration::from_secs(seconds.clamp(1, 86400)),
            last: Arc::new(Mutex::new(Instant::now())),
            cancel,
        }
    }
    fn cancelled(&self) -> WorkflowResult<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(invalid(
                "observation parent closed or cancellation requested",
            ));
        }
        Ok(())
    }
    /// A progress point: renews the window and reports activity.
    pub fn check(&self) -> WorkflowResult<()> {
        self.cancelled()?;
        self.progress();
        Ok(())
    }
    fn progress(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        pulse(self.window);
    }
    /// A wait point: no renewal; a stall once the window passes unrenewed.
    pub fn poll(&self, waiting_on: &str) -> WorkflowResult<()> {
        self.cancelled()?;
        let last = *self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= self.window {
            return Err(stalled(self.window, waiting_on));
        }
        Ok(())
    }
    pub fn run<T>(&self, f: impl FnOnce() -> WorkflowResult<T>) -> WorkflowResult<T> {
        struct Restore(Option<Control>);
        impl Drop for Restore {
            fn drop(&mut self) {
                ACTIVE.with(|c| *c.borrow_mut() = self.0.take());
            }
        }
        self.check()?;
        let _restore = Restore(ACTIVE.with(|c| c.replace(Some(self.clone()))));
        let result = f();
        // A finished phase is progress too, even one with no inner points.
        if result.is_ok() {
            self.progress();
        }
        result
    }
}

/// The first of `errors` (an observation's recorded operational errors)
/// that is a no-progress stall: the host's, so its caller pauses.
pub fn observation_stall<'a>(errors: impl IntoIterator<Item = &'a String>) -> Option<&'a String> {
    errors
        .into_iter()
        .find(|error| error.contains(OBSERVATION_STALLED))
}

/// The resumable stall error for `window` while waiting on `what`.
fn stalled(window: Duration, what: &str) -> WorkflowError {
    WorkflowError::ControlPaused(format!(
        "{OBSERVATION_STALLED} for {}s while waiting on {what}; resumable",
        window.as_secs()
    ))
}

/// One coalesced activity report for the whole process, at a cadence below
/// `window` (at most a quarter of it, at most 60 s).
fn pulse(window: Duration) {
    #[cfg(test)]
    TEST_PULSES.with(|n| n.set(n.get() + 1));
    static PULSE: OnceLock<archon_shell::progress::Progress> = OnceLock::new();
    let pulse = PULSE.get_or_init(|| archon_shell::progress::Progress::new(true));
    let cadence = (window / 4).clamp(Duration::from_secs(1), Duration::from_secs(60));
    pulse.set_report_interval(cadence);
    pulse.record();
}

#[cfg(test)]
thread_local! {
    pub(super) static TEST_PULSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn active() -> Option<Control> {
    ACTIVE.with(|c| c.borrow().clone())
}
/// A progress point of the active phase, if any.
pub(super) fn check() -> WorkflowResult<()> {
    match active() {
        Some(c) => c.check(),
        None => Ok(()),
    }
}
/// A wait point of the active phase, if any (see [`Control::poll`]).
pub(super) fn poll(waiting_on: &str) -> WorkflowResult<()> {
    match active() {
        Some(c) => c.poll(waiting_on),
        None => Ok(()),
    }
}
pub(super) fn git(root: &Path, args: &[&str], paths: &[&Path]) -> WorkflowResult<String> {
    use std::{io::Read, process::Stdio};
    check()?;
    let mut command = archon_shell::spawn::command("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .args(paths)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| WorkflowError::io(root, e))?;
    #[cfg(unix)]
    let pgid = child.id() as i32;
    // Output bytes and the child's CPU time are its progress.
    let received = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let received = received.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0; 8192];
            loop {
                let n = pipe.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                if out.len() + n > 4 * 1024 * 1024 {
                    return Err(std::io::Error::other("git output limit"));
                }
                out.extend_from_slice(&buf[..n]);
                received.fetch_add(n as u64, Ordering::SeqCst);
            }
            Ok::<_, std::io::Error>(out)
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    // Its own renewable window: the active phase's, else the default. Never
    // a total: a long checkout that keeps working is never cut.
    let phase = active();
    let watch = Control {
        window: (phase.as_ref()).map_or(Duration::from_secs(DEFAULT_WINDOW_SECS), |c| c.window),
        last: Arc::new(Mutex::new(Instant::now())),
        cancel: (phase.as_ref()).map_or_else(Default::default, |c| c.cancel.clone()),
    };
    let leader = child.id();
    let pin = archon_shell::process_tree::Pinned {
        pid: leader,
        start: archon_shell::process_tree::start_of(leader).unwrap_or_default(),
    };
    let mut cpu = archon_shell::process_tree::Activity::default();
    let (mut seen, mut sampled) = (0, Instant::now());
    let outcome = loop {
        let bytes = received.load(Ordering::SeqCst);
        let mut active = bytes != seen;
        seen = bytes;
        if sampled.elapsed() >= Duration::from_secs(1) {
            sampled = Instant::now();
            // An unreadable sample never manufactures progress.
            active |= cpu
                .observe(&[pin], sampled + Duration::from_secs(2))
                .unwrap_or(false);
        }
        if active {
            watch.progress();
            if let Some(phase) = &phase {
                phase.progress();
            }
        }
        if let Err(e) = watch.poll("a git child with no output or CPU activity") {
            break Err(e);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(WorkflowError::io(root, e)),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Unix reaps the whole group; Windows has no process group to signal, so
    // only a leader that is still running can be killed there.
    #[cfg(unix)]
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    let reap = Instant::now() + Duration::from_secs(3);
    while child
        .try_wait()
        .map_err(|e| WorkflowError::io(root, e))?
        .is_none()
    {
        if Instant::now() >= reap {
            return Err(invalid("scratch git reap deadline exceeded"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    while !stdout.is_finished() || !stderr.is_finished() {
        if Instant::now() >= reap {
            return Err(invalid("scratch git pipe deadline exceeded"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let out = stdout
        .join()
        .map_err(|_| invalid("git stdout reader failed"))?
        .map_err(|e| WorkflowError::io(root, e))?;
    let err = stderr
        .join()
        .map_err(|_| invalid("git stderr reader failed"))?
        .map_err(|e| WorkflowError::io(root, e))?;
    if !outcome?.success() {
        return Err(invalid(format!(
            "scratch git command failed: {}",
            String::from_utf8_lossy(&err)
        )));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
#[path = "acceptance_scratch_control_tests.rs"]
mod tests;
