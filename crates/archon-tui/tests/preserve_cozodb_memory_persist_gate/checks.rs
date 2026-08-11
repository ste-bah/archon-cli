//! The post-restart persistence checks for
//! `preserve_cozodb_memory_persist_gate`, factored out so the gate and
//! its negative control consult the *same* code.
//!
//! This is the shape `scripts/check-preserve-invariants.sh` and
//! `scripts/check-r0-entry-gate.sh` already use: one `run_checks` that
//! the real run and `--self-test` both call, the self-test pointing it
//! at a deliberately broken input. Sharing the checks is the point — a
//! control that re-implemented them would only prove the copy works,
//! and would drift from the gate the first time either changed.
//!
//! ## Why a returned verdict and not `assert!`
//!
//! The checks report what they found instead of panicking. The gate
//! demands [`RestartFinding::Persisted`] and panics with the returned
//! detail otherwise, so every assertion the gate had is still enforced,
//! with its original message and the `REQ-FOR-PRESERVE-D8` + `CozoDB`
//! failure-message contract intact. The control demands
//! [`RestartFinding::Absent`]. Neither verdict can be satisfied by the
//! other's evidence, which is what makes the control worth having.
//!
//! ## Why `Absent` is narrow
//!
//! A control that accepted "an error occurred" would be satisfied by a
//! missing parent directory, a permissions fault or a poisoned lock —
//! none of which say anything about persistence, and all of which
//! would let the control pass while the gate had quietly stopped being
//! able to observe the memory at all.
//!
//! `Absent` is therefore returned only when the store came back
//! *healthy* and specifically reported no such memory: the reopen
//! succeeded, the schema and HNSW index initialised, `memory_count`
//! answered 0 rather than erroring, `get_memory` returned
//! `MemoryError::NotFound` carrying the exact id that was stored, and
//! both recall paths returned empty. Anything else is
//! [`RestartFinding::Broken`] — including a mixed state where
//! `get_memory` reports NotFound but keyword recall still yields the
//! row, which is a real defect and must not read as a clean deletion.

use std::path::Path;
use std::sync::Arc;

use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use archon_memory::{MemoryGraph, MemoryTrait};

use super::{HashEmbedProvider, PAYLOAD_CONTENT, PAYLOAD_TITLE, fail_msg};

/// Keyword-path recall probe. A substring of the stored content, so the
/// non-vector half of `hybrid_search` has something to match on.
const KEYWORD_QUERY: &str = "preserve";

/// HNSW vector-path recall probe. Deliberately *not* a substring of the
/// stored content: it shares tokens but not a literal span, so a
/// keyword match cannot satisfy it and the nearest-neighbour query has
/// to do the work.
const VECTOR_QUERY: &str = "archon-cli restart survive";

/// How many hits to ask each recall path for. Larger than the one row
/// the gate stores, so "not returned" means absent rather than ranked
/// out.
const RECALL_LIMIT: usize = 5;

/// What [`check_after_restart`] observed.
#[derive(Debug)]
pub(super) enum RestartFinding {
    /// Every REQ-FOR-PRESERVE-D8 persistence check passed: the store
    /// reopened, and the memory came back intact through the direct,
    /// keyword and HNSW paths.
    Persisted,

    /// The store reopened healthily and reported no such memory. This
    /// is what the negative control expects to see once the CozoDB file
    /// has been deleted between instantiations.
    Absent(String),

    /// A check failed for some reason other than the memory being
    /// cleanly absent. Fails the gate and the control alike.
    Broken(String),
}

