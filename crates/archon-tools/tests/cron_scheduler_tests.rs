//! Tests for TASK-CLI-311: CronScheduler — jitter, PID lock, cron validation.

use archon_tools::cron_scheduler::{CronJitterConfig, CronScheduler, validate_cron_expression};
use archon_tools::cron_task::CronTask;
use tempfile::TempDir;

fn make_recurring_task(id: &str) -> CronTask {
    CronTask {
        id: id.to_string(),
        cron: "* * * * *".to_string(),
        prompt: "test prompt".to_string(),
        created_at: chrono::Utc::now().timestamp_millis() as u64,
        recurring: None,
    }
}

// ---------------------------------------------------------------------------
// Cron expression validation
// ---------------------------------------------------------------------------

#[test]
fn valid_cron_expressions_pass() {
    assert!(validate_cron_expression("* * * * *").is_ok(), "every minute");
    assert!(validate_cron_expression("0 9 * * 1").is_ok(), "9am Monday");
    assert!(validate_cron_expression("*/5 * * * *").is_ok(), "every 5 minutes");
    assert!(validate_cron_expression("0 0 1 * *").is_ok(), "first of month");
    assert!(validate_cron_expression("30 14 * * 5").is_ok(), "Friday 2:30pm");
}

#[test]
fn invalid_cron_expressions_fail() {
    assert!(validate_cron_expression("").is_err(), "empty string");
    assert!(validate_cron_expression("* * * *").is_err(), "only 4 fields");
    assert!(validate_cron_expression("60 * * * *").is_err(), "minute 60");
    assert!(validate_cron_expression("* 25 * * *").is_err(), "hour 25");
    assert!(validate_cron_expression("not-a-cron").is_err(), "text");
}

#[test]
fn six_field_cron_rejected() {
    // 5 fields only — 6-field (with seconds) should fail
    assert!(validate_cron_expression("0 * * * * *").is_err(), "6 fields should fail");
}

// ---------------------------------------------------------------------------
// Jitter configuration
// ---------------------------------------------------------------------------

#[test]
fn jitter_config_default_values() {
    let cfg = CronJitterConfig::default();
    assert!(cfg.recurring_frac > 0.0 && cfg.recurring_frac <= 1.0);
    assert!(cfg.recurring_cap_ms > 0);
    assert!(cfg.one_shot_max_ms > 0);
    assert!(cfg.one_shot_floor_ms >= 0);
    assert!(cfg.one_shot_floor_ms < cfg.one_shot_max_ms);
}

#[test]
fn jitter_for_recurring_bounded_by_cap() {
    let cfg = CronJitterConfig {
        recurring_frac: 0.1,
        recurring_cap_ms: 1000,
        one_shot_max_ms: 5000,
        one_shot_floor_ms: 0,
    };
    let scheduler = CronScheduler::new_with_jitter(cfg);
    for _ in 0..100 {
        let jitter = scheduler.jitter_recurring_ms(60_000);
        assert!(jitter <= 1000, "jitter must not exceed cap: got {jitter}");
    }
}

#[test]
fn jitter_for_one_shot_within_bounds() {
    let cfg = CronJitterConfig {
        recurring_frac: 0.1,
        recurring_cap_ms: 1000,
        one_shot_max_ms: 5000,
        one_shot_floor_ms: 100,
    };
    let scheduler = CronScheduler::new_with_jitter(cfg);
    for _ in 0..100 {
        let jitter = scheduler.jitter_one_shot_ms();
        assert!(jitter >= 100, "jitter must be >= floor: got {jitter}");
        assert!(jitter <= 5000, "jitter must be <= max: got {jitter}");
    }
}

// ---------------------------------------------------------------------------
// PID lock
// ---------------------------------------------------------------------------

#[test]
fn pid_lock_acquired_for_fresh_file() {
    let dir = TempDir::new().unwrap();
    let lock_path = dir.path().join("scheduled_tasks.lock");
    let result = archon_tools::cron_scheduler::try_acquire_pid_lock(&lock_path);
    assert!(result.is_ok(), "fresh lock file should be acquired");
    assert!(result.unwrap(), "fresh lock should succeed");
}

#[test]
fn pid_lock_with_dead_pid_is_taken() {
    let dir = TempDir::new().unwrap();
    let lock_path = dir.path().join("scheduled_tasks.lock");
    // Write a definitely-dead PID (PID 1 is init, but we write an impossible PID)
    std::fs::write(&lock_path, "99999999").unwrap();
    let result = archon_tools::cron_scheduler::try_acquire_pid_lock(&lock_path);
    assert!(result.is_ok());
    assert!(result.unwrap(), "dead PID lock should be taken over");
}

#[test]
fn pid_lock_with_own_pid_succeeds() {
    let dir = TempDir::new().unwrap();
    let lock_path = dir.path().join("scheduled_tasks.lock");
    // Write our own PID
    std::fs::write(&lock_path, std::process::id().to_string()).unwrap();
    let result = archon_tools::cron_scheduler::try_acquire_pid_lock(&lock_path);
    assert!(result.is_ok());
    // Our own PID → lock is "ours"
    assert!(result.unwrap());
}

// ---------------------------------------------------------------------------
// Task due-time detection
// ---------------------------------------------------------------------------

#[test]
fn is_due_returns_true_for_past_time() {
    let now = chrono::Utc::now();
    let past = now - chrono::Duration::seconds(1);
    assert!(archon_tools::cron_scheduler::is_timestamp_due(past, now));
}

#[test]
fn is_due_returns_false_for_future_time() {
    let now = chrono::Utc::now();
    let future = now + chrono::Duration::seconds(10);
    assert!(!archon_tools::cron_scheduler::is_timestamp_due(future, now));
}

// ---------------------------------------------------------------------------
// One-shot deletion flag
// ---------------------------------------------------------------------------

#[test]
fn one_shot_task_marked_for_deletion() {
    let task = CronTask {
        id: "shot-1".to_string(),
        cron: "* * * * *".to_string(),
        prompt: "once".to_string(),
        created_at: 0,
        recurring: Some(false),
    };
    assert!(archon_tools::cron_scheduler::is_one_shot(&task));
}

#[test]
fn recurring_task_not_marked_for_deletion() {
    let task = make_recurring_task("r-1");
    assert!(!archon_tools::cron_scheduler::is_one_shot(&task));
}

#[test]
fn explicit_recurring_true_not_one_shot() {
    let task = CronTask {
        id: "r-2".to_string(),
        cron: "* * * * *".to_string(),
        prompt: "p".to_string(),
        created_at: 0,
        recurring: Some(true),
    };
    assert!(!archon_tools::cron_scheduler::is_one_shot(&task));
}
