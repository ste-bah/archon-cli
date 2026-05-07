use crate::client::MemoryClient;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

use super::MemoryTrait;

/// Helper: run an async call on the current tokio runtime from a sync context.
///
/// Uses `block_in_place` + `block_on` so the current thread yields to the
/// scheduler while blocking.
///
/// # Panics
///
/// Panics if called outside a **multi-threaded** tokio runtime (i.e. there is
/// no current `Handle`, or the runtime is single-threaded and
/// `block_in_place` is unsupported).
fn block_on_async<F: std::future::Future>(f: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

/// [`MemoryTrait`] implementation that bridges async [`MemoryClient`] calls
/// into the synchronous trait interface via [`block_on_async`].
///
/// # Runtime requirement
///
/// `MemoryClient` requires a **multi-threaded** tokio runtime.  Calling any
/// [`MemoryTrait`] method on a `MemoryClient` from outside a tokio runtime
/// (or from a single-threaded runtime) will panic.
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

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        let result = block_on_async(self.call("get_memory", serde_json::json!({"id": id})))?;
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

    fn update_importance(&self, id: &str, importance: f64) -> Result<(), MemoryError> {
        block_on_async(self.call(
            "update_importance",
            serde_json::json!({"id": id, "importance": importance}),
        ))?;
        Ok(())
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
}
