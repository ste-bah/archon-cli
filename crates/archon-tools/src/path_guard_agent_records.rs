//! Refusing a spawned agent's file-tool write into the agent records (#241).
//!
//! A resume restores an agent's confinement from the metadata its spawn
//! wrote under [`crate::agent_records::sessions_root`]. A confined agent whose
//! write roots contained that directory (its home directory as its working
//! directory, or a declared root above it) could rewrite its own record to
//! "no isolation, no roots" and be resumed unconfined. Keeping the record out
//! of every write root cannot be promised, because the caller names the
//! roots. So the rule is on the directory: no spawned agent writes it, however
//! wide its roots are. Only the host writes it.
//!
//! Judged on every spelling of the path the agent named and of what it
//! resolves to, against every spelling of the directory, so `/var` against
//! `/private/var` or a link into the directory does not walk around it.
//!
//! The top-level agent is not refused: it is the host's own session, not an
//! agent a record confines. A shell is not judged here, as with every write
//! root (see `BashTool::description_for`); an agent with an unconfined shell
//! already writes wherever it likes.

use std::path::Path;

use crate::tool::ToolContext;
use crate::workflow_read_guard::spellings;

/// `Err` when a spawned agent's write would land under the agent records.
pub(crate) fn ensure_not_agent_record(
    requested: &Path,
    resolved: &Path,
    ctx: &ToolContext,
) -> Result<(), String> {
    if ctx.subagent_id.is_none() {
        return Ok(());
    }
    match crate::agent_records::sessions_root() {
        Some(root) => refuse_under(requested, resolved, &root),
        None => Ok(()),
    }
}

/// `Err` when `requested` or `resolved`, in any spelling, lies in `root`.
pub(crate) fn refuse_under(requested: &Path, resolved: &Path, root: &Path) -> Result<(), String> {
    let roots = spellings(root);
    let inside = spellings(requested)
        .into_iter()
        .chain(spellings(resolved))
        .any(|target| roots.iter().any(|root| target.starts_with(root)));
    if !inside {
        return Ok(());
    }
    Err(format!(
        "Refused: '{}' is under the agent records directory {}. A resume restores an \
         agent's confinement from the records kept there, so no spawned agent may write \
         them; only the host does. Write your files elsewhere.",
        requested.display(),
        root.display()
    ))
}

#[cfg(test)]
#[path = "path_guard_agent_records_tests.rs"]
mod tests;
