//! The guard's test suites, split by what they exercise.
//!
//! [`lock_path`] derives the sidecar path, [`retry_policy`] states the sleep
//! budgets and the busy-signal classification, [`retry_loop`] drives the loop
//! those produce, and [`write_lock`] takes the actual OS lock.

mod lock_path;
mod retry_loop;
mod retry_policy;
mod write_lock;

use super::*;
