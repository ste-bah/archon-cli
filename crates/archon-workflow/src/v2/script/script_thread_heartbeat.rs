//! The script thread's heartbeat, checked from outside it (Issue 364).
//!
//! A workflow.js script, its host-call futures and their timers share one
//! current-thread runtime. JavaScript that never gives that runtime a turn
//! starves all of them: a loop of microtasks (`await` on settled promises)
//! runs inside one QuickJS job drain, so an in-flight host call is never
//! polled again and none of its timers fire. The CPU watchdog cannot always
//! end it: the watchdog is paused while a host call is in flight, and the
//! last bridge event of a pool of parallel calls is often a call's start.
//!
//! So the thread carries a heartbeat. A ticker task on the script runtime
//! beats it every tick, and the host bridge beats it at every call's start and
//! end; each beat stores the monotonic time and the thread's CPU time. Two
//! checks read it from outside the code that starves:
//!
//! - The QuickJS interrupt handler ([`ScriptThreadHeartbeat::should_cut`]),
//!   which runs only while JavaScript runs. When the script thread has used a
//!   whole no-progress window of CPU since the last beat, the script is cut:
//!   from then on every interrupt check stops the script, and
//!   [`ScriptThreadHeartbeat::cut_signal`] ends the wait for its promise.
//!   CPU time does not grow while the thread waits for a core or the machine
//!   sleeps, so load and sleep never cut a script.
//! - A monitor OS thread ([`ScriptThreadHeartbeat::spawn_monitor`]). When the
//!   heartbeat has been stale for a whole window of monotonic time while the
//!   process used at least half of that window in CPU, it records the
//!   starvation (what was in flight) at once, so even a starvation no
//!   interrupt can end (a synchronous Rust loop inside a host future) names
//!   itself. It records the recovery too, if the heartbeat comes back. The
//!   monitor only records: process CPU may be other threads', so it never
//!   cuts a script.
//!
//! A healthy long host call (a model stream that takes an hour) is never cut:
//! the runtime is idle in it, so the ticker beats on time.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::js_cpu_clock::{process_cpu_time, thread_cpu_time};

/// What the host had in flight when the thread starved, as the caller shapes it.
pub type InFlightSnapshot = Box<dyn Fn() -> Vec<serde_json::Value> + Send + Sync>;
/// Where a starvation record goes (a durable file of the run, in the host).
pub type StarvationSink = Box<dyn Fn(&Starvation) + Send + Sync>;

/// One starvation of the script thread, as recorded.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Starvation {
    /// `interrupt_handler`, `cpu_watchdog` or `monitor`.
    pub detected_by: String,
    /// `suspected` (monitor only), `cut` or `recovered`.
    pub state: String,
    /// Monotonic time since the last beat.
    pub no_progress_ms: u64,
    /// The window it is measured against.
    pub window_ms: u64,
    /// Script-thread CPU since the last beat, when the script thread read it.
    pub script_thread_cpu_ms: Option<u64>,
    /// Process CPU since the beat, when the monitor read it.
    pub process_cpu_ms: Option<u64>,
    /// What was in flight.
    pub in_flight: Vec<serde_json::Value>,
}

const UNKNOWN: u64 = u64::MAX;

struct Shared {
    base: Instant,
    window: Duration,
    /// Monotonic nanoseconds since `base` at the last beat.
    beat_at: AtomicU64,
    /// The script thread's CPU nanoseconds at the last beat, or `UNKNOWN`.
    beat_cpu: AtomicU64,
    cut: AtomicBool,
    declared: Mutex<Option<Starvation>>,
    in_flight: InFlightSnapshot,
    sink: StarvationSink,
}

/// The heartbeat of one script thread. Cheap to clone.
#[derive(Clone)]
pub struct ScriptThreadHeartbeat {
    shared: Arc<Shared>,
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(UNKNOWN - 1)
}

fn millis(nanos: u64) -> u64 {
    nanos / 1_000_000
}

impl ScriptThreadHeartbeat {
    /// A heartbeat that cuts after `window` without progress. Create it on
    /// the script thread: the first beat is taken now.
    pub fn new(window: Duration, in_flight: InFlightSnapshot, sink: StarvationSink) -> Self {
        let heartbeat = Self {
            shared: Arc::new(Shared {
                base: Instant::now(),
                window,
                beat_at: AtomicU64::new(0),
                beat_cpu: AtomicU64::new(UNKNOWN),
                cut: AtomicBool::new(false),
                declared: Mutex::new(None),
                in_flight,
                sink,
            }),
        };
        heartbeat.beat();
        heartbeat
    }

    pub fn window(&self) -> Duration {
        self.shared.window
    }

    /// The script thread made progress. Call it on the script thread only.
    pub fn beat(&self) {
        let shared = &self.shared;
        shared
            .beat_cpu
            .store(thread_cpu_time().map_or(UNKNOWN, nanos), Ordering::SeqCst);
        shared
            .beat_at
            .store(nanos(shared.base.elapsed()), Ordering::SeqCst);
    }

    fn stale_for(&self) -> Duration {
        let beat = Duration::from_nanos(self.shared.beat_at.load(Ordering::SeqCst));
        self.shared.base.elapsed().saturating_sub(beat)
    }

    /// Script-thread CPU since the last beat. Call it on the script thread.
    fn thread_cpu_since_beat(&self) -> Option<Duration> {
        let at = self.shared.beat_cpu.load(Ordering::SeqCst);
        let now = thread_cpu_time()?;
        (at != UNKNOWN).then(|| now.saturating_sub(Duration::from_nanos(at)))
    }

