use std::cell::RefCell;
use std::future::{Future, pending};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<T>(future: impl Future<Output = T>) -> Poll<T> {
    let waker = Waker::from(Arc::new(NoopWake));
    std::pin::pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
}

#[test]
fn handled_modes_never_set_up_voice() {
    for mode in [
        "subcommand",
        "headless",
        "print",
        "catalog",
        "resume-list",
        "sessions",
        "background",
    ] {
        let events = RefCell::new(Vec::new());
        let outcome = poll_once(super::run(
            async {
                events.borrow_mut().push(mode);
                Ok::<Option<()>, &str>(None)
            },
            || async { events.borrow_mut().push("voice") },
            |(), ()| async { panic!("handled mode reached interactive session") },
        ));
        assert_eq!(outcome, Poll::Ready(Ok(())));
        assert_eq!(*events.borrow(), [mode], "voice started for {mode}");
    }
}

#[test]
fn dispatch_errors_including_tty_rejection_never_set_up_voice() {
    for error in [
        "subcommand error",
        "invalid resume",
        "stdin is not a TTY",
        "stdout is not a TTY",
    ] {
        let calls = RefCell::new(0);
        let outcome = poll_once(super::run(
            async { Err::<Option<()>, _>(error) },
            || async { *calls.borrow_mut() += 1 },
            |(), ()| async { panic!("failed dispatch reached interactive session") },
        ));
        assert_eq!(outcome, Poll::Ready(Err(error)));
        assert_eq!(*calls.borrow(), 0, "voice started before {error}");
    }
}

#[test]
fn interactive_receives_voice_only_after_dispatch_and_tty_validation() {
    let events = RefCell::new(Vec::new());
    let outcome = poll_once(super::run(
        async {
            events.borrow_mut().push("dispatch and TTY validation");
            Ok::<_, &str>(Some("session"))
        },
        || async {
            events.borrow_mut().push("voice");
            Some("receiver")
        },
        |session, voice| {
            let events = &events;
            async move {
                assert_eq!(session, "session");
                assert_eq!(voice, Some("receiver"));
                events.borrow_mut().push("interactive");
                Ok(())
            }
        },
    ));
    assert_eq!(outcome, Poll::Ready(Ok(())));
    assert_eq!(
        *events.borrow(),
        ["dispatch and TTY validation", "voice", "interactive"]
    );
}

#[test]
fn interactive_without_voice_preserves_session_errors_and_order() {
    let events = RefCell::new(Vec::new());
    let outcome = poll_once(super::run(
        async {
            events.borrow_mut().push("dispatch");
            Ok::<_, &str>(Some(()))
        },
        || async {
            events.borrow_mut().push("voice disabled or unavailable");
            None::<()>
        },
        |(), voice| async move {
            assert_eq!(voice, None);
            Err("session error")
        },
    ));
    assert_eq!(outcome, Poll::Ready(Err("session error")));
    assert_eq!(
        *events.borrow(),
        ["dispatch", "voice disabled or unavailable"]
    );
}

#[test]
fn pending_dispatch_never_starts_voice() {
    let calls = RefCell::new(0);
    let outcome = poll_once(super::run(
        pending::<Result<Option<()>, &str>>(),
        || async { *calls.borrow_mut() += 1 },
        |(), ()| async { panic!("pending dispatch reached interactive session") },
    ));
    assert_eq!(outcome, Poll::Pending);
    assert_eq!(*calls.borrow(), 0);
}
