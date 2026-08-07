//! Opening *the configured* memory store: one path resolution, one election,
//! one embedding provider.
//!
//! Four entry points open memory — the TUI (`interactive_bootstrap`), `--print`
//! and `--headless` (`build_agent_board`), a standalone `archon workflow`
//! (`workflow_live_board`) and the `archon memory` subcommands (`memory_cli`) —
//! and until #146 each did the same three steps in its own copy. The copies
//! were not decorative: while the workflow one was being written, another change
//! added `intra_threads` to [`EmbeddingConfig`], and that copy had to be
//! hand-patched to match. The next field added would have been missed in one to
//! three of them, and the symptom — one session embedding differently from
//! another against the same database — leaves no error behind.
//!
//! Two properties of the sequence are load-bearing and both are easy to lose in
//! a rewrite.
//!
//! **The open goes through the election**, never `MemoryGraph::open`. CozoDB
//! admits one writer and nothing below it enforces that: a second raw open of a
//! database a live session holds returns in milliseconds with correct-looking
//! rows (#134), so a bypass is undetectable at runtime rather than loud.
//! [`open_memory_with_db_path`] reads `memory.port` and connects as a client
//! when a server answers, and only otherwise takes the lock and becomes one.
//!
//! **The paths come from the spec**, which comes from `config.memory.db_path`.
//! Resolving anything else — the default data dir, a guessed `memory.db` —
//! hands the caller a private store that accepts writes nobody ever reads. That
//! fails silently, which is strictly worse than the honest error it replaces.
//!
//! What this function deliberately does *not* decide is whether the failure is
//! fatal. Its callers disagree, correctly: a print run does useful work with no
//! board and degrades, the TUI cannot hand a session a memory it does not have
//! and aborts. So the error comes back and each caller keeps the disposition it
//! already had.

use std::path::PathBuf;

use tracing::{info, warn};

use crate::embedding::{EmbeddingConfig, create_provider};
use crate::types::MemoryError;

use super::{MemoryAccess, open_memory_with_db_path, resolve_memory_paths};

/// Everything in `[memory]` that opening the store depends on.
///
/// A struct rather than four positional arguments so that a field added to
/// [`EmbeddingConfig`] reaches every entry point by being filled in once, in
/// `archon_core::config::MemoryConfig::open_spec`. That is the whole point of
/// the type: `archon-memory` cannot see `ArchonConfig` (archon-core depends on
/// this crate, not the reverse), so the config → spec mapping lives there and
/// the open lives here.
pub struct MemoryOpenSpec {
    /// `config.memory.db_path` verbatim, resolved by [`resolve_memory_paths`].
    pub db_path: Option<String>,
    pub embedding: EmbeddingConfig,
}

impl MemoryOpenSpec {
    /// The data dir and database this spec names.
    ///
    /// Public because a caller that wants to name the database in a *failure*
    /// message has no [`OpenedMemory`] to read it from. Same pure function the
    /// open uses, so the two can never name different files.
    pub fn resolve_paths(&self) -> (PathBuf, PathBuf) {
        resolve_memory_paths(self.db_path.as_deref())
    }
}

/// What happened to the embedding provider, for the one caller that cares.
///
/// Reported rather than only logged because `archon memory reindex` must refuse
/// to run without a provider — re-embedding with no embedder is not a partial
/// success, it is a no-op that prints a completion. Every other caller treats
/// all three arms the same and just carries on.
pub enum EmbeddingSetup {
    /// Built and attached; the hybrid alpha is applied.
    Attached,
    /// `Remote` access: no local graph to configure, and the process on the
    /// other end of the socket already did it.
    NotApplicable,
    /// Keyword-only search. Carries the message a caller should fail with.
    Unavailable(String),
}

/// An opened store, and the paths it was actually opened at.
pub struct OpenedMemory {
    pub access: MemoryAccess,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub embedding: EmbeddingSetup,
}

/// Resolve the configured paths, open through the election, attach embeddings.
///
/// See the module docs for why each of those three is not substitutable.
pub async fn open_configured_memory(spec: &MemoryOpenSpec) -> Result<OpenedMemory, MemoryError> {
    let (data_dir, db_path) = spec.resolve_paths();
    let access = open_memory_with_db_path(&data_dir, &db_path).await?;
    let embedding = attach_embedding_provider(&access, &spec.embedding);
    Ok(OpenedMemory {
        access,
        data_dir,
        db_path,
        embedding,
    })
}

/// Give the graph its embedding provider, when this process won the election.
///
/// Not the caller's business individually, but a consequence of opening at all:
/// whichever process becomes the server serves every process that starts while
/// it lives, and a server with no provider serves keyword-only search to all of
/// them and stores no vectors for what it writes. So this runs wherever memory
/// is opened, not only where the opener intends to search.
fn attach_embedding_provider(access: &MemoryAccess, cfg: &EmbeddingConfig) -> EmbeddingSetup {
    let Some(graph) = access.graph() else {
        return EmbeddingSetup::NotApplicable;
    };
    let provider = match create_provider(cfg) {
        Ok(provider) => provider,
        Err(error) => {
            warn!(%error, "embedding provider unavailable, using keyword-only search");
            return EmbeddingSetup::Unavailable(format!(
                "failed to create embedding provider: {error}"
            ));
        }
    };
    if let Err(error) = graph.set_embedding_provider(provider) {
        warn!(%error, "failed to initialise embedding schema");
        return EmbeddingSetup::Unavailable(format!(
            "failed to attach embedding provider to graph: {error}"
        ));
    }
    graph.set_hybrid_alpha(cfg.hybrid_alpha);
    info!(
        provider = %cfg.provider,
        alpha = cfg.hybrid_alpha,
        "semantic embedding provider active"
    );
    EmbeddingSetup::Attached
}
