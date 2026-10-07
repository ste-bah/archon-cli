//! Captured host admission, carried through spawned agent/tool contexts.
//!
//! The one fence mechanism for a run's executor (Issues 291, 300). Before each
//! poll of fenced work a short check asks whether the captured owner may go
//! on. The check is the only critical section: the work is then polled with
//! no lock held, so a synchronous verifier, a git snapshot or a long provider
//! stream inside the work never makes `pause`, `cancel` or `resume` wait.
//! Durable writes fence themselves at the write.
//!
//! A refusal never turns into an ordinary failure:
//! - `Refused` (no control decision, e.g. a store bound to another run) is
//!   an ordinary error of the fenced work. The run store's fence never
//!   refuses for an unreadable state: it pauses the run instead (a stall
//!   pauses, never fails).
//! - A pause or cancel of a still-owned run lets work that was already
//!   admitted finish in one last poll; a finished result is kept as evidence.
//!   Nothing new is admitted: nested fences see the same stop.
//! - When an enclosing fence of the same owner on this poll stack reacts to
//!   the stop, the inner fence parks (Pending) instead of returning a text the
//!   agent could read as a tool failure. The enclosing fence then stops the
//!   whole tree with its own typed control error.
use std::{cell::RefCell, future::Future, sync::Arc, task::Poll, time::Duration, time::Instant};

/// How often pending fenced work is woken to look at run control. It only
/// wakes work; it is never a deadline.
const WATCH: Duration = Duration::from_secs(2);
/// A nested fence of the same owner reuses an enclosing check made this
/// recently on the same poll stack, instead of reading the state again.
const NESTED_REUSE: Duration = Duration::from_millis(50);

/// What a refused check means for the work it fences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopKind {
    /// The owner's run is paused. Admitted work may finish; nothing new starts.
    Paused,
    /// The owner's run is cancelled. Same rule as a pause.
    Cancelled,
    /// A newer executor owns the run, or the run is gone. Nothing of this
    /// owner is polled again.
    Superseded,
    /// No run-control decision (e.g. a fence misuse): an ordinary error.
    Refused,
}

impl StopKind {
    /// Whether this is run control (a pause, cancel or new owner).
    pub fn is_control(self) -> bool {
        self != Self::Refused
    }
}

/// A refusal that names its run-control meaning.
pub trait FenceStop {
    fn stop_kind(&self) -> StopKind;
}

/// What a fence checks. An admission fence also stops for a pause or cancel;
/// an ownership fence only for a new owner (it lets a paused owner unwind).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FenceKind {
    Ownership,
    Admission,
}

impl FenceKind {
    fn reacts_to(self, stop: StopKind) -> bool {
        match stop {
            StopKind::Superseded => true,
            StopKind::Paused | StopKind::Cancelled => self == Self::Admission,
            StopKind::Refused => false,
        }
    }
}

struct Frame {
    owner: Arc<str>,
    kind: FenceKind,
    /// When this fence's check passed; `None` while it drains after a stop.
    passed_at: Option<Instant>,
}

