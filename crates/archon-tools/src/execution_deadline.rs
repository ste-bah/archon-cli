use std::future::Future;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

pub struct ExecutionDeadline {
    end: Instant,
}

impl ExecutionDeadline {
    pub fn new(timeout: Duration) -> Self {
        Self {
            end: Instant::now() + timeout,
        }
    }

    pub fn remaining(&self) -> Duration {
        self.end.saturating_duration_since(Instant::now())
    }

    pub async fn wait<T>(&self, future: impl Future<Output = T>) -> Option<T> {
        tokio::time::timeout_at(self.end.into(), future).await.ok()
    }
}

pub async fn join_pipe_tasks<T>(
    deadline: &ExecutionDeadline,
    stdout: &mut JoinHandle<T>,
    stderr: &mut JoinHandle<T>,
) -> Option<(T, T)> {
    let joined = deadline
        .wait(async { tokio::join!(&mut *stdout, &mut *stderr) })
        .await?;
    Some((joined.0.ok()?, joined.1.ok()?))
}

pub fn abort_pipe_tasks<T>(stdout: &JoinHandle<T>, stderr: &JoinHandle<T>) {
    stdout.abort();
    stderr.abort();
}
