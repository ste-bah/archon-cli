//! Lint test for TASK-TUI-109 / ERR-TUI-002 regression prevention.
//!
//! TUI-107 removed the inline `.process_message(...).await` from the
//! main event loop body and moved it into `AgentHandle::run_turn`
//! (src/agent_handle.rs), which is the *only* legitimate holder of
//! that await chain (it guards the await with a mutex and runs inside
//! a spawned tokio task via `AgentDispatcher::spawn_turn`, so it never
//! blocks the event loop).
//!
//! If anyone re-introduces `agent.process_message(...).await` inside
//! `src/main.rs` — the event-loop-carrying binary — the original
//! ERR-TUI-002 input-freeze blocker comes right back, because that
//! await is reached from the select! body on the main loop thread.
//!
//! This test is a **pure-Rust static scanner**. It does NOT invoke
//! `rg` as a subprocess (see deviation D1 in the TUI-109 report:
//! ripgrep is not on the system PATH in this environment, so
//! `std::process::Command::new("rg")` would fail with ENOENT — the
//! only working `rg` is the Claude-vendored one reachable through a
//! node alias, which is unusable from a test binary).
//!
//! Instead we read the file with `std::fs::read_to_string` and do a
//! per-line substring scan.
//!
//! ## Scope
//!
//! - **Scans:** `src/main.rs` (resolved relative to `CARGO_MANIFEST_DIR`)
//! - **Deliberately excludes:** `src/agent_handle.rs`. That file is
//!   the intended home of `.process_message(&prompt).await` —
//!   excluding it is not an oversight, it is the whole point of
//!   TUI-107. The second test below asserts the exclusion is
//!   *intentional* by verifying agent_handle.rs still contains the
//!   pattern; if that ever changes, the reviewer is forced to
//!   reconsider the lint scope.

use std::path::PathBuf;

fn worktree_root_path(relative: &str) -> PathBuf {
    // A test binary compiled from `crates/archon-tui/tests/*.rs` has
    // CARGO_MANIFEST_DIR = `<worktree>/crates/archon-tui`. Jump up
    // two levels to reach the worktree root, then append.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn test_no_inline_await_on_process_message_in_input_loop() {
    let main_rs_path = worktree_root_path("src/main.rs");
    let main_rs = std::fs::read_to_string(&main_rs_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", main_rs_path.display(), e));

    // Pattern A: inline `.process_message(...).await` on a single line.
    // Substring-based: matches `agent.process_message(&prompt).await`,
    // `self.process_message("foo").await`, etc. This is deliberately
    // over-inclusive; any true positive is a bug regardless of the
    // receiver expression.
    let mut pattern_a_hits: Vec<(usize, String)> = Vec::new();
    for (idx, line) in main_rs.lines().enumerate() {
        if line.contains(".process_message(") && line.contains(").await") {
            pattern_a_hits.push((idx + 1, line.to_string()));
        }
    }

    // Pattern B: the identifier `current_agent_task_inner`, which was
    // the name of the per-turn serialization slot deleted in TUI-107
    // D6. Its reappearance means somebody is trying to serialize
    // turns inside the main loop instead of via AgentDispatcher.
    let pattern_b_hit = main_rs.contains("current_agent_task_inner");

    if !pattern_a_hits.is_empty() {
        let mut msg = String::new();
        msg.push_str(
            "LINT FAIL (ERR-TUI-002 regression): inline \
             `.process_message(...).await` found in src/main.rs.\n\n\
             This is exactly the pattern TUI-107 migrated away from. \
             The input event loop MUST NOT await `agent.process_message` \
             directly — it must dispatch the turn through \
             `AgentDispatcher::spawn_turn`, which owns an \
             `Arc<dyn TurnRunner>` (see src/agent_handle.rs for the \
             legitimate impl). Awaiting in the main loop reintroduces \
             the input-freeze blocker.\n\n\
             Offending lines:\n",
        );
        for (lineno, content) in &pattern_a_hits {
            msg.push_str(&format!(
                "  {}:{}: {}\n",
                main_rs_path.display(),
                lineno,
                content.trim_end()
            ));
        }
        panic!("{}", msg);
    }

    if pattern_b_hit {
        panic!(
            "LINT FAIL: identifier `current_agent_task_inner` found in \
             src/main.rs. This was the per-turn serialization slot \
             deleted in TUI-107 D6. Any code referencing it is either \
             stale or is trying to re-implement dispatcher serialization \
             inside the main loop, which is a structural regression \
             against TUI-107's design."
        );
    }
}

#[test]
fn test_lint_scope_excludes_agent_handle_rs() {
    // Sanity / documentation test. src/agent_handle.rs is the
    // intentional holder of `.process_message(&prompt).await` — it
    // runs inside a spawned task via AgentDispatcher, so the await
    // never touches the main event loop thread. This test verifies
    // the file still contains both substrings; if it doesn't, either
    // the adapter was removed/renamed (which is a design change
    // requiring a reviewer to update the lint) or the pattern moved,
    // in which case the main-loop lint above may need to re-scope.
    //
    // Concretely: if this test starts failing, DO NOT just delete it.
    // Stop and re-verify that the `.process_message(...).await` call
    // chain is still hosted by a spawned-task adapter outside of
    // main.rs, and then update the file path here accordingly.

    let agent_handle_path = worktree_root_path("src/agent_handle.rs");
    let agent_handle = std::fs::read_to_string(&agent_handle_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", agent_handle_path.display(), e));

    assert!(
        agent_handle.contains(".process_message("),
        "expected src/agent_handle.rs to still contain `.process_message(` \
         (the legitimate TurnRunner adapter). If this assert fires, the \
         lint scope in test_no_inline_await_on_process_message_in_input_loop \
         probably needs review — see module docs."
    );
    assert!(
        agent_handle.contains(".await"),
        "expected src/agent_handle.rs to still contain `.await` \
         somewhere — the TurnRunner impl for AgentHandle owns the \
         await chain by design. See module docs."
    );
}
