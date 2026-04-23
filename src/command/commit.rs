//! TASK-TUI-624 /commit slash-command handler (Gate 2 implementation).
//!
//! `/commit` gathers git context (branch, recent log, status, diff) and
//! builds a structured commit-authoring prompt for the model with a
//! mandatory safety protocol:
//!
//!   - NEVER amend unless the user explicitly requests it.
//!   - NEVER skip hooks (no --no-verify / --no-gpg-sign).
//!   - Warn if `.env` or credentials files are staged.
//!   - No `-i` (interactive) flags on git commands.
//!   - Stage relevant files explicitly; do NOT `git add -A` or `git add .`.
//!
//! Emits one `TuiEvent::TextDelta` carrying the full prompt. If the
//! cwd is not a git repo OR the tree is clean, returns `Err(...)`.
//!
//! # Reconciliation with TASK-TUI-624.md spec
//!
//! Spec references `crates/archon-tui/src/slash/commit.rs` +
//! `SlashCommand` + `SlashOutcome::Message`. Actual: bin-crate
//! `src/command/commit.rs` + `CommandHandler` (re-exported as
//! `SlashCommand` at `src/command/mod.rs:86`) + `ctx.emit(TuiEvent::TextDelta)`.
//!
//! # Testability — git-runner seam
//!
//! `git` is an external binary; direct `std::process::Command`
//! invocation in tests would require a real repo with a known HEAD,
//! diff, and branch state — non-deterministic. Gate 2 introduces a
//! `GitRunner` trait with:
//!
//!   - `RealGit` — default impl shelling out to `git`.
//!   - `MockGit` (`#[cfg(test)]`) — returns canned stdout/Err.
//!
//! `CommitHandler` stores `Arc<dyn GitRunner>` (default `RealGit` via
//! `CommitHandler::new()`). Tests inject `MockGit` via
//! `CommitHandler::with_runner()`.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// External `git` CLI runner seam. Tests inject canned output via MockGit.
pub(crate) trait GitRunner: Send + Sync {
    /// `git rev-parse --git-dir` — Err if cwd is not a git repo.
    fn rev_parse_git_dir(&self) -> Result<String, String>;
    /// `git status --porcelain` — empty stdout means clean tree.
    fn status_porcelain(&self) -> Result<String, String>;
    /// `git branch --show-current`.
    fn branch_show_current(&self) -> Result<String, String>;
    /// `git log --oneline -10`.
    fn log_oneline_10(&self) -> Result<String, String>;
    /// `git status` — full (non-porcelain) output for the prompt.
    fn status(&self) -> Result<String, String>;
    /// `git diff HEAD`.
    fn diff_head(&self) -> Result<String, String>;
}

/// Default runner — shells out to real `git`.
pub(crate) struct RealGit;

impl GitRunner for RealGit {
    fn rev_parse_git_dir(&self) -> Result<String, String> {
        run_git(&["rev-parse", "--git-dir"])
    }
    fn status_porcelain(&self) -> Result<String, String> {
        run_git(&["status", "--porcelain"])
    }
    fn branch_show_current(&self) -> Result<String, String> {
        run_git(&["branch", "--show-current"])
    }
    fn log_oneline_10(&self) -> Result<String, String> {
        run_git(&["log", "--oneline", "-10"])
    }
    fn status(&self) -> Result<String, String> {
        run_git(&["status"])
    }
    fn diff_head(&self) -> Result<String, String> {
        run_git(&["diff", "HEAD"])
    }
}

