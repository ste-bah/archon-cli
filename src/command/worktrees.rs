//! `/worktrees` — review, merge and discard isolated agents' work (#184 M7).
//!
//! An isolated agent's output lands on a branch in a directory nobody looked
//! at. Without a way to see and act on it, the choice is to trust it or go
//! reading, and neither scales past two agents. Worse, the directories
//! accumulate: stale worktrees are the operational gotcha every surveyed
//! harness reports.
//!
//! Merging is always explicit. Every harness the issue surveyed converged on
//! that, and so does this: `merge` is a subcommand you type, never something a
//! completion does for you.

use archon_tools::worktree_manager::{ExitAction, WorktreeInfo, WorktreeManager};
use archon_tools::{worktree_ownership, worktree_review};
use archon_tui::events::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct WorktreesHandler;

impl CommandHandler for WorktreesHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let subcommand = args.first().map(|s| s.as_str()).unwrap_or("list");
        let rest: &[String] = if args.is_empty() { &[] } else { &args[1..] };

        let message = match subcommand {
            "" | "list" | "ls" => render_list(false),
            // Sizing walks every file under a worktree and its build
            // directory. A `target/` is gigabytes across hundreds of thousands
            // of files, and this handler runs synchronously on the dispatch
            // path — so the common case stays instant and the expensive one is
            // asked for by name.
            "sizes" | "du" => render_list(true),
            "merge" => act(rest, ExitAction::Merge),
            "discard" => act(rest, ExitAction::Discard),
            "keep" => act(rest, ExitAction::Keep),
            "prune" => render_prune(),
            other => usage(&format!("unknown subcommand `{other}`")),
        };

        ctx.emit(TuiEvent::TextDelta(message));
        Ok(())
    }

    fn description(&self) -> &str {
        "Review, merge or discard isolated agents' worktrees (list, sizes, merge, discard, prune)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["wt"]
    }
}

/// One worktree as the listing sees it.
struct Row {
    info: WorktreeInfo,
    live: bool,
    summary: String,
    age: String,
    size: Option<String>,
}

fn collect(with_sizes: bool) -> Vec<Row> {
    let root = WorktreeManager::worktrees_dir();
    let mut rows: Vec<Row> = WorktreeManager::list_worktrees()
        .into_iter()
        .map(|info| {
            let live = worktree_ownership::owner_liveness(&root, &info.owner_id)
                != worktree_ownership::OwnerLiveness::Free;
            let summary = worktree_review::review_for(&info)
                .map(|review| review.describe())
                .unwrap_or_else(|| format!("branch '{}' — unreadable", info.branch_name));
            let size = with_sizes.then(|| WorktreeManager::disk_usage(&info.owner_id).describe());
            let age = humanise_age(chrono::Utc::now() - info.created_at);
            Row {
                info,
                live,
                summary,
                age,
                size,
            }
        })
        .collect();

    // Oldest first: the ones worth acting on are the ones that have been
    // sitting around.
    rows.sort_by_key(|row| row.info.created_at);
    rows
}

fn render_list(with_sizes: bool) -> String {
    let rows = collect(with_sizes);
    if rows.is_empty() {
        return "\nNo agent worktrees.\n".to_string();
    }

    let mut out = format!("\n{} agent worktree(s):\n\n", rows.len());
    for row in &rows {
        // Whether the owner is still running decides what is safe to do, so it
        // leads the line rather than hiding in a column.
        out.push_str(&format!(
            "  {} {}\n    {}\n    owner: {}  age: {}",
            if row.live { "●" } else { "○" },
            row.info.owner_id,
            row.summary,
            if row.live { "running" } else { "finished" },
            row.age,
        ));
        if let Some(size) = &row.size {
            out.push_str(&format!("  disk: {size}"));
        }
        out.push_str(&format!("\n    {}\n\n", row.info.worktree_path.display()));
    }

    out.push_str(
        "  /worktrees merge <owner>    integrate the branch and remove the worktree\n\
         \x20 /worktrees discard <owner>  throw the work away\n\
         \x20 /worktrees keep <owner>     leave it, branch and all\n\
         \x20 /worktrees prune            remove every finished agent's worktree\n",
    );
    if !with_sizes {
        out.push_str("  /worktrees sizes            same list with disk usage (slow)\n");
    }
    out
}

fn act(args: &[String], action: ExitAction) -> String {
    let Some(owner) = args.first() else {
        return usage("missing <owner> — run `/worktrees` to see the list");
    };

    // Refuses while the owner is still running — the same rule that stops a
    // spawn from reclaiming a live worktree, and for the same reason (#184 M4).
    match WorktreeManager::exit_by_owner(owner, action) {
        Ok(message) => format!("\n{message}\n"),
        Err(error) => format!("\nFailed: {error}\n"),
    }
}

/// Remove every finished agent's worktree.
///
/// Liveness-filtered rather than age-filtered: a finished agent's worktree is
/// reclaimable now, and a running agent's never is, whatever its age. Anything
/// with uncommitted work refuses and says so — prune is for tidying, not for
/// discarding work nobody reviewed.
fn render_prune() -> String {
    let rows = collect(false);
    let finished: Vec<&Row> = rows.iter().filter(|row| !row.live).collect();

    if finished.is_empty() {
        return "\nNothing to prune — every worktree belongs to a running agent.\n".to_string();
    }

    let mut removed = 0usize;
    let mut kept = Vec::new();
    for row in finished {
        match WorktreeManager::cleanup_session(&row.info.owner_id) {
            Ok(()) => removed += 1,
            Err(reason) => kept.push(format!("  {} — {reason}", row.info.owner_id)),
        }
    }

    let mut out = format!("\nPruned {removed} worktree(s).\n");
    if !kept.is_empty() {
        out.push_str(&format!(
            "\nKept {} with work that would have been lost:\n{}\n",
            kept.len(),
            kept.join("\n")
        ));
    }
    out
}

fn humanise_age(age: chrono::Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = age.num_hours();
    if hours < 48 {
        return format!("{hours}h");
    }
    format!("{}d", age.num_days())
}

fn usage(problem: &str) -> String {
    format!(
        "\n{problem}\n\n\
         Usage:\n\
         \x20 /worktrees                  list isolated agents' worktrees\n\
         \x20 /worktrees sizes            list with disk usage (walks every file)\n\
         \x20 /worktrees merge <owner>    integrate the branch and remove the worktree\n\
         \x20 /worktrees discard <owner>  throw the work away\n\
         \x20 /worktrees keep <owner>     leave it, branch and all\n\
         \x20 /worktrees prune            remove every finished agent's worktree\n"
    )
}

#[cfg(test)]
#[path = "worktrees_tests.rs"]
mod tests;
