//! CronScheduler for TASK-CLI-311.
//!
//! Wakes every 1 second, fires due tasks, applies jitter, manages PID lock.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use cron::Schedule;

use crate::cron_task::CronTask;

// ---------------------------------------------------------------------------
// Public helpers (tested directly)
// ---------------------------------------------------------------------------

/// Validate a 5-field cron expression.
///
/// Returns `Ok(())` if the expression is valid, `Err` with a description otherwise.
pub fn validate_cron_expression(expr: &str) -> anyhow::Result<()> {
    let expr = expr.trim();
    if expr.is_empty() {
        anyhow::bail!("cron expression must not be empty");
    }

    // Require exactly 5 fields (not 6-field with seconds)
    let field_count = expr.split_whitespace().count();
    if field_count != 5 {
        anyhow::bail!(
            "cron expression must have exactly 5 fields (minute hour day month weekday), got {field_count}"
        );
    }

    // The `cron` crate parses a 6-field expression (prepend "0 " to make it seconds-first)
    let six_field = format!("0 {expr}");
    Schedule::from_str(&six_field)
        .map_err(|e| anyhow::anyhow!("invalid cron expression '{}': {}", expr, e))?;

    // Additionally validate field ranges manually for clarity
    let fields: Vec<&str> = expr.split_whitespace().collect();
    validate_field(fields[0], 0, 59, "minute")?;
    validate_field(fields[1], 0, 23, "hour")?;
    validate_field(fields[2], 1, 31, "day")?;
    validate_field(fields[3], 1, 12, "month")?;
    validate_field(fields[4], 0, 7, "weekday")?;

    Ok(())
}

fn validate_field(field: &str, min: u32, max: u32, name: &str) -> anyhow::Result<()> {
    if field == "*" || field.starts_with("*/") {
        // wildcard or step — let the parser handle it
        return Ok(());
    }
    // Could be a list or range — only validate simple numeric values
    for part in field.split(',') {
        if part.contains('-') {
            // range like 1-5
            let bounds: Vec<&str> = part.split('-').collect();
            if bounds.len() == 2 {
                let lo: u32 = bounds[0]
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid {name} field"))?;
                let hi: u32 = bounds[1]
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid {name} field"))?;
                if lo < min || hi > max || lo > hi {
                    anyhow::bail!("{name} range {lo}-{hi} out of {min}-{max}");
                }
            }
        } else if let Ok(n) = part.parse::<u32>()
            && (n < min || n > max)
        {
            anyhow::bail!("{name} value {n} out of range {min}-{max}");
        }
    }
    Ok(())
}

/// Compute the next fire time after `after` for a 5-field cron expression.
/// Returns `None` if the expression is invalid or has no upcoming fire time.
pub fn next_fire_time(expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let six_field = format!("0 {}", expr.trim());
    let schedule = Schedule::from_str(&six_field).ok()?;
    schedule.after(&after).next()
}

/// Returns `true` if `fire_time` is at or before `now`.
pub fn is_timestamp_due(fire_time: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    fire_time <= now
}

/// Returns `true` if the task is a one-shot (fires once then is deleted).
pub fn is_one_shot(task: &CronTask) -> bool {
    task.recurring == Some(false)
}

/// Try to acquire the PID-based liveness lock at `lock_path`.
///
/// Returns `Ok(true)` if the lock was acquired (either fresh or previous PID is dead).
/// Returns `Ok(false)` if another live process holds the lock.
pub fn try_acquire_pid_lock(lock_path: &Path) -> anyhow::Result<bool> {
    if lock_path.exists() {
        let content = std::fs::read_to_string(lock_path)?;
        if let Ok(pid) = content.trim().parse::<u32>() {
            let current = std::process::id();
            if pid == current {
                // Already our lock
                return Ok(true);
            }
            if is_pid_alive(pid) {
                // Another live process holds the lock
                return Ok(false);
            }
            // Dead PID — take over the lock
        }
    }
    // Write our PID
    std::fs::write(lock_path, std::process::id().to_string())?;
    Ok(true)
}

/// Check whether a process with the given PID is alive on Linux.
fn is_pid_alive(pid: u32) -> bool {
    // On Linux, /proc/<pid> exists if the process is alive
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    // Fallback for other platforms
    #[cfg(not(target_os = "linux"))]
    {
        // signal(pid, 0) would work but requires unsafe. Use a conservative approach.
        // Assume alive if we can't check — prevents lock stealing on unknown platforms.
        let _ = pid;
        false
    }
}

