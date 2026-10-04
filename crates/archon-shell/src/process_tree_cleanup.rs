//! Shared cleanup lifetime and bounded tracker access.
use std::io;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(5);
static CLEANUPS: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());

static EXIT_HOOK: OnceLock<i32> = OnceLock::new();

/// libc exit hooks cover both normal Rust main returns and process::exit.
/// Cleanup uses private runtimes, so it can finish after main's runtime drops.
pub(super) fn install_exit_drain() -> io::Result<()> {
    let result = EXIT_HOOK.get_or_init(|| {
        // SAFETY: the callback has C ABI, static lifetime, and catches panics.
        unsafe { libc::atexit(exit_drain) }
    });
    if *result == 0 {
        Ok(())
    } else {
        Err(io::Error::other("could not register cleanup exit hook"))
    }
}

extern "C" fn exit_drain() {
    let _ = std::panic::catch_unwind(|| {
        let bound = Duration::from_secs(10);
        let deadline = Instant::now() + bound;
        let cleanup = drain_cleanup(bound);
        let probes =
            super::bounded::drain_probes(deadline.saturating_duration_since(Instant::now()));
        if !cleanup || !probes {
            tracing::warn!("shutdown cleanup timed out; survivors remain unknown");
        }
    });
}

/// Try a lock within the caller's end-to-end budget. Run on a blocking
/// worker, never on an async runtime thread.
pub fn lock_until<T>(mutex: &Mutex<T>, deadline: Instant) -> io::Result<MutexGuard<'_, T>> {
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "tree tracker stayed busy",
            ));
        }
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(io::Error::other("tracker poisoned"));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
            }
        }
    }
}

/// Keep cleanup workers alive through CLI shutdown. Finished handles are
/// joined at registration too, so the registry does not grow with commands.
pub fn register_cleanup(handle: std::thread::JoinHandle<()>) {
    if let Err(error) = install_exit_drain() {
        tracing::warn!(%error, "cleanup shutdown drain unavailable; survivors unknown");
    }
    let mut cleanups = CLEANUPS.lock().unwrap_or_else(|error| error.into_inner());
    let mut index = 0;
    while index < cleanups.len() {
        if cleanups[index].is_finished() {
            let _ = cleanups.swap_remove(index).join();
        } else {
            index += 1;
        }
    }
    cleanups.push(handle);
}

/// Bounded shutdown drain. False means cleanup is still unknown; records
/// remain fail-closed until their workers have settled them.
pub fn drain_cleanup(bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        {
            let Ok(mut cleanups) = lock_until(&CLEANUPS, deadline) else {
                return false;
            };
            let mut index = 0;
            while index < cleanups.len() {
                if cleanups[index].is_finished() {
                    let _ = cleanups.swap_remove(index).join();
                } else {
                    index += 1;
                }
            }
            if cleanups.is_empty() {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) static SERIAL: Mutex<()> = Mutex::new(());
    #[test]
    fn cli_exit_drains_registered_cleanup() {
        const FLAG: &str = "ARCHON_TEST_CLEANUP_EXIT";
        if let Some(path) = std::env::var_os(FLAG) {
            register_cleanup(std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                std::fs::write(path, "finished").unwrap();
            }));
            std::process::exit(0);
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("finished");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process_tree::cleanup::tests::cli_exit_drains_registered_cleanup",
            ])
            .env(FLAG, &path)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            path.exists(),
            "CLI exit abandoned a registered cleanup worker"
        );
    }
    #[test]
    fn busy_tracker_acquisition_obeys_the_callers_deadline() {
        let mutex = std::sync::Arc::new(Mutex::new(()));
        let held = mutex.lock().unwrap();
        let copy = mutex.clone();
        let task = std::thread::spawn(move || {
            let start = Instant::now();
            let result = lock_until(&copy, start + Duration::from_millis(30));
            (result.is_err(), start.elapsed())
        });
        std::thread::sleep(Duration::from_millis(150));
        drop(held);
        let (unknown, elapsed) = task.join().unwrap();
        assert!(
            unknown && elapsed < Duration::from_millis(100),
            "busy lock took {elapsed:?}"
        );
    }
    #[test]
    fn shutdown_drain_waits_for_registered_cleanup() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = completed.clone();
        register_cleanup(std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        assert!(drain_cleanup(Duration::from_secs(1)));
        assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
    }
}

#[cfg(test)]
#[test]
fn shutdown_drain_reports_unfinished_cleanup_within_its_bound() {
    let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    register_cleanup(std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(150))
    }));
    let begin = Instant::now();
    assert!(!drain_cleanup(Duration::from_millis(20)));
    assert!(begin.elapsed() < Duration::from_millis(100));
    assert!(drain_cleanup(Duration::from_secs(1)));
}
