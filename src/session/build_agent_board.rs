//! The task board handle for the non-interactive session modes.
//!
//! `--print` and `--headless` build their agent through `build_agent.rs`, which
//! never opened memory. `BoardHandle::Global` therefore resolved to nothing and
//! every board call in those modes answered "the task board is unavailable"
//! (#137). The four board tools are in `DEFAULT_TOOLS`, so the model is offered
//! them, tries them, and always fails: the TUI — the one caller of
//! `install_board_access` — was the only surface where the board existed at all.

use std::sync::Arc;

use archon_core::config::ArchonConfig;

/// Open this session's memory and put a real handle behind the board tools.
///
/// Mirrors `interactive_bootstrap::prepare`, and the two things it mirrors are
/// exactly what separates a working board from a silent one.
///
/// **The paths come from the config.** `config.memory.db_path` names the
/// database the rest of this session's world reads and writes;
/// `resolve_memory_paths` is what turns it into a data dir and a db path.
/// Resolving anything else — the default data dir, a guessed `memory.db` —
/// would hand the run a private board that accepts writes nobody ever reads.
/// That fails silently, which is strictly worse than the honest error it
/// replaces.
///
/// **The open goes through the election**, never `MemoryGraph::open`. CozoDB
/// admits one writer and nothing below it enforces that: a second raw open of a
/// database a live session holds returns in milliseconds with correct-looking
/// rows (#134), so a bypass is undetectable at runtime rather than loud.
/// `open_memory_with_db_path` reads `memory.port` and connects as a client when
/// a server answers, and only otherwise takes the lock and becomes one.
///
/// Nothing here is fatal. A print run does useful work without a board, so a
/// memory that will not open is logged once and left uninstalled; a later board
/// call then gets the same "unavailable" message it always did, which is a
/// truthful answer rather than a session that refused to start.
pub(in crate::session) async fn install_session_board_access(config: &ArchonConfig) {
    let (data_dir, db_path) = archon_memory::resolve_memory_paths(config.memory.db_path.as_deref());
    let access = match archon_memory::open_memory_with_db_path(&data_dir, &db_path).await {
        Ok(access) => access,
        Err(error) => {
            tracing::warn!(
                %error,
                db_path = %db_path.display(),
                "memory unavailable: board tools will report the board as offline"
            );
            return;
        }
    };
    configure_embeddings(&access, config);

    // Installed once per process. `main_modes` exits the process from the print
    // and headless arms before the interactive path can be reached, so this and
    // `interactive_bootstrap`'s install are mutually exclusive rather than two
    // racers for the `OnceLock` — which one wins is never a question that gets
    // asked.
    archon_tools::board::install_board_access(
        Arc::new(access) as Arc<dyn archon_memory::board::BoardAccess>
    );
    tracing::info!(db_path = %db_path.display(), "task board available");
}

/// Give the graph its embedding provider, when this process elected itself the
/// memory server.
///
/// Not board state, but a consequence of opening memory here at all: before
/// this, a non-interactive run never stood for election, and now one that wins
/// serves every process that starts while it lives. A server with no provider
/// serves keyword-only search to all of them and stores no vectors for what it
/// writes. The interactive path configures it for the same reason; `Remote`
/// access has no graph to configure and the server on the other end already
/// did it.
fn configure_embeddings(access: &archon_memory::MemoryAccess, config: &ArchonConfig) {
    let Some(graph) = access.graph() else {
        return;
    };
    let embed_cfg = archon_memory::embedding::EmbeddingConfig {
        provider: config.memory.embedding_provider,
        hybrid_alpha: config.memory.hybrid_alpha,
        base_url: config.memory.embedding_base_url.clone(),
        model: config.memory.embedding_model.clone(),
    };
    match archon_memory::embedding::create_provider(&embed_cfg) {
        Ok(provider) => match graph.set_embedding_provider(provider) {
            Ok(()) => graph.set_hybrid_alpha(embed_cfg.hybrid_alpha),
            Err(error) => tracing::warn!(%error, "failed to initialise embedding schema"),
        },
        Err(error) => {
            tracing::warn!(%error, "embedding provider unavailable, using keyword-only search");
        }
    }
}
