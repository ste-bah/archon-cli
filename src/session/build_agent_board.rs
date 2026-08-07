//! The task board handle for the non-interactive session modes.
//!
//! `--print` and `--headless` build their agent through `build_agent.rs`, which
//! never opened memory. `BoardHandle::Global` therefore resolved to nothing and
//! every board call in those modes answered "the task board is unavailable"
//! (#137). The four board tools are in `DEFAULT_TOOLS`, so the model is offered
//! them, tries them, and always fails: the TUI — the one caller of
//! `install_board_access` — was the only surface where the board existed at all.

use archon_core::config::ArchonConfig;

/// Open this session's memory and put a real handle behind the board tools.
///
/// The body is `crate::command::board_access`, shared with the standalone
/// workflow path since #146; what stays here is which consequence a print run
/// actually suffers when memory will not open. The two properties that separate
/// a working board from a silent one — the paths come from
/// `config.memory.db_path`, and the open goes through the election rather than
/// `MemoryGraph::open` — are documented and enforced there, in the one place
/// they can no longer drift apart from each other.
pub(in crate::session) async fn install_session_board_access(config: &ArchonConfig) {
    crate::command::board_access::install_process_board_access(
        config,
        "board tools will report the board as offline",
    )
    .await;
}
