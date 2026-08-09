//! Electing, caching, and retiring the board reader.
//!
//! A child of `board.rs` rather than more of it, and the split is by job:
//! everything there projects board rows onto the wire, and this is the separate
//! problem of having a store to read at all. The endpoints are the same shape
//! whichever arm the election returns, and this module is the only place that
//! difference exists.
//!
//! Reaching the board is what this has to arrange, and it cannot do what its
//! neighbours do. `inspect.rs` reuses `WebRuntimeHandles::memory`, the handle the
//! host session already has open, and falls back to opening the file only when
//! there is no handle — the right shape, and unavailable here: that handle is an
//! `Arc<dyn MemoryTrait>`, the board is deliberately NOT on `MemoryTrait` (see
//! `archon_memory::board`, which keeps `BoardAccess` separate so seventeen mock
//! implementations do not grow board methods), and no `BoardAccess` can be
//! recovered from a `MemoryTrait` object.
//!
//! WHAT IT MUST NOT DO INSTEAD IS OPEN THE DATABASE ITSELF. CozoDB admits one
//! writer, and in attached mode the host session already holds it on this very
//! file, so a direct `MemoryGraph::open` here is a second writer against a locked
//! database. `open_memory_with_db_path` is the substitute for the handle this
//! module cannot borrow: the singleton election reads `memory.port`, connects as
//! a client when a server answers, and only otherwise takes `memory.lock` and
//! opens the graph. `MemoryAccess` implements `BoardAccess`, which is what makes
//! that work — the server is `Direct` when it is the only process and `Remote`
//! over TCP when the TUI owns the writer, decided by the same code path as every
//! other entry point in Archon.

use std::sync::Arc;

use archon_memory::access::MemoryAccess;
use archon_memory::board::BoardAccess;
use archon_memory::open_memory_with_db_path;

use crate::web::WebRuntimePaths;

/// The board reader, elected on first use and held until it stops working.
///
/// Three rules, and the third is why this is a mutex over an `Option` rather
/// than the `OnceCell` it used to be.
///
/// **A failed election is not cached.** The memory server can be down when the
/// first request arrives and up a minute later, so a slot that recorded the
/// failure would report an unreachable board for the life of the process. The
/// `?` below leaves the slot empty, and the next request tries again.
///
/// **A successful election is cached.** Re-running it per request would re-read
/// the port file and reconnect on every poll, and in the `Direct` branch would
/// try to take a lock this process already holds.
///
/// **A cached election is dropped once it fails.** This is the case a
/// write-once cell cannot express. The elected arm is `Remote` whenever another
/// process owns the writer, and that server exits when its process does; the
/// socket is then closed and every read fails with `IO error: Broken pipe`,
/// forever, while a live server for the same database may be listening on a new
/// port the whole time. Observed live: a web process holding a CLOSED socket to
/// a server that had exited hours earlier, `memory.port` naming a healthy one,
/// and every board request returning 500. So [`Self::invalidate`] clears the
/// slot on a read failure and the next request re-elects — bounded, because it
/// only happens after a request has already failed, never on the happy path.
///
/// It holds the elected `MemoryAccess` rather than an `Arc<dyn BoardAccess>`
/// because which arm was elected is the thing worth asserting: a second direct
/// open of a held database does not hang and does not error — it silently
/// succeeds and even reads correctly — so no behavioural test can tell the two
/// apart. The mode is the only observable difference, and `elected` exists so a
/// test can pin it.
#[derive(Clone, Default)]
pub struct WebBoardStore {
    opened: Arc<tokio::sync::Mutex<Option<Arc<MemoryAccess>>>>,
}

impl WebBoardStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The board reader, or `None` when there is no database yet.
    ///
    /// `None` is a real answer rather than a failure, and it is checked before
    /// the election because `open_memory_with_db_path` creates what it cannot
    /// find. Looking at a dashboard must not be what brings a memory database
    /// into existence.
    ///
    /// Reaches the whole `web` module, not just `board`, because `board_activity`
    /// resolves the same store for its own endpoint — the reach `pub(super)` had
    /// before this module was split out of `board.rs`.
    pub(in crate::web) async fn resolve(
        &self,
        paths: &WebRuntimePaths,
    ) -> Result<Option<Arc<dyn BoardAccess>>, String> {
        let mut slot = self.opened.lock().await;
        if let Some(access) = slot.as_ref() {
            return Ok(Some(Arc::clone(access) as Arc<dyn BoardAccess>));
        }
        if !paths.memory_db.exists() {
            return Ok(None);
        }
        // Still "store only on success": the `?` below leaves the slot empty on
        // a failed election, so the next request tries again.
        let access = {
            let elect = async {
                // The election coordinates on `memory.port` and `memory.lock` in
                // the data directory, not on the database file. That directory
                // is the database's parent in every case `resolve_memory_paths`
                // produces — including a configured path, where it derives the
                // data dir from the `.db` file's parent — so taking it from here
                // lands on the same files the session is coordinating through.
                //
                // A bare relative filename has `Some("")` as its parent, not
                // `None`, and coordinating in "" would put the port file
                // somewhere neither process agrees on.
                let data_dir = paths
                    .memory_db
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(&paths.archon_data);
                let access = open_memory_with_db_path(data_dir, &paths.memory_db)
                    .await
                    .map_err(|error| format!("board: could not reach memory: {error}"))?;
                Ok::<Arc<MemoryAccess>, String>(Arc::new(access))
            };
            elect.await?
        };
        *slot = Some(Arc::clone(&access));
        Ok(Some(access as Arc<dyn BoardAccess>))
    }

    /// Drop the cached election so the next request re-elects.
    ///
    /// Called when a board read fails. It cannot tell a dead connection from a
    /// transient database error and does not try: re-electing after a failure
    /// costs one port-file read, and guessing wrong in the other direction
    /// costs every subsequent request.
    ///
    /// Same reach as [`Self::resolve`], and for the same reason: the activity
    /// endpoint in `board_activity` retires a dead handle too.
    pub(in crate::web) async fn invalidate(&self) {
        *self.opened.lock().await = None;
    }

    /// Which arm the election returned, once it has run.
    ///
    /// `pub(super)` so the election tests can reach it from `board`'s test tree,
    /// which is where they were written; private here would confine it to this
    /// module.
    #[cfg(test)]
    pub(super) async fn elected(&self) -> Option<Arc<MemoryAccess>> {
        self.opened.lock().await.as_ref().map(Arc::clone)
    }
}
