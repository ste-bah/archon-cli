use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use archon_policy::CognitivePolicy;
use cozo::DbInstance;

use crate::CognitiveError;
use crate::config::CognitiveDaemonConfig;
use crate::daemon::job::{CognitiveTickJob, DaemonJob, DaemonJobReport};
use crate::daemon::lock::DaemonLock;
use crate::daemon::state::{DaemonPaths, DaemonState, DaemonStatus, status_for};

pub struct CognitiveDaemon<'a> {
    paths: DaemonPaths,
    config: CognitiveDaemonConfig,
    policy: CognitivePolicy,
    jobs: Vec<Box<dyn DaemonJob + 'a>>,
}

impl<'a> CognitiveDaemon<'a> {
    pub fn new(
        root: impl AsRef<Path>,
        config: CognitiveDaemonConfig,
        db: &'a DbInstance,
        policy: CognitivePolicy,
    ) -> Self {
        Self {
            paths: DaemonPaths::new(root),
            config,
            policy: policy.clone(),
            jobs: vec![Box::new(CognitiveTickJob::new(db, policy))],
        }
    }

    pub fn status(root: impl AsRef<Path>, stale_ms: u64) -> Result<DaemonStatus, CognitiveError> {
        status_for(&DaemonPaths::new(root), stale_ms)
    }

    pub fn request_stop(root: impl AsRef<Path>) -> Result<(), CognitiveError> {
        DaemonPaths::new(root).request_stop()
    }

    pub fn add_job(&mut self, job: impl DaemonJob + 'a) {
        self.jobs.push(Box::new(job));
    }

    pub fn run_once(&mut self) -> Result<DaemonState, CognitiveError> {
        self.ensure_allowed()?;
        let _lock = DaemonLock::acquire(&self.paths, self.config.stale_heartbeat_ms)?;
        self.paths.clear_stop()?;
        let mut state = DaemonState::new();
        self.run_jobs(&mut state)?;
        state.status = "stopped".into();
        self.paths.write_state(&state)?;
        Ok(state)
    }

    pub fn run_forever(&mut self) -> Result<DaemonState, CognitiveError> {
        self.ensure_allowed()?;
        let _lock = DaemonLock::acquire(&self.paths, self.config.stale_heartbeat_ms)?;
        self.paths.clear_stop()?;
        let mut state = DaemonState::new();
        self.paths.write_state(&state)?;
        if self.config.run_on_start {
            self.run_jobs(&mut state)?;
        }
        while self.should_continue(&state) {
            if !self.wait_interval() {
                break;
            }
            state.heartbeat();
            self.paths.write_state(&state)?;
            if self.paths.stop_path.exists() {
                break;
            }
            self.run_jobs(&mut state)?;
        }
        state.status = "stopped".into();
        self.paths.clear_stop()?;
        self.paths.write_state(&state)?;
        Ok(state)
    }

    fn ensure_allowed(&self) -> Result<(), CognitiveError> {
        if !self.config.enabled {
            return Err(CognitiveError::Store(
                "learning.cognitive.daemon.enabled is false".into(),
            ));
        }
        if !self.policy.can_run_daemon() {
            return Err(CognitiveError::Store(
                "policy.cognitive must enable autonomous tick and background daemon".into(),
            ));
        }
        Ok(())
    }

    fn should_continue(&self, state: &DaemonState) -> bool {
        if self.config.max_ticks_per_run == 0 {
            return true;
        }
        state.ticks_run < self.config.max_ticks_per_run
    }

    fn run_jobs(&mut self, state: &mut DaemonState) -> Result<(), CognitiveError> {
        let reports = self.run_all_jobs(state)?;
        let error = reports
            .iter()
            .find(|report| !report.ok)
            .map(|report| report.summary.clone());
        state.record_tick(error);
        self.paths.write_state(state)
    }

    fn run_all_jobs(
        &mut self,
        state: &mut DaemonState,
    ) -> Result<Vec<DaemonJobReport>, CognitiveError> {
        let mut reports = Vec::with_capacity(self.jobs.len());
        for job in self.jobs.iter_mut() {
            if self.paths.stop_path.exists() {
                break;
            }
            state.record_job_start(job.name());
            self.paths.write_state(state)?;
            let heartbeat = HeartbeatGuard::start(
                self.paths.clone(),
                state.clone(),
                heartbeat_interval_ms(self.config.stale_heartbeat_ms),
            );
            let report = match job.run() {
                Ok(report) => report,
                Err(error) => DaemonJobReport {
                    name: job.name().into(),
                    ok: false,
                    summary: error.to_string(),
                },
            };
            drop(heartbeat);
            state.heartbeat();
            self.paths.write_state(state)?;
            reports.push(report);
        }
        Ok(reports)
    }

    fn wait_interval(&self) -> bool {
        let mut waited_ms = 0;
        while waited_ms < self.config.interval_ms {
            if self.paths.stop_path.exists() {
                return false;
            }
            let next = (self.config.interval_ms - waited_ms).min(250);
            std::thread::sleep(Duration::from_millis(next));
            waited_ms += next;
        }
        true
    }
}

struct HeartbeatGuard {
    stop: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl HeartbeatGuard {
    fn start(paths: DaemonPaths, mut state: DaemonState, interval: Duration) -> Self {
        let (stop, stop_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            while stop_rx.recv_timeout(interval).is_err() {
                state.heartbeat();
                let _ = paths.write_state(&state);
            }
        });
        Self {
            stop: Some(stop),
            handle: Some(handle),
        }
    }
}

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn heartbeat_interval_ms(stale_ms: u64) -> Duration {
    Duration::from_millis((stale_ms / 4).clamp(1_000, 30_000))
}