    /// Beats every `tick` on the current (script) runtime until dropped.
    pub fn spawn_ticker(&self, tick: Duration) -> TickerGuard {
        let heartbeat = self.clone();
        TickerGuard(tokio::spawn(async move {
            let mut every = tokio::time::interval(tick);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                heartbeat.beat();
            }
        }))
    }

    /// The QuickJS interrupt check, on the script thread while JavaScript
    /// runs: true once the thread has starved for a whole window. Sticky.
    pub fn should_cut(&self) -> bool {
        if self.shared.cut.load(Ordering::SeqCst) {
            return true;
        }
        let window = self.shared.window;
        let cpu = self.thread_cpu_since_beat();
        // Only the thread's own CPU decides: a thread that was blocked (off
        // the CPU) while others burned it is not starving. Without a CPU
        // clock, monotonic time still bounds it.
        let starved = cpu.unwrap_or_else(|| self.stale_for()) >= window;
        if starved {
            self.declare_cut("interrupt_handler", cpu);
        }
        starved
    }

    /// The CPU watchdog cut the script. With host calls in flight that is a
    /// starvation of those calls too, and it is recorded as one.
    pub fn note_watchdog_cut(&self) {
        if self.shared.cut.load(Ordering::SeqCst) || (self.shared.in_flight)().is_empty() {
            return;
        }
        self.declare_cut("cpu_watchdog", self.thread_cpu_since_beat());
    }

    fn declare_cut(&self, detected_by: &str, cpu: Option<Duration>) {
        let starvation = Starvation {
            detected_by: detected_by.to_string(),
            state: "cut".to_string(),
            no_progress_ms: millis(nanos(self.stale_for())),
            window_ms: millis(nanos(self.shared.window)),
            script_thread_cpu_ms: cpu.map(|cpu| millis(nanos(cpu))),
            process_cpu_ms: None,
            in_flight: (self.shared.in_flight)(),
        };
        if let Ok(mut declared) = self.shared.declared.lock() {
            declared.get_or_insert(starvation);
        }
        self.shared.cut.store(true, Ordering::SeqCst);
    }

    /// The starvation that cut the script, if one did.
    pub fn cut(&self) -> Option<Starvation> {
        if !self.shared.cut.load(Ordering::SeqCst) {
            return None;
        }
        self.shared.declared.lock().ok()?.clone()
    }

    /// Writes `starvation` to the sink.
    pub fn record(&self, starvation: &Starvation) {
        (self.shared.sink)(starvation);
    }

    /// Resolves once the script is cut; pending before.
    pub async fn cut_signal(&self) {
        let poll =
            (self.shared.window / 10).clamp(Duration::from_millis(1), Duration::from_secs(1));
        while !self.shared.cut.load(Ordering::SeqCst) {
            tokio::time::sleep(poll).await;
        }
    }

    /// Starts the monitor thread; it polls every `poll` until the returned
    /// handle is dropped.
    pub fn spawn_monitor(&self, poll: Duration) -> MonitorHandle {
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let heartbeat = self.clone();
        let spawned = std::thread::Builder::new()
            .name("workflow-js-heartbeat".into())
            .spawn(move || {
                let mut monitor = Monitor::new(heartbeat);
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    stopped.recv_timeout(poll)
                {
                    monitor.check();
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "workflow.js heartbeat monitor did not start");
        }
        MonitorHandle(Some(stop))
    }
}

/// What the monitor thread remembers between polls.
struct Monitor {
    heartbeat: ScriptThreadHeartbeat,
    seen_beat: u64,
    cpu_at_beat: Option<Duration>,
    reported: Option<Starvation>,
}

impl Monitor {
    fn new(heartbeat: ScriptThreadHeartbeat) -> Self {
        Self {
            seen_beat: heartbeat.shared.beat_at.load(Ordering::SeqCst),
            heartbeat,
            cpu_at_beat: process_cpu_time(),
            reported: None,
        }
    }

    fn check(&mut self) {
        let shared = &self.heartbeat.shared;
        if shared.cut.load(Ordering::SeqCst) {
            return;
        }
        let beat = shared.beat_at.load(Ordering::SeqCst);
        if beat != self.seen_beat {
            self.seen_beat = beat;
            self.cpu_at_beat = process_cpu_time();
            if let Some(mut recovered) = self.reported.take() {
                recovered.state = "recovered".to_string();
                self.heartbeat.record(&recovered);
            }
            return;
        }
        let stale = self.heartbeat.stale_for();
        if self.reported.is_some() || stale < shared.window {
            return;
        }
        let burned = match (self.cpu_at_beat, process_cpu_time()) {
            (Some(at), Some(now)) => now.saturating_sub(at),
            _ => return,
        };
        if burned < stale / 2 {
            return;
        }
        let starvation = Starvation {
            detected_by: "monitor".to_string(),
            state: "suspected".to_string(),
            no_progress_ms: millis(nanos(stale)),
            window_ms: millis(nanos(shared.window)),
            script_thread_cpu_ms: None,
            process_cpu_ms: Some(millis(nanos(burned))),
            in_flight: (shared.in_flight)(),
        };
        self.heartbeat.record(&starvation);
        self.reported = Some(starvation);
    }
}

/// Aborts the ticker task when dropped.
pub struct TickerGuard(tokio::task::JoinHandle<()>);

impl Drop for TickerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Stops the monitor thread when dropped.
pub struct MonitorHandle(Option<std::sync::mpsc::Sender<()>>);

impl Drop for MonitorHandle {
    fn drop(&mut self) {
        // Dropping the sender disconnects the channel; the thread ends at
        // its next poll.
        self.0.take();
    }
}

#[cfg(test)]
#[path = "script_thread_heartbeat_tests.rs"]
mod tests;
