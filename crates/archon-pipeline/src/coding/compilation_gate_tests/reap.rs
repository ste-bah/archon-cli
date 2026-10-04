//! Issue 242 (round 2): the direct child's reap after a timeout is bounded.
//! A child whose termination never completes (pending I/O on Windows) must
//! not hold the gate, and so the job confirmation behind it, forever.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::process::ExitStatus;

use process_wrap::tokio::ChildWrapper;

use super::super::{ChildReap, CleanupOutcome, cleanup_child};
use super::*;

/// A child whose termination is accepted but never completes.
#[derive(Debug)]
struct NeverReaped(tokio::process::Child);

impl ChildWrapper for NeverReaped {
    fn inner(&self) -> &dyn ChildWrapper {
        &self.0
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        &mut self.0
    }
    fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
        Box::new(self.0)
    }
    fn start_kill(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(None)
    }
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_reap_of_a_child_whose_termination_never_completes_is_bounded() {
    let real = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut child: Box<dyn ChildWrapper> = Box::new(NeverReaped(real));
    let outcome = tokio::time::timeout(Duration::from_secs(600), cleanup_child(&mut child))
        .await
        .expect("cleanup must end within its own bound");
    assert_eq!(
        outcome,
        CleanupOutcome::TerminationRequestAccepted {
            reap: ChildReap::Failed
        }
    );
}