/// Reopen the store at `db_path` and ask it, four ways, whether
/// `stored_id` survived.
///
/// Takes no position on which answer is correct — that is the caller's
/// job, and it is the only difference between the gate and its control.
pub(super) fn check_after_restart(db_path: &Path, stored_id: &str) -> RestartFinding {
    // If the file lock leaked on drop, the reopen fails here with a
    // sqlite "database is locked" error. Surfaced as INV-PRESERVE-003
    // per spec line 40. Note this is `Broken`, never `Absent`: a store
    // that will not open has told us nothing about what is in it.
    let graph = match MemoryGraph::open(db_path) {
        Ok(graph) => graph,
        Err(error) => {
            return RestartFinding::Broken(format!(
                "INV-PRESERVE-003 violated: file lock leaked — {} (underlying: {error})",
                fail_msg("CozoDB could not be re-opened after first-instance drop")
            ));
        }
    };

    // Reattach the same deterministic provider so HNSW queries use the
    // same embedding space as the stored vectors.
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(HashEmbedProvider::new());
    if let Err(error) = graph.set_embedding_provider(provider) {
        return RestartFinding::Broken(fail_msg(&format!(
            "phase-2 set_embedding_provider (HNSW re-init) failed: {error}"
        )));
    }

    // Post-restart contract surface: `&dyn MemoryTrait`, the trait
    // REQ-FOR-PRESERVE-D8 names. Routing through it (rather than the
    // inherent `MemoryGraph` methods) is what breaks the gate if the
    // trait surface is ever narrowed.
    let memory: &dyn MemoryTrait = &graph;

    let count = match memory.memory_count() {
        Ok(count) => count,
        Err(error) => {
            return RestartFinding::Broken(fail_msg(&format!(
                "phase-2 memory_count failed: {error}"
            )));
        }
    };

    // Direct get by id — the narrowest retrieval path, no ranking
    // heuristics. Held as a `Result` rather than unwrapped because its
    // error *is* one of the two signals being classified below.
    let recovered = memory.get_memory(stored_id);

    let keyword = match memory.recall_memories(KEYWORD_QUERY, RECALL_LIMIT) {
        Ok(hits) => hits,
        Err(error) => {
            return RestartFinding::Broken(fail_msg(&format!(
                "recall_memories (keyword) failed: {error}"
            )));
        }
    };

    // `recall_memories` routes through `hybrid_search` when a provider
    // is attached, which calls `vector_search::search_similar` — the
    // HNSW nearest-neighbour entrypoint, and the "issue a
    // vector-similarity search" requirement from spec line 44.
    let vector = match memory.recall_memories(VECTOR_QUERY, RECALL_LIMIT) {
        Ok(hits) => hits,
        Err(error) => {
            return RestartFinding::Broken(fail_msg(&format!(
                "recall_memories (hybrid/vector via HNSW) failed: {error}"
            )));
        }
    };

    match recovered {
        Ok(found) => classify_present(&found, count, &keyword, &vector, stored_id),
        Err(MemoryError::NotFound(ref missing)) if missing == stored_id => {
            classify_absent(count, &keyword, &vector, stored_id)
        }
        Err(error) => RestartFinding::Broken(fail_msg(&format!(
            "phase-2 get_memory(id={stored_id}) failed for a reason other than absence: {error}"
        ))),
    }
}

/// The memory came back. Every field and both recall paths must agree,
/// or the store is `Broken` rather than persisted.
fn classify_present(
    found: &archon_memory::types::Memory,
    count: usize,
    keyword: &[archon_memory::types::Memory],
    vector: &[archon_memory::types::Memory],
    stored_id: &str,
) -> RestartFinding {
    if count != 1 {
        return RestartFinding::Broken(fail_msg(&format!(
            "memory retrievable by id but memory_count is {count}, expected 1"
        )));
    }
    if found.content != PAYLOAD_CONTENT {
        return RestartFinding::Broken(fail_msg(
            "memory content mismatch after restart (byte-for-byte)",
        ));
    }
    if found.title != PAYLOAD_TITLE {
        return RestartFinding::Broken(fail_msg("memory title mismatch after restart"));
    }
    if !found.tags.iter().any(|tag| tag == "preserve-d8") {
        return RestartFinding::Broken(fail_msg(
            "memory tags mismatch after restart (preserve-d8 tag missing)",
        ));
    }
    if !keyword.iter().any(|hit| hit.id == stored_id) {
        return RestartFinding::Broken(fail_msg(
            "memory not retrievable after restart (keyword recall missed stored memory)",
        ));
    }
    if !vector.iter().any(|hit| hit.id == stored_id) {
        return RestartFinding::Broken(fail_msg(
            "memory not retrievable after restart (HNSW vector recall missed stored memory)",
        ));
    }
    RestartFinding::Persisted
}

/// `get_memory` reported NotFound for the stored id. That alone is not
/// enough: the rest of the store must agree it is empty, otherwise the
/// id lookup and the recall paths disagree and something is wrong that
/// is not a clean deletion.
fn classify_absent(
    count: usize,
    keyword: &[archon_memory::types::Memory],
    vector: &[archon_memory::types::Memory],
    stored_id: &str,
) -> RestartFinding {
    if count != 0 {
        return RestartFinding::Broken(fail_msg(&format!(
            "get_memory(id={stored_id}) reported NotFound but memory_count is {count}, not 0"
        )));
    }
    if !keyword.is_empty() || !vector.is_empty() {
        return RestartFinding::Broken(fail_msg(&format!(
            "get_memory(id={stored_id}) reported NotFound but recall still returned \
             {} keyword and {} vector hit(s)",
            keyword.len(),
            vector.len()
        )));
    }
    RestartFinding::Absent(format!(
        "store reopened healthily and reports no memory {stored_id}: \
         count=0, get_memory=NotFound, keyword and HNSW recall both empty"
    ))
}
