//! Issue 330: a lock the process released is free at once, even while a
//! child that another thread forked still shares the lock's open file.
//!
//! A `flock` lock belongs to the open file, and `fork` gives the child that
//! same open file until its `exec` closes the CLOEXEC descriptor. Under load
//! that window grows long, and a released lease or chain lock then looked
//! held: a free run was refused as live. Each test pauses a forked child
//! before its `exec`, releases the lock, and takes it again at once.

#![cfg(unix)]

use super::*;
use crate::command::workflow_task_set::ChainLock;
use std::{io::Read, os::fd::AsRawFd, os::unix::process::CommandExt, thread::JoinHandle};

/// How long the forked child waits before its `exec`.
const PAUSE_MICROS: libc::useconds_t = 1_500_000;

/// Forks a child from another thread and returns once that child has
/// started and paused before its `exec`: it holds a copy of every open file
/// of this process.
fn fork_child_paused_before_exec() -> JoinHandle<()> {
    let (mut started, signal) = std::io::pipe().unwrap();
    let signal_fd = signal.as_raw_fd();
    let spawner = std::thread::spawn(move || {
        let mut command = std::process::Command::new("true");
        // SAFETY: the closure runs in the forked child before `exec` and
        // calls only `write` and `usleep`, which are async-signal-safe.
        unsafe {
            command.pre_exec(move || {
                libc::write(signal_fd, b"x".as_ptr().cast(), 1);
                libc::usleep(PAUSE_MICROS);
                Ok(())
            });
        }
        let status = command.status().unwrap();
        drop(signal);
        assert!(status.success());
    });
    let mut byte = [0u8; 1];
    started.read_exact(&mut byte).unwrap();
    spawner
}

#[test]
fn a_released_executor_lease_is_free_while_a_forked_child_has_not_exec_d() {
    let temp = tempfile::tempdir().unwrap();
    let lease = acquire(temp.path(), "wf-330").unwrap();
    let child = fork_child_paused_before_exec();

    // The lease is held: a second holder is refused while it is.
    let refused = acquire(temp.path(), "wf-330").unwrap_err();
    assert!(refused.to_string().contains("is live"), "{refused:#}");

    drop(lease);
    acquire(temp.path(), "wf-330")
        .expect("a released lease is free although a forked child still shares its file");
    child.join().unwrap();
}

#[test]
fn a_released_chain_lock_is_free_while_a_forked_child_has_not_exec_d() {
    let temp = tempfile::tempdir().unwrap();
    let pins = temp.path().join(".archon/task-set-pins");
    let tasks = temp.path().join("tasks");
    std::fs::create_dir_all(&pins).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    let pin = pins.join("set.json");
    let lock = ChainLock::acquire(&pin, &tasks).unwrap();
    let child = fork_child_paused_before_exec();

    let refused = ChainLock::acquire(&pin, &tasks).err().expect("held");
    assert!(refused.to_string().contains("holds"), "{refused:#}");

    drop(lock);
    ChainLock::acquire(&pin, &tasks)
        .expect("a released chain lock is free although a forked child still shares its file");
    child.join().unwrap();
}
