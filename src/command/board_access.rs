//! Opening memory for the sake of the process-global task board.
//!
//! Two entry points install a board without otherwise needing a memory handle —
//! `--print`/`--headless` (`src/session/build_agent_board.rs`, #137) and a
//! standalone `archon workflow` (`src/command/workflow_live_board.rs`, #142) —
//! and both want the same disposition: open the configured store through the
//! election, and if it will not open, log once and leave the board offline.
//! This is that shared body. The TUI does not use it: `interactive_bootstrap`
//! needs the `MemoryAccess` itself for the session, so it opens through
//! `open_configured_memory` directly and installs the handle it already holds.
//!
//! The open lives in `archon-memory`; the *install* cannot, because
//! `archon-tools` depends on `archon-memory` and not the reverse. That is why
//! #146 ended up as an open helper plus a separate install rather than an open
//! with a board flag — the layering decided it, not taste.

use std::sync::Arc;

use archon_core::config::ArchonConfig;

/// Open this process's configured memory and put a real handle behind the four
/// board tools.
///
/// `offline_consequence` completes the warning when memory will not open, and
/// the two callers say different true things: a print session loses its board
/// tools, a workflow run additionally fails its drain gate. Passing it keeps
/// both messages exact instead of flattening them to whichever was shorter.
///
/// **Nothing here is fatal.** Both callers do useful work without a board, so a
/// memory that will not open is logged once and left uninstalled; a later board
/// call gets the same "unavailable" message it always did, which is a truthful
/// answer rather than a session that refused to start. What that costs is
/// deliberate and paid elsewhere: `workflow_board_drain.rs` refuses a run whose
/// board it could not read rather than passing it in silence.
pub(crate) async fn install_process_board_access(config: &ArchonConfig, offline_consequence: &str) {
    // The three install sites are mutually exclusive by control flow —
    // `main_modes` exits the process from the print and headless arms before the
    // interactive path is reachable, and `main` returns straight out of
    // `handle_subcommand_if_present` for a workflow run. This guard is not for
    // that; it is for the test binary, where every one of those sites is
    // reachable from the same process and a second open against a database this
    // process already holds is exactly what the election exists to prevent.
    // Resolving first costs a `OnceLock` read and removes the question.
    //
    // It also makes the second call a no-op rather than a wasted open:
    // `install_board_access` keeps the first handle regardless, so an open whose
    // result could only be discarded is not worth performing.
    if archon_tools::board::BoardHandle::Global.resolve().is_ok() {
        tracing::debug!("board access already installed in this process; not opening memory again");
        return;
    }
    let spec = config.memory.open_spec();
    let opened = match archon_memory::open_configured_memory(&spec).await {
        Ok(opened) => opened,
        Err(error) => {
            let (_, db_path) = spec.resolve_paths();
            tracing::warn!(
                %error,
                db_path = %db_path.display(),
                "memory unavailable: {offline_consequence}"
            );
            return;
        }
    };
    let db_path = opened.db_path.clone();
    archon_tools::board::install_board_access(
        Arc::new(opened.access) as Arc<dyn archon_memory::board::BoardAccess>
    );
    tracing::info!(db_path = %db_path.display(), "task board available");
}