fn run_git(args: &[&str]) -> Result<String, String> {
    use std::process::Command;
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("git not available: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git command failed (exit {:?}): {}",
            output.status.code(),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `/commit` handler — builds a structured git-commit prompt for the model.
pub(crate) struct CommitHandler {
    runner: std::sync::Arc<dyn GitRunner>,
}

impl CommitHandler {
    pub(crate) fn new() -> Self {
        Self {
            runner: std::sync::Arc::new(RealGit),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_runner(runner: std::sync::Arc<dyn GitRunner>) -> Self {
        Self { runner }
    }
}

impl CommandHandler for CommitHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        let prompt = build_commit_prompt(&*self.runner)
            .map_err(|e| anyhow::anyhow!(e))?;
        ctx.emit(TuiEvent::TextDelta(prompt));
        Ok(())
    }

    fn description(&self) -> &str {
        "Create a git commit with AI assistance (gathers status/diff/log, emits structured prompt)"
    }
}

fn build_commit_prompt(runner: &dyn GitRunner) -> Result<String, String> {
    const SAFETY_PROTOCOL: &str = "\
SAFETY PROTOCOL — follow strictly:
  - NEVER amend unless the user explicitly requests `git commit --amend`.
  - NEVER skip hooks (`--no-verify`, `--no-gpg-sign`).
  - Warn the user if `.env`, `credentials.json`, or any secret-looking \
    file is staged — do NOT commit it without explicit confirmation.
  - Do NOT use `-i` / `--interactive` flags on any git command.
  - Stage files explicitly by name; do NOT `git add -A` or `git add .`.
  - Present the commit message for user review BEFORE running `git commit`.";

    // Verify git repo first — short-circuit on non-repo.
    runner.rev_parse_git_dir().map_err(|_| {
        "not a git repository (run /commit inside a git repo)".to_string()
    })?;

    // Verify changes exist — clean tree short-circuits with clear Err.
    let porcelain = runner.status_porcelain()?;
    if porcelain.trim().is_empty() {
        return Err("nothing to commit (working tree clean)".to_string());
    }

    let branch = runner.branch_show_current().unwrap_or_default();
    let log = runner.log_oneline_10().unwrap_or_default();
    let status = runner.status().unwrap_or_default();
    let diff = runner.diff_head().unwrap_or_default();

    Ok(format!(
        "\n/commit — create a git commit with AI assistance.\n\n\
         ## Branch\n{}\n\n\
         ## Recent commits (last 10)\n{}\n\n\
         ## Status\n{}\n\n\
         ## Diff vs HEAD\n{}\n\n\
         ## Instructions\n\
         Analyze the changes. Draft a concise 1-2 sentence commit message \
         in this repo's existing style. Stage the relevant files by name. \
         Present the message for confirmation before running `git commit`.\n\n\
         {}\n",
        branch.trim(),
        log.trim(),
        status.trim(),
        diff,
        SAFETY_PROTOCOL,
    ))
}

#[cfg(test)]
mod tests {
    //! Gate 2 tests — assert prompt formatting and Err paths via `MockGit`.

    use super::*;
    use crate::command::test_support::*;
    use std::sync::Arc;

    struct MockGit {
        rev_parse_out: Result<String, String>,
        porcelain_out: Result<String, String>,
        branch_out: Result<String, String>,
        log_out: Result<String, String>,
        status_out: Result<String, String>,
        diff_out: Result<String, String>,
    }
    impl GitRunner for MockGit {
        fn rev_parse_git_dir(&self) -> Result<String, String> {
            self.rev_parse_out.clone()
        }
        fn status_porcelain(&self) -> Result<String, String> {
            self.porcelain_out.clone()
        }
        fn branch_show_current(&self) -> Result<String, String> {
            self.branch_out.clone()
        }
        fn log_oneline_10(&self) -> Result<String, String> {
            self.log_out.clone()
        }
        fn status(&self) -> Result<String, String> {
            self.status_out.clone()
        }
        fn diff_head(&self) -> Result<String, String> {
            self.diff_out.clone()
        }
    }

    fn mock_with_changes() -> MockGit {
        MockGit {
            rev_parse_out: Ok(".git".to_string()),
            porcelain_out: Ok(" M src/foo.rs\n?? new.rs".to_string()),
            branch_out: Ok("archonfixes".to_string()),
            log_out: Ok("abc1234 prior commit\ndef5678 another".to_string()),
            status_out: Ok(
                "On branch archonfixes\nChanges not staged".to_string(),
            ),
            diff_out: Ok(
                "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1 +1 @@\n-old\n+new"
                    .to_string(),
            ),
        }
    }

    #[test]
    fn no_git_repo_returns_err() {
        let mut mock = mock_with_changes();
        mock.rev_parse_out = Err("fatal: not a git repository".to_string());
        let handler = CommitHandler::with_runner(Arc::new(mock));
        let (mut ctx, mut _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "expected Err when rev-parse fails");
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("git") && msg.contains("repo"),
            "err must mention git repo; got: {}",
            msg
        );
    }

    #[test]
    fn no_changes_returns_err() {
        let mut mock = mock_with_changes();
        mock.porcelain_out = Ok(String::new()); // clean tree
        let handler = CommitHandler::with_runner(Arc::new(mock));
        let (mut ctx, mut _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "expected Err on clean tree");
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("nothing to commit") || msg.contains("clean"),
            "err must indicate clean tree; got: {}",
            msg
        );
    }

    #[test]
    fn with_changes_returns_prompt_containing_diff() {
        let handler = CommitHandler::with_runner(Arc::new(mock_with_changes()));
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected one TextDelta; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("archonfixes"),
                    "prompt must contain branch name; got: {}",
                    s
                );
                assert!(
                    s.contains("--- a/src/foo.rs"),
                    "prompt must contain diff; got: {}",
                    s
                );
                assert!(
                    s.contains("abc1234"),
                    "prompt must contain recent log entry; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn prompt_contains_safety_protocol() {
        let handler = CommitHandler::with_runner(Arc::new(mock_with_changes()));
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                let lower = s.to_lowercase();
                assert!(
                    lower.contains("never amend"),
                    "prompt must warn 'never amend'; got: {}",
                    s
                );
                assert!(
                    lower.contains("never skip hooks"),
                    "prompt must warn 'never skip hooks'; got: {}",
                    s
                );
                assert!(
                    lower.contains("credentials") || lower.contains(".env"),
                    "prompt must warn about secret files; got: {}",
                    s
                );
                assert!(
                    lower.contains("interactive") || lower.contains("-i"),
                    "prompt must warn about -i flag; got: {}",
                    s
                );
                assert!(
                    lower.contains("git add -a") || lower.contains("git add ."),
                    "prompt must warn against blanket git add; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn commit_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("commit") must return Some(handler) because
        // default_registry() registers CommitHandler::new() (with the real RealGit
        // runner). Execute may Ok or Err depending on whether cwd is a dirty git
        // repo — both outcomes prove the registration + dispatch wiring works.
        // The real-git environment state is out of scope.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("commit")
            .expect("commit must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);

        match result {
            Ok(()) => {
                let events = drain_tui_events(&mut rx);
                assert_eq!(
                    events.len(),
                    1,
                    "expected exactly one TextDelta on Ok path; got: {:?}",
                    events
                );
                match &events[0] {
                    TuiEvent::TextDelta(s) => {
                        let lower = s.to_lowercase();
                        // Ok path: real git ran, prompt must carry the safety protocol
                        // AND at least one of the structural headers.
                        assert!(
                            lower.contains("safety protocol"),
                            "Ok-path TextDelta must carry SAFETY PROTOCOL; got: {}",
                            s
                        );
                        assert!(
                            lower.contains("never amend"),
                            "Ok-path TextDelta must contain 'never amend'; got: {}",
                            s
                        );
                    }
                    other => panic!("expected TextDelta on Ok path, got: {:?}", other),
                }
            }
            Err(e) => {
                let msg = format!("{:#}", e).to_lowercase();
                // Err path covers: not-a-git-repo OR clean-tree. Either is acceptable —
                // proves the dispatcher ran build_commit_prompt through the real RealGit.
                assert!(
                    (msg.contains("git") && msg.contains("repo"))
                        || msg.contains("nothing to commit")
                        || msg.contains("clean"),
                    "Err path must mention 'git repo' or 'nothing to commit'/'clean'; got: {}",
                    msg
                );
                let events = drain_tui_events(&mut rx);
                assert!(
                    events.is_empty(),
                    "Err path must not emit any events; got: {:?}",
                    events
                );
            }
        }
    }
}