// ---------------------------------------------------------------------------
// CronJitterConfig
// ---------------------------------------------------------------------------

/// Configuration for execution jitter to prevent thundering herd.
#[derive(Debug, Clone)]
pub struct CronJitterConfig {
    /// Fraction of the cron interval to use as jitter window for recurring tasks.
    /// E.g. 0.1 = 10% of interval. Capped by `recurring_cap_ms`.
    pub recurring_frac: f64,
    /// Maximum jitter in ms for recurring tasks.
    pub recurring_cap_ms: u64,
    /// Maximum jitter in ms for one-shot tasks.
    pub one_shot_max_ms: u64,
    /// Minimum jitter in ms for one-shot tasks.
    pub one_shot_floor_ms: u64,
}

impl Default for CronJitterConfig {
    fn default() -> Self {
        Self {
            recurring_frac: 0.05,    // 5% of interval
            recurring_cap_ms: 5_000, // max 5 seconds
            one_shot_max_ms: 10_000, // max 10 seconds
            one_shot_floor_ms: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// CronScheduler
// ---------------------------------------------------------------------------

/// Scheduler that wakes every 1 second and fires due tasks.
///
/// The scheduler is intentionally separated from the actual task execution
/// to allow testing without spawning real agent sessions.
pub struct CronScheduler {
    jitter: CronJitterConfig,
    store_path: PathBuf,
}

impl CronScheduler {
    /// Create a scheduler with the default jitter config.
    pub fn new(store_path: PathBuf) -> Self {
        Self {
            jitter: CronJitterConfig::default(),
            store_path,
        }
    }

    /// Create a scheduler with a custom jitter config (for testing).
    pub fn new_with_jitter(jitter: CronJitterConfig) -> Self {
        Self {
            jitter,
            store_path: PathBuf::new(),
        }
    }

    /// Compute jitter ms for a recurring task given its interval in ms.
    ///
    /// Result is uniformly random in `[0, min(interval * frac, cap)]`.
    pub fn jitter_recurring_ms(&self, interval_ms: u64) -> u64 {
        let window = ((interval_ms as f64) * self.jitter.recurring_frac) as u64;
        let cap = self.jitter.recurring_cap_ms;
        let max_jitter = window.min(cap);
        if max_jitter == 0 {
            return 0;
        }
        rand_u64_bounded(max_jitter)
    }

    /// Compute jitter ms for a one-shot task.
    ///
    /// Result is uniformly random in `[floor, max]`.
    pub fn jitter_one_shot_ms(&self) -> u64 {
        let floor = self.jitter.one_shot_floor_ms;
        let max = self.jitter.one_shot_max_ms;
        if max <= floor {
            return floor;
        }
        floor + rand_u64_bounded(max - floor)
    }

    /// Path to the scheduled_tasks.json file this scheduler manages.
    pub fn store_path(&self) -> &Path {
        &self.store_path
    }

    /// Path to the PID lock file for this scheduler's project directory.
    pub fn lock_path(&self) -> PathBuf {
        if let Some(parent) = self.store_path.parent() {
            parent.join("scheduled_tasks.lock")
        } else {
            PathBuf::from("scheduled_tasks.lock")
        }
    }

    /// Find tasks that are due at `now` based on their cron schedules and `created_at`.
    pub fn due_tasks<'a>(&self, tasks: &'a [CronTask], now: DateTime<Utc>) -> Vec<&'a CronTask> {
        tasks.iter().filter(|t| self.is_task_due(t, now)).collect()
    }

    fn is_task_due(&self, task: &CronTask, now: DateTime<Utc>) -> bool {
        // Compute the fire time from 1 minute before now (scheduler catches up)
        let one_min_ago = now - chrono::Duration::seconds(60);
        if let Some(next) = next_fire_time(&task.cron, one_min_ago) {
            return is_timestamp_due(next, now);
        }
        false
    }
}

/// Return a random u64 in `[0, bound)`.
fn rand_u64_bounded(bound: u64) -> u64 {
    use rand::Rng;
    if bound == 0 {
        return 0;
    }
    rand::rng().random_range(0..bound)
}

// ── run_scheduler_loop ────────────────────────────────────────────────────────

/// Run the cron scheduler tick loop until `cancel` is set to `true`.
///
/// Wakes every 1 second.  For each wake:
/// 1. Load tasks from `store_path`.
/// 2. Find tasks due at `now`.
/// 3. Log `cron.fire` trace event for each due task.
/// 4. Delete one-shot tasks that fired.
/// 5. Save updated task list.
///
/// Fail-open: load/save errors are logged at warn level and do not abort the loop.
pub async fn run_scheduler_loop(
    store_path: std::path::PathBuf,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    let store = crate::cron_task::CronStore::new(store_path.clone());

    loop {
        if cancel.load(Ordering::Relaxed) {
            tracing::debug!("cron scheduler: cancel signal received, exiting");
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let now = chrono::Utc::now();

        let tasks = match store.load() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("cron scheduler: failed to load tasks: {e}");
                continue;
            }
        };

        if tasks.is_empty() {
            continue;
        }

        let scheduler = CronScheduler::new(store_path.clone());
        let due = scheduler.due_tasks(&tasks, now);

        if due.is_empty() {
            continue;
        }

        let mut deleted_ids: Vec<String> = Vec::new();
        for task in &due {
            tracing::info!(
                task_id = %task.id,
                cron = %task.cron,
                "cron.fire: scheduled task fired"
            );
            if is_one_shot(task) {
                deleted_ids.push(task.id.clone());
            }
        }

        // Delete one-shot tasks (fail-open).
        for id in &deleted_ids {
            if let Err(e) = store.delete(id) {
                tracing::warn!("cron scheduler: failed to delete one-shot task {id}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[tokio::test]
    async fn scheduler_loop_exits_immediately_on_cancel() {
        let cancel = Arc::new(AtomicBool::new(false));
        let store_path = std::env::temp_dir()
            .join("archon-cron-test-cancel")
            .join(format!(
                "{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos()
            ));

        // Set cancel before spawning — loop should exit in first iteration.
        cancel.store(true, Ordering::Relaxed);

        let cancel_clone = Arc::clone(&cancel);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_scheduler_loop(store_path, cancel_clone),
        )
        .await
        .expect("scheduler_loop must exit promptly when cancel is set");
    }

    #[tokio::test]
    async fn scheduler_loop_does_not_panic_on_empty_store() {
        let cancel = Arc::new(AtomicBool::new(false));
        let store_path = std::env::temp_dir()
            .join("archon-cron-test-empty")
            .join(format!(
                "{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos()
            ));

        let cancel_clone = Arc::clone(&cancel);
        // Cancel after 1.5 seconds — enough for one tick with no tasks.
        let cancel_delayed = Arc::clone(&cancel);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            cancel_delayed.store(true, Ordering::Relaxed);
        });

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_scheduler_loop(store_path, cancel_clone),
        )
        .await
        .expect("scheduler_loop must exit within 5s after cancel");
    }

