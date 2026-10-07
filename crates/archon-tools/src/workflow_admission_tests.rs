//! The fence rule: check, then poll with no lock; drain admitted work on a
//! pause; park under an enclosing fence that will stop the tree.
use super::*;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const SUPERSEDED: u8 = 2;
const UNREADABLE: u8 = 3;

fn fence(state: &Arc<AtomicU8>, checks: &Arc<AtomicUsize>) -> AdmissionFence {
    let (state, checks) = (state.clone(), checks.clone());
    AdmissionFence::new("run#1", move || {
        checks.fetch_add(1, Ordering::SeqCst);
        let kind = match state.load(Ordering::SeqCst) {
            RUNNING => return Ok(()),
            PAUSED => StopKind::Paused,
            SUPERSEDED => StopKind::Superseded,
            _ => StopKind::Refused,
        };
        Err(AdmissionStop {
            kind,
            reason: "test".into(),
        })
    })
}

fn bounded<T>(work: impl Future<Output = T>) -> impl Future<Output = T> {
    async move {
        tokio::time::timeout(Duration::from_secs(30), work)
            .await
            .expect("a fence regression must fail, not hang")
    }
}

/// Spawn fenced work that reports when it was admitted, then waits on `rx`.
fn admitted<T: Send + 'static>(
    fence: AdmissionFence,
    rx: tokio::sync::oneshot::Receiver<T>,
) -> (
    tokio::task::JoinHandle<Result<T, AdmissionStop>>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (entered_tx, entered) = tokio::sync::oneshot::channel();
    let work = tokio::spawn(async move {
        fence
            .execute(async move {
                entered_tx.send(()).unwrap();
                rx.await.unwrap()
            })
            .await
    });
    (work, entered)
}

#[tokio::test]
async fn pause_before_admission_never_polls_the_work() {
    let (state, checks) = (Arc::new(AtomicU8::new(PAUSED)), Arc::default());
    let polled = AtomicUsize::new(0);
    let result = bounded(fence(&state, &checks).execute(async {
        polled.fetch_add(1, Ordering::SeqCst);
    }))
    .await;
    assert_eq!(result.unwrap_err().kind, StopKind::Paused);
    assert_eq!(polled.load(Ordering::SeqCst), 0, "nothing new is admitted");
}

#[tokio::test]
async fn admitted_work_that_finishes_as_the_pause_lands_is_kept() {
    let (state, checks) = (Arc::new(AtomicU8::new(RUNNING)), Arc::default());
    let (tx, rx) = tokio::sync::oneshot::channel::<&str>();
    let (work, entered) = admitted(fence(&state, &checks), rx);
    bounded(entered).await.unwrap();
    state.store(PAUSED, Ordering::SeqCst);
    tx.send("finished").unwrap();
    assert_eq!(bounded(work).await.unwrap(), Ok("finished"));
}

