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
//!
//! Dispatch itself is *blocking*: the graph's write path now goes through the
//! `archon-cozo` guard, whose SQLITE_BUSY retry loop parks the calling thread
//! with `thread::sleep`. Running that directly on a Tokio worker would stall
//! the runtime, so every request is handed to `spawn_blocking`.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::{debug, error, warn};

use crate::board::NewBoardItem;
use crate::graph::MemoryGraph;
use crate::protocol::{Request, make_response_err, make_response_ok};
use crate::types::{MemoryError, SearchFilter};

mod params;

use params::{
    board_status_array_param, board_status_param, f64_param, memory_type_param, opt_str_param,
    opt_string_array_param, rel_type_param, str_param, string_array_param, usize_param,
};

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

            let dispatch_graph = Arc::clone(&graph);
            let method = req.method.clone();
            let params = req.params.clone();
            let dispatched =
                tokio::task::spawn_blocking(move || dispatch(&dispatch_graph, &method, &params))
                    .await;
            let resp = match dispatched {
                Ok(Ok(val)) => make_response_ok(req.id, val),
                Ok(Err(msg)) => make_response_err(req.id, msg),
                Err(join_error) => {
                    make_response_err(req.id, format!("memory dispatch task failed: {join_error}"))
                }
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

        "store_memory_with_id_outcome" => {
            let id = str_param(params, "id")?;
            let content = str_param(params, "content")?;
            let title = str_param(params, "title")?;
            let memory_type = memory_type_param(params, "memory_type")?;
            let importance = f64_param(params, "importance")?;
            let tags = string_array_param(params, "tags")?;
            let source_type = str_param(params, "source_type")?;
            let project_path = str_param(params, "project_path")?;
            let outcome = graph
                .store_memory_with_id_outcome(
                    &id,
                    &content,
                    &title,
                    memory_type,
                    importance,
                    &tags,
                    &source_type,
                    &project_path,
                )
                .map_err(|error| error.to_string())?;
            serde_json::to_value(outcome).map_err(|error| error.to_string())
        }

        "store_memory_with_id" => {
            let id = str_param(params, "id")?;
            let content = str_param(params, "content")?;
            let title = str_param(params, "title")?;
            let memory_type = memory_type_param(params, "memory_type")?;
            let importance = f64_param(params, "importance")?;
            let tags = string_array_param(params, "tags")?;
            let source_type = str_param(params, "source_type")?;
            let project_path = str_param(params, "project_path")?;
            let memory = graph
                .store_memory_with_id(
                    &id,
                    &content,
                    &title,
                    memory_type,
                    importance,
                    &tags,
                    &source_type,
                    &project_path,
                )
                .map_err(|error| error.to_string())?;
            serde_json::to_value(memory).map_err(|error| error.to_string())
        }

        "get_memory" => {
            let id = str_param(params, "id")?;
            let mem = graph.get_memory(&id).map_err(|e| e.to_string())?;
            serde_json::to_value(mem).map_err(|e| e.to_string())
        }

        "inspect_memory" => {
            let id = str_param(params, "id")?;
            let mem = graph.read_memory(&id).map_err(|e| e.to_string())?;
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

        "apply_importance_delta" => {
            let id = str_param(params, "id")?;
            let delta = f64_param(params, "delta")?;
            let provenance_id = str_param(params, "provenance_id")?;
            let memory = graph
                .apply_importance_delta(&id, delta, &provenance_id)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(memory).map_err(|error| error.to_string())
        }

        "reconcile_importance_trend" => {
            let id = str_param(params, "id")?;
            let previous_importance = f64_param(params, "previous_importance")?;
            let memory = graph
                .reconcile_importance_trend(&id, previous_importance)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(memory).map_err(|error| error.to_string())
        }

        "has_importance_application" => {
            let memory_id = str_param(params, "memory_id")?;
            let provenance_id = str_param(params, "provenance_id")?;
            let applied = graph
                .has_importance_application(&memory_id, &provenance_id)
                .map_err(|error| error.to_string())?;
            Ok(Value::Bool(applied))
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

        "embedding_neighbours" => {
            let memory_id = str_param(params, "memory_id")?;
            let top_k = usize_param(params, "top_k")?;
            // `null` on the wire is "this store has no vector search", which is
            // a different answer from an empty list and the reason the client
            // deserializes into an `Option`.
            let neighbours =
                crate::access::MemoryTrait::embedding_neighbours(graph, &memory_id, top_k)
                    .map_err(|error| error.to_string())?;
            serde_json::to_value(neighbours).map_err(|error| error.to_string())
        }

        // ── task board ─────────────────────────────────────────
        //
        // On the RPC surface for the same reason every memory operation is:
        // CozoDB admits one writer, so every Archon process after the first
        // reaches the graph through here. A board only reachable in-process
        // would be a board no subagent in a second process could hand work to.
        "create_board_item" => {
            let item: NewBoardItem = params
                .get("item")
                .ok_or_else(|| "missing param: item".to_string())
                .and_then(|value| {
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
                })?;
            let created = graph
                .create_board_item(&item)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(created).map_err(|error| error.to_string())
        }

        "get_board_item" => {
            let id = str_param(params, "id")?;
            let item = graph.get_board_item(&id).map_err(|e| e.to_string())?;
            serde_json::to_value(item).map_err(|e| e.to_string())
        }

        // Takes no parameters, and is on the wire regardless: a client that
        // could not enumerate runs would have to be told one, which is exactly
        // the handle a reader outside the run does not have.
        "list_board_runs" => {
            let runs = graph.list_board_runs().map_err(|e| e.to_string())?;
            serde_json::to_value(runs).map_err(|e| e.to_string())
        }

        "list_board_items_by_run" => {
            let run_id = str_param(params, "run_id")?;
            let statuses = board_status_array_param(params, "statuses")?;
            let items = graph
                .list_board_items_by_run(&run_id, &statuses)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(items).map_err(|e| e.to_string())
        }

        "claim_board_item" => {
            let id = str_param(params, "id")?;
            let agent_id = str_param(params, "agent_id")?;
            let update = graph
                .claim_board_item(&id, &agent_id)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(update).map_err(|e| e.to_string())
        }

        "release_board_claim" => {
            let id = str_param(params, "id")?;
            let update = graph.release_board_claim(&id).map_err(|e| e.to_string())?;
            serde_json::to_value(update).map_err(|e| e.to_string())
        }

        "set_board_item_status" => {
            let id = str_param(params, "id")?;
            let from = board_status_param(params, "from")?;
            let to = board_status_param(params, "to")?;
            let update = graph
                .set_board_item_status(&id, from, to)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(update).map_err(|e| e.to_string())
        }

        // Declining is on the wire as its own method rather than as a `to` on
        // `set_board_item_status`, so that the requirement travels: a remote
        // caller cannot reach `declined` through a shape with no reason in it.
        "decline_board_item" => {
            let id = str_param(params, "id")?;
            let from = board_status_param(params, "from")?;
            let reason = str_param(params, "reason")?;
            let update = graph
                .decline_board_item(&id, from, &reason)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(update).map_err(|e| e.to_string())
        }

        "board_item_history" => {
            let id = str_param(params, "id")?;
            let events = graph.board_item_history(&id).map_err(|e| e.to_string())?;
            serde_json::to_value(events).map_err(|e| e.to_string())
        }

        other => Err(format!("unknown method: {other}")),
    }
}
