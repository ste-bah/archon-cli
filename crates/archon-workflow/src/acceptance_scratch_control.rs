//! Synchronous filesystem/git phases cooperate with the observation deadline.
//! Scope never crosses an await, so a runtime worker cannot inherit another task's control.
use super::*;
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone)]
pub(super) struct Control {
    deadline: Instant,
    cancel: Arc<AtomicBool>,
}
thread_local! { static ACTIVE:RefCell<Option<Control>>=const { RefCell::new(None) }; }
impl Control {
    pub fn new(seconds: u64, cancel: Arc<AtomicBool>) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(seconds.min(86400)),
            cancel,
        }
    }
    pub fn check(&self) -> WorkflowResult<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(invalid(
                "observation parent closed or cancellation requested",
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(invalid("native observation phase deadline exceeded"));
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
        f()
    }
}
pub(super) fn check() -> WorkflowResult<()> {
    ACTIVE.with(|c| match c.borrow().as_ref() {
        Some(c) => c.check(),
        None => Ok(()),
    })
}
pub(super) fn git(root: &Path, args: &[&str], paths: &[&Path]) -> WorkflowResult<String> {
    use std::os::unix::process::CommandExt;
    use std::{io::Read, process::Stdio};
    check()?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .args(paths)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|e| WorkflowError::io(root, e))?;
    let pgid = child.id() as i32;
    let drain = |mut pipe: Box<dyn Read + Send>| {
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
            }
            Ok::<_, std::io::Error>(out)
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let fallback = Instant::now() + Duration::from_secs(300);
    let outcome = loop {
        if let Err(e) = check() {
            break Err(e);
        }
        if Instant::now() >= fallback {
            break Err(invalid("scratch git deadline exceeded"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(WorkflowError::io(root, e)),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
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