#[tokio::test]
async fn pending_work_under_a_pause_stops_typed_within_the_watch() {
    let (state, checks) = (Arc::new(AtomicU8::new(RUNNING)), Arc::default());
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (work, entered) = admitted(fence(&state, &checks), rx);
    bounded(entered).await.unwrap();
    let started = Instant::now();
    state.store(PAUSED, Ordering::SeqCst);
    let result = bounded(work).await.unwrap();
    assert_eq!(result.unwrap_err().kind, StopKind::Paused);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn a_new_owner_drops_even_finished_work() {
    let (state, checks) = (Arc::new(AtomicU8::new(RUNNING)), Arc::default());
    let (tx, rx) = tokio::sync::oneshot::channel::<&str>();
    let (work, entered) = admitted(fence(&state, &checks), rx);
    bounded(entered).await.unwrap();
    state.store(SUPERSEDED, Ordering::SeqCst);
    tx.send("stale").unwrap();
    assert_eq!(
        bounded(work).await.unwrap().unwrap_err().kind,
        StopKind::Superseded
    );
}

#[tokio::test]
async fn an_unreadable_state_is_an_ordinary_refusal() {
    let (state, checks) = (Arc::new(AtomicU8::new(UNREADABLE)), Arc::default());
    let result = bounded(fence(&state, &checks).execute(async {})).await;
    let refused = result.unwrap_err();
    assert_eq!(refused.kind, StopKind::Refused);
    assert_eq!(refused.to_string(), "test", "no control wording");
}

#[tokio::test]
async fn an_inner_refusal_parks_and_the_outer_fence_stops_the_tree() {
    let (state, checks) = (Arc::new(AtomicU8::new(RUNNING)), Arc::default());
    let (outer, inner) = (fence(&state, &checks), fence(&state, &checks));
    let saw_inner_error = Arc::new(AtomicUsize::new(0));
    let seen = saw_inner_error.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (entered_tx, entered) = tokio::sync::oneshot::channel::<()>();
    let work = tokio::spawn(async move {
        outer
            .execute(async move {
                entered_tx.send(()).unwrap();
                rx.await.unwrap();
                // The pause landed meanwhile: a new nested admission.
                if inner.execute(async {}).await.is_err() {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
                "agent read a tool failure"
            })
            .await
    });
    bounded(entered).await.unwrap();
    state.store(PAUSED, Ordering::SeqCst);
    tx.send(()).unwrap();
    assert_eq!(
        bounded(work).await.unwrap().unwrap_err().kind,
        StopKind::Paused
    );
    assert_eq!(
        saw_inner_error.load(Ordering::SeqCst),
        0,
        "the stop never reaches the agent as an ordinary tool error"
    );
}

#[tokio::test]
async fn nested_fences_reuse_a_fresh_enclosing_check() {
    let (state, checks) = (
        Arc::new(AtomicU8::new(RUNNING)),
        Arc::new(AtomicUsize::new(0)),
    );
    let (outer, inner) = (fence(&state, &checks), fence(&state, &checks));
    let value = bounded(outer.execute(async move { inner.execute(async { 7 }).await }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value, 7);
    assert_eq!(
        checks.load(Ordering::SeqCst),
        1,
        "one state read per poll stack"
    );
}

#[tokio::test]
async fn an_ownership_fence_does_not_park_a_paused_admission() {
    let (state, checks) = (Arc::new(AtomicU8::new(PAUSED)), Arc::default());
    let inner = fence(&state, &checks);
    // An ownership fence lets a paused owner unwind, so it would never stop
    // the tree: the inner admission must answer itself instead of parking.
    let result = bounded(drive_fenced(
        Arc::from("run#1"),
        FenceKind::Ownership,
        || Ok::<(), AdmissionStop>(()),
        async move { inner.execute(async {}).await },
    ))
    .await
    .unwrap();
    assert_eq!(result.unwrap_err().kind, StopKind::Paused);
}

#[tokio::test]
async fn a_long_synchronous_poll_holds_no_lock_and_rechecks_inner_admission() {
    // The check is the only critical section: a poll that blocks for a while
    // (a verifier child, a git snapshot) does not delay the stop, and an
    // admission after it reads the state again.
    let (state, checks) = (
        Arc::new(AtomicU8::new(RUNNING)),
        Arc::new(AtomicUsize::new(0)),
    );
    let (outer, inner) = (fence(&state, &checks), fence(&state, &checks));
    let pauser = state.clone();
    let result = bounded(outer.execute(async move {
        let blocking = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            pauser.store(PAUSED, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(200));
        blocking.join().unwrap();
        inner.execute(async { "admitted after the pause" }).await
    }))
    .await;
    assert_eq!(result.unwrap_err().kind, StopKind::Paused);
    assert!(
        checks.load(Ordering::SeqCst) >= 2,
        "the inner admission checked"
    );
}

#[tokio::test]
async fn pending_running_work_is_not_busy_polled() {
    let (state, checks) = (
        Arc::new(AtomicU8::new(RUNNING)),
        Arc::new(AtomicUsize::new(0)),
    );
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (work, entered) = admitted(fence(&state, &checks), rx);
    bounded(entered).await.unwrap();
    tokio::time::sleep(Duration::from_millis(4500)).await;
    let polls = checks.load(Ordering::SeqCst);
    assert!(
        polls <= 5,
        "the watch wakes every 2 s, not continuously: {polls}"
    );
    work.abort();
}