thread_local! {
    // Fences currently polling their work on this thread. A poll is
    // synchronous, so every entry belongs to the same task's poll stack.
    static ENCLOSING: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

struct Pop;
impl Drop for Pop {
    fn drop(&mut self) {
        ENCLOSING.with(|frames| {
            frames.borrow_mut().pop();
        });
    }
}

fn enclosing_reacts(owner: &str, stop: StopKind) -> bool {
    ENCLOSING.with(|frames| {
        frames
            .borrow()
            .iter()
            .any(|frame| &*frame.owner == owner && frame.kind.reacts_to(stop))
    })
}

/// When an enclosing fence of the same owner, at least as strict, last
/// passed its check, if that was within [`NESTED_REUSE`]. A reuse keeps the
/// original check time, so nesting never extends a check's age.
fn enclosing_checked(owner: &str, kind: FenceKind) -> Option<Instant> {
    ENCLOSING.with(|frames| {
        frames
            .borrow()
            .iter()
            .filter(|frame| &*frame.owner == owner && frame.kind >= kind)
            .filter_map(|frame| frame.passed_at)
            .filter(|at| at.elapsed() < NESTED_REUSE)
            .max()
    })
}

fn poll_enclosed<F: Future + ?Sized>(
    owner: &Arc<str>,
    kind: FenceKind,
    passed_at: Option<Instant>,
    work: std::pin::Pin<&mut F>,
    cx: &mut std::task::Context<'_>,
) -> Poll<F::Output> {
    ENCLOSING.with(|frames| {
        frames.borrow_mut().push(Frame {
            owner: owner.clone(),
            kind,
            passed_at,
        })
    });
    let _pop = Pop;
    work.poll(cx)
}

/// Poll `work` under the fence rule above. `owner` names the captured owner
/// (run and executor); `check` is the short critical section.
pub async fn drive_fenced<T, E: FenceStop>(
    owner: Arc<str>,
    kind: FenceKind,
    check: impl Fn() -> Result<(), E>,
    work: impl Future<Output = T>,
) -> Result<T, E> {
    let mut work = Box::pin(work);
    // The first look is the check below; the watch's first tick is one
    // period out, so it never wakes the work spuriously at once.
    let mut watch = tokio::time::interval_at(tokio::time::Instant::now() + WATCH, WATCH);
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let (mut admitted, mut parked) = (false, false);
    std::future::poll_fn(|cx| {
        // Register the next wake even when the immediate tick is ready.
        while watch.poll_tick(cx).is_ready() {}
        let (checked_at, refused) = match enclosing_checked(&owner, kind) {
            Some(at) => (at, None),
            None => (Instant::now(), check().err()),
        };
        let Some(refused) = refused else {
            admitted = true;
            return poll_enclosed(&owner, kind, Some(checked_at), work.as_mut(), cx).map(Ok);
        };
        let stop = refused.stop_kind();
        if !stop.is_control() {
            return Poll::Ready(Err(refused));
        }
        // Admitted work of a still-owned run finishes if it can, in one poll
        // in which every nested admission sees this same stop.
        if admitted
            && matches!(stop, StopKind::Paused | StopKind::Cancelled)
            && let Poll::Ready(value) = poll_enclosed(&owner, kind, None, work.as_mut(), cx)
        {
            return Poll::Ready(Ok(value));
        }
        if enclosing_reacts(&owner, stop) {
            // The enclosing fence ends the tree with a typed control error.
            // Wake it once at once; its own watch wakes it after that.
            if !parked {
                parked = true;
                cx.waker().wake_by_ref();
            }
            return Poll::Pending;
        }
        Poll::Ready(Err(refused))
    })
    .await
}

/// A typed refusal carried through spawned agent and tool contexts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionStop {
    pub kind: StopKind,
    pub reason: String,
}

impl FenceStop for AdmissionStop {
    fn stop_kind(&self) -> StopKind {
        self.kind
    }
}

impl std::fmt::Display for AdmissionStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            StopKind::Paused => write!(f, "workflow run control: paused; {}", self.reason),
            StopKind::Cancelled => write!(f, "workflow run control: cancelled; {}", self.reason),
            StopKind::Superseded => {
                write!(f, "workflow run control: superseded; {}", self.reason)
            }
            StopKind::Refused => f.write_str(&self.reason),
        }
    }
}

type Check = dyn Fn() -> Result<(), AdmissionStop> + Send + Sync;
#[derive(Clone)]
pub struct AdmissionFence {
    owner: Arc<str>,
    check: Arc<Check>,
}
impl std::fmt::Debug for AdmissionFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmissionFence(..)")
    }
}
impl AdmissionFence {
    /// `owner` names the host's captured owner; `check` is its short check.
    pub fn new(
        owner: impl Into<Arc<str>>,
        check: impl Fn() -> Result<(), AdmissionStop> + Send + Sync + 'static,
    ) -> Self {
        Self {
            owner: owner.into(),
            check: Arc::new(check),
        }
    }
    pub async fn execute<T>(&self, work: impl Future<Output = T>) -> Result<T, AdmissionStop> {
        // Boxed at once (#246): the driver holds a pointer, not a copy.
        let work = Box::pin(work);
        drive_fenced(
            self.owner.clone(),
            FenceKind::Admission,
            || (self.check)(),
            work,
        )
        .await
    }
}

#[cfg(test)]
#[path = "workflow_admission_tests.rs"]
mod tests;
