//! Notification queue for toast-style overlay messages (REQ-MOD-017).
//!
//! Provides a typed `NotificationQueue` that holds in-flight notifications
//! with expiry times. Expired notifications are pruned on `tick()`.

use std::collections::VecDeque;
use std::time::Instant;

/// Notification severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
    Success,
}

/// A single notification with an expiry time.
#[derive(Debug)]
pub struct Notification {
    /// Unique identifier, incrementing on each push.
    pub id: u64,
    /// Severity level.
    pub level: Level,
    /// Human-readable message text.
    pub text: String,
    /// Instant at which this notification becomes stale.
    pub expires_at: Instant,
}

/// FIFO queue of active notifications.
///
/// Notifications whose `expires_at` has passed are removed on `tick()`.
pub struct NotificationQueue {
    notifications: VecDeque<Notification>,
    next_id: u64,
}

impl NotificationQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self {
            notifications: VecDeque::new(),
            next_id: 0,
        }
    }

    /// Add a notification that expires after `duration`.
    ///
    /// Returns the assigned id (monotonically increasing).
    pub fn push(&mut self, level: Level, text: String, duration: std::time::Duration) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let expires_at = Instant::now()
            .checked_add(duration)
            .unwrap_or_else(Instant::now);
        self.notifications.push_back(Notification {
            id,
            level,
            text,
            expires_at,
        });
        id
    }

    /// Remove all notifications whose `expires_at` has passed.
    pub fn tick(&mut self, now: Instant) {
        while self
            .notifications
            .front()
            .map_or(false, |n| n.expires_at <= now)
        {
            self.notifications.pop_front();
        }
    }

    /// Reference to the currently active (not yet expired) notifications.
    pub fn active(&self) -> &VecDeque<Notification> {
        &self.notifications
    }
}

impl Default for NotificationQueue {
    fn default() -> Self {
        Self::new()
    }
}
