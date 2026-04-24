use archon_tui::notifications::{Level, NotificationQueue};
use std::time::{Duration, Instant};

#[test]
fn expired_notification_removed_after_tick() {
    let mut q = NotificationQueue::new();
    q.push(Level::Info, "short".into(), Duration::from_millis(10));
    q.push(Level::Warn, "long".into(), Duration::from_secs(60));

    std::thread::sleep(Duration::from_millis(20));
    q.tick(Instant::now());

    assert_eq!(q.active().len(), 1);
    assert_eq!(q.active()[0].text, "long");
}

#[test]
fn empty_queue_tick_is_noop() {
    let mut q = NotificationQueue::new();
    q.tick(Instant::now());
    assert_eq!(q.active().len(), 0);
}

#[test]
fn push_returns_incrementing_ids() {
    let mut q = NotificationQueue::new();
    let id1 = q.push(Level::Info, "a".into(), Duration::from_secs(60));
    let id2 = q.push(Level::Warn, "b".into(), Duration::from_secs(60));
    assert!(id2 > id1);
}

#[test]
fn all_expired_empty_queue() {
    let mut q = NotificationQueue::new();
    q.push(Level::Info, "x".into(), Duration::from_millis(5));
    std::thread::sleep(Duration::from_millis(15));
    q.tick(Instant::now());
    assert_eq!(q.active().len(), 0);
}
