//! Lossless TUI event channel with content coalescing.
//!
//! Assistant text, thinking, payload-bearing progress, and state transitions
//! are lossless. Adjacent text and thinking deltas are merged to limit event
//! count without changing byte order or crossing event boundaries.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::sync::Notify;
use tokio::sync::mpsc::error::SendError;

use crate::events::TuiEvent;

pub const TUI_EVENT_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug)]
struct Inner {
    queue: std::sync::Mutex<VecDeque<TuiEvent>>,
    notify: Notify,
    closed: AtomicBool,
    sender_count: AtomicUsize,
    dropped_progress: AtomicUsize,
    dropped_content: AtomicUsize,
    dropped_state: AtomicUsize,
    #[cfg(test)]
    pause_before_send_lock: AtomicBool,
    #[cfg(test)]
    send_reached_lock: AtomicBool,
    #[cfg(test)]
    pause_before_recv_wait: AtomicBool,
    #[cfg(test)]
    recv_reached_wait: AtomicBool,
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
    /// Payload-bearing events are lossless and may temporarily exceed the
    /// event-count target. Adjacent text and thinking deltas are coalesced to
    /// limit that growth without losing bytes.
    // The failed event is returned to the caller (mpsc convention), so the
    // error size is inherent to the contract.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(SendError(event));
        }

        #[cfg(test)]
        {
            self.inner.send_reached_lock.store(true, Ordering::Release);
            while self.inner.pause_before_send_lock.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }

        {
            let mut queue = self.inner.queue.lock().expect("tui event queue lock");
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(SendError(event));
            }
            let Some(event) = enqueue_or_coalesce_content_delta(&mut queue, event) else {
                return Ok(());
            };
            queue.push_back(event);
            crate::observability::record_tui_event_enqueued();
        }

        self.inner.notify.notify_one();
        Ok(())
    }

    pub fn dropped_progress(&self) -> usize {
        self.inner.dropped_progress.load(Ordering::Relaxed)
    }

    pub fn dropped_content(&self) -> usize {
        self.inner.dropped_content.load(Ordering::Relaxed)
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
        let dropped = {
            let mut queue = self.inner.queue.lock().expect("tui event queue lock");
            self.inner.closed.store(true, Ordering::Release);
            let dropped = queue.len();
            queue.clear();
            dropped
        };
        for _ in 0..dropped {
            crate::observability::record_tui_event_discarded();
        }
        self.inner.notify.notify_waiters();
    }
}

impl TuiEventReceiver {
    pub async fn recv(&mut self) -> Option<TuiEvent> {
        loop {
            let inner = Arc::clone(&self.inner);
            let notified = inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Ok(event) = self.try_recv() {
                return Some(event);
            }
            if self.inner.sender_count.load(Ordering::Acquire) == 0 {
                return None;
            }
            #[cfg(test)]
            {
                self.inner.recv_reached_wait.store(true, Ordering::Release);
                while self.inner.pause_before_recv_wait.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }
            notified.await;
        }
    }

    pub fn try_recv(&mut self) -> Result<TuiEvent, tokio::sync::mpsc::error::TryRecvError> {
        let mut queue = self.inner.queue.lock().expect("tui event queue lock");
        if let Some(event) = queue.pop_front() {
            crate::observability::record_tui_event_dequeued();
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
        dropped_progress: AtomicUsize::new(0),
        dropped_content: AtomicUsize::new(0),
        dropped_state: AtomicUsize::new(0),
        #[cfg(test)]
        pause_before_send_lock: AtomicBool::new(false),
        #[cfg(test)]
        send_reached_lock: AtomicBool::new(false),
        #[cfg(test)]
        pause_before_recv_wait: AtomicBool::new(false),
        #[cfg(test)]
        recv_reached_wait: AtomicBool::new(false),
    });
    (
        TuiEventSender {
            inner: Arc::clone(&inner),
        },
        TuiEventReceiver { inner },
    )
}

fn enqueue_or_coalesce_content_delta(
    queue: &mut VecDeque<TuiEvent>,
    event: TuiEvent,
) -> Option<TuiEvent> {
    match event {
        TuiEvent::TextDelta(text) => {
            if let Some(TuiEvent::TextDelta(previous)) = queue.back_mut() {
                previous.push_str(&text);
                None
            } else {
                Some(TuiEvent::TextDelta(text))
            }
        }
        TuiEvent::ThinkingDelta(text) => {
            if let Some(TuiEvent::ThinkingDelta(previous)) = queue.back_mut() {
                previous.push_str(&text);
                None
            } else {
                Some(TuiEvent::ThinkingDelta(text))
            }
        }
        event => Some(event),
    }
}

#[cfg(test)]
#[path = "event_channel_tests.rs"]
mod tests;
