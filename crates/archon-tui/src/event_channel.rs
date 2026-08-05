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

use crate::event_framing::ContentFrames;
use crate::events::TuiEvent;

pub const TUI_EVENT_CHANNEL_CAPACITY: usize = 1024;
pub const MAX_COALESCED_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct Inner {
    queue: std::sync::Mutex<VecDeque<TuiEvent>>,
    capacity: usize,
    notify: Notify,
    not_full: Notify,
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

impl Inner {
    /// Events queued in THIS channel, read from the queue itself.
    ///
    /// The process-wide `TUI_EVENT_PENDING` gauge answers the same question for
    /// the render loop, which has exactly one channel -- but a test process has
    /// many, concurrently, all moving the same counter. Tests that asserted an
    /// exact depth through the global were therefore asserting on every other
    /// test's traffic too, and failed whenever anything unrelated shifted the
    /// schedule (measured: 2 failures in 5 runs after three new tests were
    /// added elsewhere in the crate). Reading the queue is exact, needs no
    /// counter, and cannot be perturbed by another channel.
    fn queued_len(&self) -> usize {
        self.queue.lock().expect("tui event queue lock").len()
    }

    fn queued_bytes(&self) -> usize {
        self.queue
            .lock()
            .expect("tui event queue lock")
            .iter()
            .map(crate::event_payload_size::heap_bytes)
            .sum()
    }
}

/// Producer side of the bounded TUI event channel.
#[derive(Debug)]
pub struct TuiEventSender {
    inner: Arc<Inner>,
}

impl TuiEventSender {
    /// Events queued in this channel. See [`Inner::queued_len`].
    pub fn queued_len(&self) -> usize {
        self.inner.queued_len()
    }

    /// Heap bytes retained by events queued in this channel.
    pub fn queued_bytes(&self) -> usize {
        self.inner.queued_bytes()
    }
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
            self.inner.not_full.notify_waiters();
        }
    }
}

impl TuiEventSender {
    /// Synchronously enqueue one logical event.
    ///
    /// Oversized content is framed before admission. The whole framed event is
    /// rejected when its frames cannot fit without exceeding channel capacity;
    /// async producers should use [`Self::send_async`] to wait for capacity.
    // The failed event is returned to the caller (mpsc convention), so the
    // error size is inherent to the contract.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        let mut frames = ContentFrames::new(event.clone());
        if frames.frame_count() == 1 {
            let frame = frames.next().expect("single frame");
            if frame_is_oversized(&frame) {
                crate::observability::record_tui_event_oversized_rejected();
                return Err(SendError(event));
            }
            return self.try_send_frame(frame);
        }

        let mut queue = self.inner.queue.lock().expect("tui event queue lock");
        if self.inner.closed.load(Ordering::Acquire) {
            crate::observability::record_tui_event_closed_send_failure();
            return Err(SendError(event));
        }
        if queue.len().saturating_add(frames.frame_count()) > self.inner.capacity {
            crate::observability::record_tui_event_full_send_failure();
            return Err(SendError(event));
        }
        if frames.any(|frame| frame_is_oversized(&frame)) {
            crate::observability::record_tui_event_oversized_rejected();
            return Err(SendError(event));
        }
        for frame in ContentFrames::new(event.clone()) {
            let Some(frame) = coalesce_with_metrics(&mut queue, frame) else {
                continue;
            };
            let bytes = crate::event_payload_size::heap_bytes(&frame);
            queue.push_back(frame);
            crate::observability::record_tui_event_enqueued(bytes);
        }
        drop(queue);
        self.inner.notify.notify_waiters();
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn try_send_frame(&self, event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        if self.inner.closed.load(Ordering::Acquire) {
            crate::observability::record_tui_event_closed_send_failure();
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
                crate::observability::record_tui_event_closed_send_failure();
                return Err(SendError(event));
            }
            let Some(event) = coalesce_with_metrics(&mut queue, event) else {
                return Ok(());
            };
            if queue.len() >= self.inner.capacity {
                crate::observability::record_tui_event_full_send_failure();
                return Err(SendError(event));
            }
            let bytes = crate::event_payload_size::heap_bytes(&event);
            queue.push_back(event);
            crate::observability::record_tui_event_enqueued(bytes);
        }