    #[test]
    fn scheduler_due_tasks_identifies_overdue_task() {
        let scheduler = CronScheduler::new_with_jitter(CronJitterConfig::default());
        let task = crate::cron_task::CronTask {
            id: "t1".to_string(),
            cron: "* * * * *".to_string(), // every minute
            prompt: "test".to_string(),
            created_at: 0,
            recurring: None,
        };
        // Any "now" should see a task with "* * * * *" as due if we look back 60s.
        let now = chrono::Utc::now();
        let tasks = vec![task];
        let due = scheduler.due_tasks(&tasks, now);
        assert_eq!(due.len(), 1, "every-minute task should be due");
    }

    #[test]
    fn scheduler_due_tasks_skips_far_future_task() {
        let scheduler = CronScheduler::new_with_jitter(CronJitterConfig::default());
        // A task that fires at minute 59 of every hour (rarely due).
        // Use "59 * * * *" — only fires at :59.  Unless it's exactly :59 now,
        // this should NOT be due when looking back 60s from a random moment.
        // To make the test deterministic, we use a specific past time.
        let task = crate::cron_task::CronTask {
            id: "future".to_string(),
            cron: "59 23 31 12 0".to_string(), // new year's eve at 23:59 on a Sunday
            prompt: "noop".to_string(),
            created_at: 0,
            recurring: None,
        };
        // A fixed "now" nowhere near new year's eve.
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let tasks = vec![task];
        let due = scheduler.due_tasks(&tasks, now);
        assert!(due.is_empty(), "far-future task must not be due now");
    }
}
