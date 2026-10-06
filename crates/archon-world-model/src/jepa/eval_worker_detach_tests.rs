use super::*;
static CHILDREN: std::sync::Mutex<()> = std::sync::Mutex::new(());
unsafe extern "C" fn fails() -> libc::pid_t {
    -1
}
unsafe extern "C" fn succeeds() -> libc::pid_t {
    1
}
static FORKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
unsafe extern "C" fn must_not_fork() -> libc::pid_t {
    FORKED.store(true, std::sync::atomic::Ordering::SeqCst);
    1
}

#[test]
fn failed_setsid_refuses_detachment_before_fork() {
    let result = unsafe { worker_detach_step(fails, must_not_fork) };
    assert!(!FORKED.load(std::sync::atomic::Ordering::SeqCst));
    assert!(result.is_err());
}
#[test]
fn failed_second_fork_refuses_detachment() {
    assert!(unsafe { worker_detach_step(succeeds, fails) }.is_err());
}
#[test]
fn failed_child_setup_is_reported_to_the_parent() {
    let _guard = CHILDREN.lock().unwrap_or_else(|error| error.into_inner());
    let child = unsafe { libc::fork() };
    assert!(child >= 0);
    if child == 0 {
        unsafe { libc::_exit(1) }
    }
    assert!(worker_wait(child).is_err());
}
#[test]
fn unrelated_exited_child_keeps_its_wait_status() {
    let _guard = CHILDREN.lock().unwrap_or_else(|error| error.into_inner());
    // Hold our first child until wait() begins, leaving only the unrelated child waitable.
    let unrelated = unsafe { libc::fork() };
    assert!(unrelated >= 0);
    if unrelated == 0 {
        unsafe { libc::_exit(23) }
    }
    // WNOWAIT observes exit without consuming the unrelated child's status.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            libc::waitid(
                libc::P_PID,
                unrelated as _,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        },
        0
    );
    let child = unsafe { libc::fork() };
    assert!(child >= 0);
    if child == 0 {
        unsafe {
            libc::usleep(100_000);
            libc::_exit(0)
        }
    }
    let result = worker_wait(child);
    let mut status = 0;
    let reaped = unsafe { libc::waitpid(unrelated, &mut status, 0) };
    let exit = libc::WEXITSTATUS(status);
    // Clean up even on the old code (which consumed the unrelated status).
    unsafe { libc::waitpid(child, &mut status, 0) };
    assert!(result.is_ok());
    assert_eq!(reaped, unrelated, "the helper stole another child's status");
    assert_eq!(exit, 23);
}

#[test]
fn a_signalled_first_child_is_a_detachment_error() {
    let _guard = CHILDREN.lock().unwrap_or_else(|error| error.into_inner());
    let child = unsafe { libc::fork() };
    assert!(child >= 0);
    if child == 0 {
        unsafe {
            let mut signals = std::mem::zeroed();
            libc::sigemptyset(&mut signals);
            libc::sigaddset(&mut signals, libc::SIGTERM);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::sigprocmask(libc::SIG_UNBLOCK, &signals, std::ptr::null_mut());
            libc::raise(libc::SIGTERM);
            libc::_exit(0);
        }
    }
    assert!(worker_wait(child).is_err());
}
