//! The task board handle for a standalone `archon workflow` run.
//!
//! `archon workflow` is a subcommand: `main_modes::handle_subcommand_if_present`
//! dispatches it and returns from `main`, so the process never builds a session
//! and neither `src/session/build_agent_board.rs` nor `interactive_bootstrap`
//! ever runs. `BoardHandle::Global` therefore resolved to nothing here — the
//! four board tools answered *"the task board is unavailable"* to every stage
//! subagent, and, worse, the lifecycle's drain gate read `None` and passed the
//! run without inspecting anything (#142).
//!
//! The second half is why this file exists rather than a bug report. A gate that
//! reports a clean run because it cannot see the board is worse than no gate at
//! all: it produces the exact record an enforced run produces. The companion
//! change is in `workflow_board_drain.rs`, which no longer lets an absent board
//! read as an empty one.

use std::sync::Arc;

use archon_core::config::ArchonConfig;

/// Open this run's memory and put a real handle behind the board tools.
///
/// Mirrors `src/session/build_agent_board.rs`, and the two things it mirrors are
/// exactly what separates a working board from a silent one.
///
/// **The paths come from the config.** `config.memory.db_path` names the
/// database the rest of this machine's world reads and writes;
/// `resolve_memory_paths` is what turns it into a data dir and a db path.
/// Resolving anything else — the default data dir, a guessed `memory.db` —
/// would hand the run a private board that accepts writes nobody ever reads,
/// and the drain gate would then pass a run whose items landed in the wrong
/// file. Silent, and strictly worse than the honest error it replaces.
///
/// **The open goes through the election**, never `MemoryGraph::open`. CozoDB
/// admits one writer and nothing below it enforces that: a second raw open of a
/// database a live session holds returns in milliseconds with correct-looking
/// rows (#134), so a bypass is undetectable at runtime rather than loud.
/// `open_memory_with_db_path` reads `memory.port` and connects as a client when
/// a server answers, and only otherwise takes the lock and becomes one. A
/// workflow run is the case that makes this matter most — it is normal to start
/// one from a terminal while a TUI session is open in another.
///
/// Nothing here is fatal to the *run*. A workflow does useful work without a
/// board, so a memory that will not open is logged once and left uninstalled.
/// What it is no longer free of is consequence: the run reaches the drain gate
/// with a board port that reports why it could not read, and is refused rather
/// than accepted.
pub(crate) async fn install_workflow_board_access(config: &ArchonConfig) {
    // The three install sites are mutually exclusive by control flow — `main`
    // returns straight out of `handle_subcommand_if_present`, so a process that
    // runs a workflow subcommand reaches neither `build_agent.rs` nor the
    // interactive bootstrap — and `run_live_cli_action` is called once per
    // invocation. This guard is not for that; it is for the test binary, where
    // every one of those sites is reachable from the same process and a second
    // `open_memory_with_db_path` against a database this process already holds
    // is the bypass the election exists to prevent. Resolving first costs a
    // `OnceLock` read and removes the question.
    if archon_tools::board::BoardHandle::Global.resolve().is_ok() {
        tracing::debug!("board access already installed in this process; not opening memory again");
        return;
    }
    let (data_dir, db_path) = archon_memory::resolve_memory_paths(config.memory.db_path.as_deref());
    let access = match archon_memory::open_memory_with_db_path(&data_dir, &db_path).await {
        Ok(access) => access,
        Err(error) => {
            tracing::warn!(
                %error,
                db_path = %db_path.display(),
                "memory unavailable: this workflow run has no task board and its drain gate will refuse it"
            );
            return;
        }
    };
    configure_embeddings(&access, config);
    archon_tools::board::install_board_access(
        Arc::new(access) as Arc<dyn archon_memory::board::BoardAccess>
    );
    tracing::info!(db_path = %db_path.display(), "task board available");
}

/// Give the graph its embedding provider, when this process elected itself the
/// memory server.
///
/// Not board state, but a consequence of opening memory here at all: a CLI
/// workflow run that wins the election serves every process that starts while it
/// lives, and a server with no provider serves keyword-only search to all of
/// them and stores no vectors for what it writes. `Remote` access has no graph
/// to configure and the server on the other end already did it.
fn configure_embeddings(access: &archon_memory::MemoryAccess, config: &ArchonConfig) {
    let Some(graph) = access.graph() else {
        return;
    };
    let embed_cfg = archon_memory::embedding::EmbeddingConfig {
        provider: config.memory.embedding_provider,
        hybrid_alpha: config.memory.hybrid_alpha,
        base_url: config.memory.embedding_base_url.clone(),
        model: config.memory.embedding_model.clone(),
        intra_threads: config.memory.embedding_intra_threads,
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
