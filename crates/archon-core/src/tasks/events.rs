use std::path::{Path, PathBuf};

use dashmap::DashMap;
use tokio::sync::{Mutex, broadcast};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::tasks::models::{TaskError, TaskEvent, TaskId};

/// Per-task event log backed by a file of length-prefixed JSON records.
///
/// File format: repeated `[u32 len (LE)][json bytes]`.
/// The [`Mutex`] ensures single-writer semantics so concurrent
/// [`append`](Self::append) calls never interleave partial records.
pub struct EventLog {
    path: PathBuf,
    writer: Mutex<tokio::fs::File>,
}

impl EventLog {
    /// Open or create an event log for the given task.
    pub async fn open(dir: &Path, task_id: TaskId) -> Result<Self, TaskError> {
        let path = dir.join(format!("{}.events.log", task_id));
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(TaskError::Io)?;

        Ok(Self {
            path,
            writer: Mutex::new(file),
        })
    }

    /// Append an event to the log. The Mutex ensures serialized writes.
    pub async fn append(&self, event: TaskEvent) -> Result<(), TaskError> {
        use tokio::io::AsyncWriteExt;

        let json = serde_json::to_vec(&event).map_err(|e| {
            TaskError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;

        let len = json.len() as u32;
        let mut writer = self.writer.lock().await;
        writer
            .write_all(&len.to_le_bytes())
            .await
            .map_err(TaskError::Io)?;
        writer.write_all(&json).await.map_err(TaskError::Io)?;
        writer.flush().await.map_err(TaskError::Io)?;

        Ok(())
    }

    /// Replay events from the log starting at `from_seq`.
    ///
    /// Reads the entire file and returns all events whose `seq >= from_seq`.
    pub async fn replay(&self, from_seq: u64) -> Result<Vec<TaskEvent>, TaskError> {
        let data = tokio::fs::read(&self.path).await.map_err(TaskError::Io)?;
        let mut cursor = 0;
        let mut events = Vec::new();

        while cursor + 4 <= data.len() {
            let len = u32::from_le_bytes([
                data[cursor],
                data[cursor + 1],
                data[cursor + 2],
                data[cursor + 3],
            ]) as usize;
            cursor += 4;

            if cursor + len > data.len() {
                break; // truncated record — stop here
            }

            if let Ok(event) = serde_json::from_slice::<TaskEvent>(&data[cursor..cursor + len]) {
                if event.seq >= from_seq {
                    events.push(event);
                }
            }
            cursor += len;
        }

        Ok(events)
    }
}

/// In-memory event bus for live event subscription via broadcast channels.
///
/// Each task gets its own broadcast channel (capacity 256). Subscribers
/// receive a [`BroadcastStream`] that yields [`TaskEvent`] items,
/// filtering out any lagged errors automatically.
pub struct EventBus {
    senders: DashMap<TaskId, broadcast::Sender<TaskEvent>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            senders: DashMap::new(),
        }
    }

    /// Subscribe to live events for a task.
    ///
    /// Returns a pinned, boxed stream that yields [`TaskEvent`] items.
    /// Lagged items (when the subscriber falls behind the 256-slot
    /// buffer) are silently dropped.
    pub fn subscribe(
        &self,
        task_id: TaskId,
    ) -> std::pin::Pin<Box<dyn tokio_stream::Stream<Item = TaskEvent> + Send>> {
        let sender = self
            .senders
            .entry(task_id)
            .or_insert_with(|| broadcast::channel(256).0);
        let rx = sender.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(
            |r: Result<TaskEvent, tokio_stream::wrappers::errors::BroadcastStreamRecvError>| r.ok(),
        );
        Box::pin(stream)
    }

    /// Broadcast an event to all subscribers of a task.
    pub fn broadcast(&self, task_id: TaskId, event: TaskEvent) {
        if let Some(sender) = self.senders.get(&task_id) {
            let _ = sender.send(event); // ignore if no receivers
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
