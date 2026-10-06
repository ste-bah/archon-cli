//! A post-fork allocation aborts the child instead of risking an allocator deadlock.
#![cfg(unix)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI32, Ordering};

struct ForkGuard;
static PARENT: AtomicI32 = AtomicI32::new(0);

fn forbid_child_allocation() {
    let parent = PARENT.load(Ordering::Relaxed);
    // SAFETY: getpid and _exit touch no Rust state, locks or heap memory.
    if parent != 0 && unsafe { libc::getpid() } != parent {
        unsafe { libc::_exit(118) };
    }
}

// SAFETY: delegates to System in the parent, and exits before touching the heap in a child.
unsafe impl GlobalAlloc for ForkGuard {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        forbid_child_allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        forbid_child_allocation();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        forbid_child_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: ForkGuard = ForkGuard;

fn spawn(controlling: bool, mask: Option<libc::mode_t>) {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    PARENT.store(unsafe { libc::getpid() }, Ordering::Relaxed);
    let pair = native_pty_system().openpty(PtySize::default()).unwrap();
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    command.set_controlling_tty(controlling);
    command.umask(mask);
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("fork hook must not allocate");
    assert!(
        child.wait().unwrap().success(),
        "child allocated before exec"
    );
}

#[test]
fn controlling_terminal_spawn_does_not_allocate_after_fork() {
    spawn(true, None);
}
#[test]
fn spawn_without_controlling_terminal_does_not_allocate_after_fork() {
    spawn(false, None);
}
#[test]
fn spawn_with_umask_does_not_allocate_after_fork() {
    spawn(true, Some(0o077));
}