        self.inner.notify.notify_one();
        Ok(())
    }

    pub async fn send_async(&self, event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        for frame in ContentFrames::new(event) {
            if frame_is_oversized(&frame) {
                crate::observability::record_tui_event_oversized_rejected();
                return Err(SendError(frame));
            }
            self.send_frame_async(frame).await?;
        }
        Ok(())
    }

    pub async fn send_atomic_async(&self, event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        let frames = ContentFrames::new(event).collect::<Vec<_>>();
        if let Some(frame) = frames.iter().find(|frame| frame_is_oversized(frame)) {
            crate::observability::record_tui_event_oversized_rejected();
            return Err(SendError(frame.clone()));
        }
        if frames.len() == 1 {
            return self
                .send_frame_async(frames.into_iter().next().expect("single frame"))
                .await;
        }
        self.send_frames_async(frames).await
    }

    async fn send_frames_async(&self, frames: Vec<TuiEvent>) -> Result<(), SendError<TuiEvent>> {
        let rejected_event = || frames.first().cloned().expect("multi-frame event");
        if frames.len() > self.inner.capacity {
            crate::observability::record_tui_event_full_send_failure();
            return Err(SendError(rejected_event()));
        }
        loop {
            let notified = self.inner.not_full.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut queue = self.inner.queue.lock().expect("tui event queue lock");
                if self.inner.closed.load(Ordering::Acquire) {
                    crate::observability::record_tui_event_closed_send_failure();
                    return Err(SendError(rejected_event()));
                }
                if queue.len().saturating_add(frames.len()) <= self.inner.capacity {
                    for frame in &frames {
                        let Some(frame) = coalesce_with_metrics(&mut queue, frame.clone()) else {
                            continue;
                        };
                        let bytes = crate::event_payload_size::heap_bytes(&frame);
                        queue.push_back(frame);
                        crate::observability::record_tui_event_enqueued(bytes);
                    }
                    drop(queue);
                    self.inner.notify.notify_waiters();
                    return Ok(());
                }
            }
            let blocked_at = std::time::Instant::now();
            notified.await;
            crate::observability::record_tui_event_blocked_send(blocked_at.elapsed());
        }
    }

    async fn send_frame_async(&self, mut event: TuiEvent) -> Result<(), SendError<TuiEvent>> {
        loop {
            let notified = self.inner.not_full.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.try_send_frame(event) {
                Ok(()) => return Ok(()),
                Err(SendError(returned)) if !self.inner.closed.load(Ordering::Acquire) => {
                    event = returned;
                    let blocked_at = std::time::Instant::now();
                    notified.await;
                    crate::observability::record_tui_event_blocked_send(blocked_at.elapsed());
                }
                Err(error) => return Err(error),
            }
        }
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

impl TuiEventReceiver {
    /// Events queued in this channel. See [`Inner::queued_len`].
    pub fn queued_len(&self) -> usize {
        self.inner.queued_len()
    }
}

impl Drop for TuiEventReceiver {
    fn drop(&mut self) {
        let dropped = {
            let mut queue = self.inner.queue.lock().expect("tui event queue lock");
            self.inner.closed.store(true, Ordering::Release);
            queue
                .drain(..)
                .map(|event| crate::event_payload_size::heap_bytes(&event))
                .collect::<Vec<_>>()
        };
        for bytes in dropped {
            crate::observability::record_tui_event_discarded(bytes);
        }
        self.inner.notify.notify_waiters();
        self.inner.not_full.notify_waiters();
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
            let bytes = crate::event_payload_size::heap_bytes(&event);
            crate::observability::record_tui_event_dequeued(bytes);
            self.inner.not_full.notify_one();
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
        capacity,
        notify: Notify::new(),
        not_full: Notify::new(),
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

pub fn bounded_content_events(event: TuiEvent) -> Vec<TuiEvent> {
    ContentFrames::new(event).collect()
}

/// Retained heap bytes owned by one event payload.
pub fn retained_event_bytes(event: &TuiEvent) -> usize {
    crate::event_payload_size::heap_bytes(event)
}

fn frame_is_oversized(event: &TuiEvent) -> bool {
    retained_event_bytes(event) > MAX_COALESCED_CONTENT_BYTES
}

fn coalesce_with_metrics(queue: &mut VecDeque<TuiEvent>, event: TuiEvent) -> Option<TuiEvent> {
    let previous_bytes = queue
        .back()
        .map(crate::event_payload_size::heap_bytes)
        .unwrap_or(0);
    let event = enqueue_or_coalesce_content_delta(queue, event);
    if event.is_none() {
        let current_bytes = queue
            .back()
            .map(crate::event_payload_size::heap_bytes)
            .unwrap_or(0);
        crate::observability::record_tui_event_coalesced_bytes(
            current_bytes.saturating_sub(previous_bytes),
        );
    }
    event
}

fn enqueue_or_coalesce_content_delta(
    queue: &mut VecDeque<TuiEvent>,
    event: TuiEvent,
) -> Option<TuiEvent> {
    match event {
        TuiEvent::TextDelta(text) => {
            if let Some(TuiEvent::TextDelta(previous)) = queue.back_mut()
                && previous.len().saturating_add(text.len()) <= MAX_COALESCED_CONTENT_BYTES
            {
                let mut combined = String::with_capacity(previous.len() + text.len());
                combined.push_str(previous);
                combined.push_str(&text);
                *previous = combined;
                return None;
            }
            Some(TuiEvent::TextDelta(text))
        }
        TuiEvent::ThinkingDelta(text) => {
            if let Some(TuiEvent::ThinkingDelta(previous)) = queue.back_mut()
                && previous.len().saturating_add(text.len()) <= MAX_COALESCED_CONTENT_BYTES
            {
                let mut combined = String::with_capacity(previous.len() + text.len());
                combined.push_str(previous);
                combined.push_str(&text);
                *previous = combined;
                return None;
            }
            Some(TuiEvent::ThinkingDelta(text))
        }
        TuiEvent::TransientThinkingDelta(text) => {
            if let Some(TuiEvent::TransientThinkingDelta(previous)) = queue.back_mut()
                && previous.len().saturating_add(text.len()) <= MAX_COALESCED_CONTENT_BYTES
            {
                let mut combined = String::with_capacity(previous.len() + text.len());
                combined.push_str(previous);
                combined.push_str(&text);
                *previous = combined;
                return None;
            }
            Some(TuiEvent::TransientThinkingDelta(text))
        }
        event => Some(event),
    }
}

#[cfg(test)]
#[path = "event_channel_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "event_channel_payload_tests.rs"]
mod payload_tests;
