//! TCP server that fronts a shared [`MemoryGraph`] instance.
//!
//! The first Archon session opens CozoDB and starts this server on
//! `127.0.0.1:0`. Subsequent sessions connect as JSON-RPC clients.
//!
//! # Concurrency model
//!
//! [`MemoryGraph`] wraps CozoDB's [`DbInstance`](cozo::DbInstance), which
//! handles its own internal concurrency via `ShardedLock` and atomic
//! counters.  All [`MemoryGraph`] methods take `&self`, so no external
//! write-lock is required — an `Arc<MemoryGraph>` is sufficient for
//! shared concurrent access across Tokio tasks.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::{debug, error, warn};

use crate::graph::MemoryGraph;
use crate::protocol::{Request, make_response_err, make_response_ok};
use crate::types::{MemoryError, MemoryType, RelType, SearchFilter};

/// A TCP server wrapping a shared [`MemoryGraph`].
pub struct MemoryServer;

impl MemoryServer {
    /// Start the server, bind to `127.0.0.1:0`, write the assigned port to
    /// `port_file`, and return `(port, join_handle)`.
    ///
    /// The server task runs until the returned handle is aborted or all
    /// connections close.
    pub async fn start(
        graph: Arc<MemoryGraph>,
        port_file: PathBuf,
    ) -> Result<(u16, tokio::task::JoinHandle<()>), MemoryError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let port = addr.port();

        // Write port file so other sessions can find us.
        if let Some(parent) = port_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&port_file, port.to_string())?;
        debug!(port, "memory server listening");

        let pf = port_file.clone();
        let handle = tokio::spawn(async move {
            Self::accept_loop(listener, graph).await;
            // Clean up port file on shutdown.
            let _ = std::fs::remove_file(&pf);
        });

        Ok((port, handle))
    }

    async fn accept_loop(listener: TcpListener, graph: Arc<MemoryGraph>) {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted memory client");
                    let g = Arc::clone(&graph);
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, g).await {
                            warn!(%peer, error = %e, "client connection error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "accept failed");
                    break;
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::TcpStream,
        graph: Arc<MemoryGraph>,
    ) -> Result<(), MemoryError> {
        let (reader, mut writer) = stream.into_split();
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let n = buf_reader.read_line(&mut line).await?;
            if n == 0 {
                // Client disconnected.
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let req: Request = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    let resp = make_response_err(0, format!("invalid request: {e}"));
                    writer.write_all(resp.as_bytes()).await?;
                    continue;
                }
            };

            let resp = match dispatch(&graph, &req.method, &req.params) {
                Ok(val) => make_response_ok(req.id, val),
                Err(msg) => make_response_err(req.id, msg),
            };

            writer.write_all(resp.as_bytes()).await?;
        }

        Ok(())
    }
}

// ── dispatch ───────────────────────────────────────────────────

