use std::io::Read as IoRead;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio_stream::StreamExt;

use crate::tasks::metrics::MetricsRegistry;
use crate::tasks::models::{
    SubmitRequest, TaskError, TaskFilter, TaskId, TaskResultStream, TaskState,
};
use crate::tasks::service::TaskService;

pub struct CliTaskApi {
    service: Arc<dyn TaskService>,
    metrics: Arc<MetricsRegistry>,
}

impl CliTaskApi {
    pub fn new(service: Arc<dyn TaskService>, metrics: Arc<MetricsRegistry>) -> Self {
        Self { service, metrics }
    }

    /// Submit: run-agent-async
    pub async fn submit(
        &self,
        agent_name: String,
        input_path: Option<String>,
        version: Option<String>,
        _detach: bool,
    ) -> Result<String, TaskError> {
        // Parse input from file, stdin, or empty
        let input_json = match input_path {
            Some(ref p) if p == "-" => {
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(TaskError::Io)?;
                serde_json::from_str(&buf).unwrap_or(serde_json::json!({"input": buf}))
            }
            Some(ref p) => {
                let content = std::fs::read_to_string(p).map_err(TaskError::Io)?;
                serde_json::from_str(&content).unwrap_or(serde_json::json!({"input": content}))
            }
            None => serde_json::json!({}),
        };

        let agent_version = version.and_then(|v| semver::Version::parse(&v).ok());

        let req = SubmitRequest {
            agent_name,
            agent_version,
            input: input_json,
            owner: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        };

        let task_id = self.service.submit(req).await?;
        Ok(serde_json::json!({"task_id": task_id.to_string()}).to_string())
    }

    /// Status: task-status [--watch]
    pub async fn status(&self, task_id_str: &str, watch: bool) -> Result<String, TaskError> {
        let id = task_id_str
            .parse::<TaskId>()
            .map_err(|_| TaskError::InvalidState)?;

        if watch {
            loop {
                let snap = self.service.status(id).await?;
                let json = serde_json::to_string_pretty(&snap).unwrap_or_default();
                println!("{json}");
                if snap.state.is_terminal() {
                    return Ok(json);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        } else {
            let snap = self.service.status(id).await?;
            Ok(serde_json::to_string_pretty(&snap).unwrap_or_default())
        }
    }

    /// Result: task-result [--stream]
    pub async fn result(&self, task_id_str: &str, stream: bool) -> Result<String, TaskError> {
        let id = task_id_str
            .parse::<TaskId>()
            .map_err(|_| TaskError::InvalidState)?;

        match self.service.result(id, stream).await {
            Ok(TaskResultStream::Inline(data)) => Ok(data),
            Ok(TaskResultStream::File(path)) => {
                std::fs::read_to_string(&path).map_err(TaskError::Io)
            }
            Err(TaskError::Pending) => {
                // Exit code 2 for pending tasks
                Err(TaskError::Pending)
            }
            Err(e) => Err(e),
        }
    }

    /// Cancel: task-cancel
    pub async fn cancel(&self, task_id_str: &str) -> Result<String, TaskError> {
        let id = task_id_str
            .parse::<TaskId>()
            .map_err(|_| TaskError::InvalidState)?;
        self.service.cancel(id).await?;
        Ok(serde_json::json!({"status": "cancelled", "task_id": task_id_str}).to_string())
    }

    /// List: task-list [--state X] [--agent Y] [--since Z]
    pub async fn list(
        &self,
        state: Option<String>,
        agent: Option<String>,
        since: Option<String>,
    ) -> Result<String, TaskError> {
        let state_filter = state.and_then(|s| match s.to_lowercase().as_str() {
            "pending" => Some(TaskState::Pending),
            "running" => Some(TaskState::Running),
            "finished" => Some(TaskState::Finished),
            "failed" => Some(TaskState::Failed),
            "cancelled" => Some(TaskState::Cancelled),
            _ => None,
        });

        let since_filter = since.and_then(|s| parse_duration_ago(&s));

        let filter = TaskFilter {
            state: state_filter,
            agent_name: agent,
            since: since_filter,
        };

        let snapshots = self.service.list(filter).await?;
        Ok(serde_json::to_string_pretty(&snapshots).unwrap_or_default())
    }

    /// Events: task-events --from-seq N (NDJSON)
    pub async fn events(&self, task_id_str: &str, from_seq: u64) -> Result<String, TaskError> {
        let id = task_id_str
            .parse::<TaskId>()
            .map_err(|_| TaskError::InvalidState)?;
        let mut stream = self.service.subscribe_events(id, from_seq).await?;

        let mut output = String::new();
        while let Some(event) = stream.next().await {
            let line = serde_json::to_string(&event).unwrap_or_default();
            println!("{line}");
            output.push_str(&line);
            output.push('\n');
        }
        Ok(output)
    }

    /// Metrics: prometheus text format
    pub fn metrics(&self) -> String {
        self.metrics.export_prometheus()
    }
}

/// Parse a duration string like "1h", "30m", "7d" into a DateTime (now - duration).
pub fn parse_duration_ago(s: &str) -> Option<chrono::DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
    let num: i64 = num_str.parse().ok()?;
    let duration = match unit {
        "s" => chrono::Duration::seconds(num),
        "m" => chrono::Duration::minutes(num),
        "h" => chrono::Duration::hours(num),
        "d" => chrono::Duration::days(num),
        _ => return None,
    };
    Some(Utc::now() - duration)
}
