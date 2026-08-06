use std::sync::Mutex;

use crate::client::MemoryClient;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome};

use super::MemoryTrait;

static CURRENT_THREAD_BRIDGE_LOCK: Mutex<()> = Mutex::new(());

/// Helper: run an async call from a synchronous trait method.
///
/// Multi-threaded Tokio runtimes can yield the current worker with
/// `block_in_place`. Callers without a runtime run the future directly on a
/// local runtime. Current-thread runtime callers use a serialized scoped
/// thread so Tokio runtimes are not nested and bridge concurrency stays bounded.
/// The remote server must not depend on the blocked current-thread executor.
pub(super) fn block_on_async<F>(future: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Ok(_) => {
            let _guard = CURRENT_THREAD_BRIDGE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::thread::scope(|scope| {
                scope
                    .spawn(|| build_bridge_runtime().block_on(future))
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
        }
        Err(_) => build_bridge_runtime().block_on(future),
    }
}

fn build_bridge_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build memory client bridge runtime")
}

/// [`MemoryTrait`] implementation that bridges async [`MemoryClient`] calls
/// into the synchronous trait interface via [`block_on_async`].
impl MemoryTrait for MemoryClient {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        let result = block_on_async(self.call(
            "store_memory",
            serde_json::json!({
                "content": content,
                "title": title,
                "memory_type": format!("{memory_type}"),
                "importance": importance,
                "tags": tags,
                "source_type": source_type,
                "project_path": project_path,
            }),
        ))?;
        result
            .as_str()
            .map(String::from)
            .ok_or_else(|| MemoryError::Database("expected string id".to_string()))
    }

    fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        let result = block_on_async(self.call(
            "store_memory_with_id_outcome",
            serde_json::json!({
                "id": id,
                "content": content,
                "title": title,
                "memory_type": format!("{memory_type}"),
                "importance": importance,
                "tags": tags,
                "source_type": source_type,
                "project_path": project_path,
            }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call(
            "store_memory_with_id",
            serde_json::json!({
                "id": id,
                "content": content,
                "title": title,
                "memory_type": format!("{memory_type}"),
                "importance": importance,
                "tags": tags,
                "source_type": source_type,
                "project_path": project_path,
            }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call("get_memory", serde_json::json!({"id": id})))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call("inspect_memory", serde_json::json!({"id": id})))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        let mut params = serde_json::json!({"id": id});
        if let Some(c) = content {
            params["content"] = serde_json::Value::String(c.to_string());
        }
        if let Some(t) = tags {
            params["tags"] = serde_json::to_value(t)?;
        }
        block_on_async(self.call("update_memory", params))?;
        Ok(())
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call(
            "apply_importance_delta",
            serde_json::json!({
                "id": id,
                "delta": delta,
                "provenance_id": provenance_id,
            }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn reconcile_importance_trend(
        &self,
        id: &str,
        previous_importance: f64,
    ) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call(
            "reconcile_importance_trend",
            serde_json::json!({
                "id": id,
                "previous_importance": previous_importance,
            }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        let result = block_on_async(self.call(
            "has_importance_application",
            serde_json::json!({
                "memory_id": memory_id,
                "provenance_id": provenance_id,
            }),
        ))?;
        result
            .as_bool()
            .ok_or_else(|| MemoryError::Database("expected boolean application status".to_string()))
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        block_on_async(self.call("delete_memory", serde_json::json!({"id": id})))?;
        Ok(())
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        block_on_async(self.call(
            "create_relationship",
            serde_json::json!({
                "from_id": from_id,
                "to_id": to_id,
                "rel_type": format!("{rel_type}"),
                "context": context,
                "strength": strength,
            }),
        ))?;
        Ok(())
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let result = block_on_async(self.call(
            "recall_memories",
            serde_json::json!({"query": query, "limit": limit}),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        let filter_val = serde_json::to_value(filter)?;
        let result = block_on_async(
            self.call("search_memories", serde_json::json!({"filter": filter_val})),
        )?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let result = block_on_async(self.call("list_recent", serde_json::json!({"limit": limit})))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        let result = block_on_async(self.call("memory_count", serde_json::json!({})))?;
        result
            .as_u64()
            .map(|v| v as usize)
            .ok_or_else(|| MemoryError::Database("expected integer count".to_string()))
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        let result = block_on_async(self.call("clear_all", serde_json::json!({})))?;
        result
            .as_u64()
            .map(|v| v as usize)
            .ok_or_else(|| MemoryError::Database("expected integer count".to_string()))
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        let result = block_on_async(self.call(
            "get_related_memories",
            serde_json::json!({"id": id, "depth": depth}),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn embedding_neighbours(
        &self,
        memory_id: &str,
        top_k: usize,
    ) -> Result<Option<Vec<(String, f64)>>, MemoryError> {
        let result = block_on_async(self.call(
            "embedding_neighbours",
            serde_json::json!({"memory_id": memory_id, "top_k": top_k}),
        ));
        match result {
            Ok(value) => serde_json::from_value(value).map_err(MemoryError::from),
            // The process holding the database may be an older build whose
            // dispatch table has no such method, and it answers "unknown
            // method" rather than null. Unavailable is the honest reading of
            // that, and unlike the old empty-vec stub it is now sayable --
            // failing the whole consolidation pass over an optional index is
            // not.
            Err(error) => {
                tracing::debug!(%error, "memory server has no vector-neighbour request");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use super::block_on_async;

    #[test]
    fn async_bridge_works_without_an_active_runtime() {
        assert_eq!(block_on_async(async { 41 + 1 }), 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_bridge_works_inside_current_thread_runtime() {
        assert_eq!(block_on_async(async { 41 + 1 }), 42);
    }

    #[test]
    fn current_thread_bridge_calls_are_serialized() {
        let start = Arc::new(Barrier::new(3));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            for _ in 0..2 {
                let start = Arc::clone(&start);
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                scope.spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("build test runtime");
                    runtime.block_on(async {
                        start.wait();
                        block_on_async(async {
                            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                            max_active.fetch_max(now_active, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            active.fetch_sub(1, Ordering::SeqCst);
                        });
                    });
                });
            }
            start.wait();
        });

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }
}