/// Route a JSON-RPC method call to the appropriate [`MemoryGraph`] method.
fn dispatch(graph: &MemoryGraph, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "ping" => Ok(Value::String("pong".to_string())),

        "store_memory" => {
            let content = str_param(params, "content")?;
            let title = str_param(params, "title")?;
            let memory_type = memory_type_param(params, "memory_type")?;
            let importance = f64_param(params, "importance")?;
            let tags = string_array_param(params, "tags")?;
            let source_type = str_param(params, "source_type")?;
            let project_path = str_param(params, "project_path")?;

            let id = graph
                .store_memory(
                    &content,
                    &title,
                    memory_type,
                    importance,
                    &tags,
                    &source_type,
                    &project_path,
                )
                .map_err(|e| e.to_string())?;

            Ok(Value::String(id))
        }

        "get_memory" => {
            let id = str_param(params, "id")?;
            let mem = graph.get_memory(&id).map_err(|e| e.to_string())?;
            serde_json::to_value(mem).map_err(|e| e.to_string())
        }

        "update_memory" => {
            let id = str_param(params, "id")?;
            let content = opt_str_param(params, "content");
            let tags = opt_string_array_param(params, "tags");
            graph
                .update_memory(&id, content.as_deref(), tags.as_deref())
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "update_importance" => {
            let id = str_param(params, "id")?;
            let importance = f64_param(params, "importance")?;
            graph
                .update_importance(&id, importance)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "delete_memory" => {
            let id = str_param(params, "id")?;
            graph.delete_memory(&id).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "create_relationship" => {
            let from_id = str_param(params, "from_id")?;
            let to_id = str_param(params, "to_id")?;
            let rel_type = rel_type_param(params, "rel_type")?;
            let context = opt_str_param(params, "context");
            let strength = f64_param(params, "strength")?;
            graph
                .create_relationship(&from_id, &to_id, rel_type, context.as_deref(), strength)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "recall_memories" => {
            let query = str_param(params, "query")?;
            let limit = usize_param(params, "limit")?;
            let mems = graph
                .recall_memories(&query, limit)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(mems).map_err(|e| e.to_string())
        }

        "search_memories" => {
            let filter: SearchFilter = params
                .get("filter")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let mems = graph.search_memories(&filter).map_err(|e| e.to_string())?;
            serde_json::to_value(mems).map_err(|e| e.to_string())
        }

        "list_recent" => {
            let limit = usize_param(params, "limit")?;
            let mems = graph.list_recent(limit).map_err(|e| e.to_string())?;
            serde_json::to_value(mems).map_err(|e| e.to_string())
        }

        "memory_count" => {
            let count = graph.memory_count().map_err(|e| e.to_string())?;
            Ok(Value::Number(serde_json::Number::from(count as u64)))
        }

        "clear_all" => {
            let count = graph.clear_all().map_err(|e| e.to_string())?;
            Ok(Value::Number(serde_json::Number::from(count as u64)))
        }

        "get_related_memories" => {
            let id = str_param(params, "id")?;
            let depth = params
                .get("depth")
                .and_then(Value::as_u64)
                .map(|v| v as u32)
                .ok_or_else(|| "missing or invalid u32 param: depth".to_string())?;
            let mems = graph
                .get_related_memories(&id, depth)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(mems).map_err(|e| e.to_string())
        }

        other => Err(format!("unknown method: {other}")),
    }
}

// ── parameter extraction helpers ───────────────────────────────

fn str_param(params: &Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| format!("missing or invalid string param: {key}"))
}

fn opt_str_param(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_str() })
        .map(String::from)
}

fn f64_param(params: &Value, key: &str) -> Result<f64, String> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing or invalid f64 param: {key}"))
}

fn usize_param(params: &Value, key: &str) -> Result<usize, String> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .ok_or_else(|| format!("missing or invalid usize param: {key}"))
}

fn string_array_param(params: &Value, key: &str) -> Result<Vec<String>, String> {
    let arr = params
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing or invalid array param: {key}"))?;
    arr.iter()
        .map(|v| {
            v.as_str()
                .map(String::from)
                .ok_or_else(|| format!("non-string element in {key}"))
        })
        .collect()
}

fn opt_string_array_param(params: &Value, key: &str) -> Option<Vec<String>> {
    params
        .get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_array() })
        .and_then(|arr| {
            arr.iter()
                .map(|v| v.as_str().map(String::from))
                .collect::<Option<Vec<_>>>()
        })
}

fn memory_type_param(params: &Value, key: &str) -> Result<MemoryType, String> {
    let s = str_param(params, key)?;
    // Support both enum variant names ("Fact") and stored format ("fact")
    MemoryType::from_str_opt(&s)
        .or_else(|| MemoryType::from_str_opt(&s.to_lowercase()))
        .ok_or_else(|| format!("invalid memory type: {s}"))
}

fn rel_type_param(params: &Value, key: &str) -> Result<RelType, String> {
    let s = str_param(params, key)?;
    // Support both enum variant names ("RelatedTo") and stored format ("related_to")
    RelType::from_str_opt(&s)
        .or_else(|| {
            // Convert PascalCase to snake_case for lookup
            let snake = pascal_to_snake(&s);
            RelType::from_str_opt(&snake)
        })
        .ok_or_else(|| format!("invalid relationship type: {s}"))
}

/// Simple PascalCase to snake_case converter for enum variant matching.
fn pascal_to_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}
