//! Bounded TUI event channel with priority shedding.
//!
//! The TUI render loop must never be fed by an unbounded producer-facing
//! queue. If rendering stalls on a huge buffer, background producers can keep
//! sending progress events and inflate RSS without limit. This channel keeps
//! the synchronous `send()` API needed by slash-command handlers while putting
//! a hard cap on queued `TuiEvent`s.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::sync::Notify;
use tokio::sync::mpsc::error::SendError;

use crate::events::TuiEvent;

pub const TUI_EVENT_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventPriority {
    State,
    Progress,
}

#[derive(Debug)]
struct Inner {
    queue: std::sync::Mutex<VecDeque<TuiEvent>>,
    notify: Notify,
    closed: AtomicBool,
    sender_count: AtomicUsize,
    capacity: usize,
    dropped_progress: AtomicUsize,
    dropped_state: AtomicUsize,
}

/// Producer side of the bounded TUI event channel.
#[derive(Debug)]
pub struct TuiEventSender {
    inner: Arc<Inner>,
}

impl Clone for TuiEventSender {
    fn clone(&self) -> Self {
        self.inner.sender_count.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for TuiEventSender {
    fn drop(&mut self) {
        if self.inner.sender_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.notify.notify_waiters();
        }
    }
}

impl TuiEventSender {
    /// Synchronously enqueue an event.
    ///
    /// The queue is hard-bounded. When full, progress events are shed before
    /// state events. If the queue is entirely state events, the oldest event is
    /// dropped as a last-resort RSS safety valve.
    pub fn send(&self, event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(SendError(event));
        }

        {
            let mut queue = self.inner.queue.lock().expect("tui event queue lock");
            if queue.len() >= self.inner.capacity {
                if priority(&event) == EventPriority::Progress {
                    self.inner.dropped_progress.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
                if drop_oldest_progress(&mut queue) {
                    self.inner.dropped_progress.fetch_add(1, Ordering::Relaxed);
                } else {
                    queue.pop_front();
                    self.inner.dropped_state.fetch_add(1, Ordering::Relaxed);
                }
            }
            queue.push_back(event);
        }

        self.inner.notify.notify_one();
        Ok(())
    }

    pub fn dropped_progress(&self) -> usize {
        self.inner.dropped_progress.load(Ordering::Relaxed)
    }

    pub fn dropped_state(&self) -> usize {
        self.inner.dropped_state.load(Ordering::Relaxed)
    }
}

/// Consumer side of the bounded TUI event channel.
#[derive(Debug)]
pub struct TuiEventReceiver {
    inner: Arc<Inner>,
}

impl Drop for TuiEventReceiver {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }
}

impl TuiEventReceiver {
    pub async fn recv(&mut self) -> Option<TuiEvent> {
        loop {
            if let Ok(event) = self.try_recv() {
                return Some(event);
            }
            if self.inner.sender_count.load(Ordering::Acquire) == 0 {
                return None;
            }
            self.inner.notify.notified().await;
        }
    }

    pub fn try_recv(&mut self) -> Result<TuiEvent, tokio::sync::mpsc::error::TryRecvError> {
        let mut queue = self.inner.queue.lock().expect("tui event queue lock");
        if let Some(event) = queue.pop_front() {
            return Ok(event);
        }
        if self.inner.sender_count.load(Ordering::Acquire) == 0 {
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        } else {
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        }
    }

    pub fn len(&self) -> usize {
        self.inner.queue.lock().expect("tui event queue lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn bounded_tui_event_channel() -> (TuiEventSender, TuiEventReceiver) {
    bounded_tui_event_channel_with_capacity(TUI_EVENT_CHANNEL_CAPACITY)
}

pub fn bounded_tui_event_channel_with_capacity(
    capacity: usize,
) -> (TuiEventSender, TuiEventReceiver) {
    let capacity = capacity.max(1);
    let inner = Arc::new(Inner {
        queue: std::sync::Mutex::new(VecDeque::with_capacity(capacity)),
        notify: Notify::new(),
        closed: AtomicBool::new(false),
        sender_count: AtomicUsize::new(1),
        capacity,
        dropped_progress: AtomicUsize::new(0),
        dropped_state: AtomicUsize::new(0),
    });
    (
        TuiEventSender {
            inner: Arc::clone(&inner),
        },
        TuiEventReceiver { inner },
    )
}

fn priority(event: &TuiEvent) -> EventPriority {
    match event {
        TuiEvent::TextDelta(_) | TuiEvent::ThinkingDelta(_) => EventPriority::Progress,
        _ => EventPriority::State,
    }
}

fn drop_oldest_progress(queue: &mut VecDeque<TuiEvent>) -> bool {
    if let Some(idx) = queue
        .iter()
        .position(|event| priority(event) == EventPriority::Progress)
    {
        queue.remove(idx);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_channel_drops_new_progress_when_full() {
        let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
        tx.send(TuiEvent::TextDelta("a".into())).unwrap();
        tx.send(TuiEvent::TextDelta("b".into())).unwrap();
        tx.send(TuiEvent::TextDelta("c".into())).unwrap();

        assert_eq!(tx.dropped_progress(), 1);
        assert!(matches!(rx.recv().await, Some(TuiEvent::TextDelta(s)) if s == "a"));
        assert!(matches!(rx.recv().await, Some(TuiEvent::TextDelta(s)) if s == "b"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn bounded_channel_preserves_state_by_shedding_progress() {
        let (tx, mut rx) = bounded_tui_event_channel_with_capacity(2);
        tx.send(TuiEvent::TextDelta("progress".into())).unwrap();
        tx.send(TuiEvent::ThinkingDelta("thinking".into())).unwrap();
        tx.send(TuiEvent::Done).unwrap();

        assert_eq!(tx.dropped_progress(), 1);
        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::ThinkingDelta(s)) if s == "thinking"
        ));
        assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
    }

    #[tokio::test]
    async fn bounded_channel_has_hard_state_cap() {
        let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
        tx.send(TuiEvent::GenerationStarted).unwrap();
        tx.send(TuiEvent::Done).unwrap();

        assert_eq!(tx.dropped_state(), 1);
        assert!(matches!(rx.recv().await, Some(TuiEvent::Done)));
    }

    #[tokio::test]
    async fn resize_is_preserved_as_state_event() {
        let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
        tx.send(TuiEvent::TextDelta("progress".into())).unwrap();
        tx.send(TuiEvent::Resize {
            cols: 120,
            rows: 40,
        })
        .unwrap();

        assert_eq!(tx.dropped_progress(), 1);
        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::Resize {
                cols: 120,
                rows: 40
            })
        ));
    }
}
