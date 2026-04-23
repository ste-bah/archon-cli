//! TASK-TUI-622 /review slash-command handler (Gate 2 implementation).
//!
//! `/review [<pr-number>]` builds a structured code-review prompt for
//! the model:
//!
//!   - No arg: run `gh pr list --limit 20` and format into a "choose a
//!     PR" prompt message.
//!   - PR number: run `gh pr view <n>` + `gh pr diff <n>` and format
//!     into a full review-request prompt.
//!
//! Output is a single `TuiEvent::TextDelta` carrying the prompt text.
//! Safety guardrail embedded in the prompt: "Do not post the review
//! automatically; present it for user confirmation."
//!
//! If `gh` is not on PATH OR cwd is not a git repo -> `Err(...)`.
//!
//! # Reconciliation with TASK-TUI-622.md spec
//!
//! Spec references `crates/archon-tui/src/slash/review.rs` +
//! `SlashCommand` trait + `SlashOutcome::Message`. Same reconciliation
//! as TASK-TUI-621: handler lives in bin crate `src/command/`, trait is
//! `CommandHandler` (re-exported as `SlashCommand` at
//! `src/command/mod.rs:86`), output via `ctx.emit(TuiEvent::TextDelta)`.
//!
//! # Testability — gh-runner seam
//!
//! `gh` is a real external binary; direct `std::process::Command`
//! invocation in tests is non-deterministic (requires network, PR state).
//! Gate 2 introduces a `GhRunner` trait with:
//!
//!   - `RealGh` — default impl that shells out via `std::process::Command`.
//!   - `MockGh` (test-only, in the tests module) — returns canned
//!     stdout/Err so prompt formatting, Err paths, and the safety
//!     guardrail text can be asserted without hitting the real `gh`
//!     binary.
//!
//! `ReviewHandler` stores an `Arc<dyn GhRunner>` (defaulting to `RealGh`
//! when constructed via `ReviewHandler::new()`). Tests use
//! `ReviewHandler::with_runner(...)` to inject a `MockGh`.
//!
//! # Shell-injection guard
//!
//! The PR-number argument is validated to be ASCII-digits-only before
//! it is passed to `gh`. This rejects any free-form user string that
//! could otherwise be interpreted as a flag or extra argument by `gh`.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// External `gh` CLI runner seam. Allows tests to inject canned output.
///
/// Methods return `Result<String, String>` where Ok(stdout) is the
/// captured UTF-8 stdout of the `gh` invocation and Err(msg) is a
/// human-readable failure message (command not found, non-zero exit,
/// not-a-git-repo, etc.).
pub(crate) trait GhRunner: Send + Sync {
    /// Run `gh pr list --limit 20` or equivalent. Returns formatted PR list.
    fn pr_list(&self) -> Result<String, String>;
    /// Run `gh pr view <number>`. Returns PR metadata.
    fn pr_view(&self, number: &str) -> Result<String, String>;
    /// Run `gh pr diff <number>`. Returns unified diff.
    fn pr_diff(&self, number: &str) -> Result<String, String>;
}

/// Default `GhRunner` impl — shells out via `std::process::Command`.
pub(crate) struct RealGh;

impl GhRunner for RealGh {
    fn pr_list(&self) -> Result<String, String> {
        run_gh(&["pr", "list", "--limit", "20"])
    }
    fn pr_view(&self, number: &str) -> Result<String, String> {
        run_gh(&["pr", "view", number])
    }
    fn pr_diff(&self, number: &str) -> Result<String, String> {
        run_gh(&["pr", "diff", number])
    }
}

fn run_gh(args: &[&str]) -> Result<String, String> {
    use std::process::Command;
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| format!("gh CLI not available: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "gh command failed (exit {:?}): {}",
            output.status.code(),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `/review` handler — builds a structured PR-review prompt for the model.
pub(crate) struct ReviewHandler {
    runner: std::sync::Arc<dyn GhRunner>,
}

impl ReviewHandler {
    pub(crate) fn new() -> Self {
        Self {
            runner: std::sync::Arc::new(RealGh),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_runner(runner: std::sync::Arc<dyn GhRunner>) -> Self {
        Self { runner }
    }
}

impl CommandHandler for ReviewHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        let prompt = build_review_prompt(&*self.runner, args)
            .map_err(|e| anyhow::anyhow!(e))?;
        ctx.emit(TuiEvent::TextDelta(prompt));
        Ok(())
    }

    fn description(&self) -> &str {
        "Review a pull request (no arg: list open PRs; with number: review diff)"
    }
}

fn build_review_prompt(
    runner: &dyn GhRunner,
    args: &[String],
) -> Result<String, String> {
    const GUARDRAIL: &str = "SAFETY: Do not post the review automatically. Present the review to the user for confirmation before any action.";

    match args.first().map(|s| s.as_str()) {
        None | Some("") => {
            let list = runner.pr_list()?;
            Ok(format!(
                "\n/review — open pull requests:\n\n{}\n\nChoose a PR number and re-run `/review <number>` to get a full review prompt.\n\n{}\n",
                list.trim(),
                GUARDRAIL,
            ))
        }
        Some(n) => {
            // Validate PR number is numeric (reject shell-injection / free-form strings).
            if !n.chars().all(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "invalid PR number '{}': must be a positive integer",
                    n
                ));
            }
            let view = runner.pr_view(n)?;
            let diff = runner.pr_diff(n)?;
            Ok(format!(
                "\n/review — pull request #{}:\n\n## Metadata\n{}\n\n## Diff\n{}\n\nReview the diff for: code quality, style consistency with the repo, performance, test coverage, security considerations, and any missing edge cases.\n\n{}\n",
                n,
                view.trim(),
                diff, // keep diff exact — do not trim whitespace-significant lines
                GUARDRAIL,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    //! Gate 2 tests — assert prompt formatting via `MockGh`.

    use super::*;
    use crate::command::test_support::*;
    use std::sync::Arc;

    struct MockGh {
        pr_list_out: Result<String, String>,
        pr_view_out: Result<String, String>,
        pr_diff_out: Result<String, String>,
    }
    impl GhRunner for MockGh {
        fn pr_list(&self) -> Result<String, String> {
            self.pr_list_out.clone()
        }
        fn pr_view(&self, _n: &str) -> Result<String, String> {
            self.pr_view_out.clone()
        }
        fn pr_diff(&self, _n: &str) -> Result<String, String> {
            self.pr_diff_out.clone()
        }
    }

    #[test]
    fn no_args_prompt_lists_prs() {
        let mock = Arc::new(MockGh {
            pr_list_out: Ok(
                "#42  Fix authentication bug\n#43  Add telemetry".to_string()
            ),
            pr_view_out: Err("should not be called".to_string()),
            pr_diff_out: Err("should not be called".to_string()),
        });
        let handler = ReviewHandler::with_runner(mock);
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
                    s.contains("#42"),
                    "prompt must list PR #42; got: {}",
                    s
                );
                assert!(
                    s.contains("#43"),
                    "prompt must list PR #43; got: {}",
                    s
                );
                assert!(
                    s.to_lowercase().contains("do not post"),
                    "prompt must carry safety guardrail; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn with_pr_number_prompt_contains_diff() {
        let mock = Arc::new(MockGh {
            pr_list_out: Err("should not be called".to_string()),
            pr_view_out: Ok(
                "title: Fix auth\nauthor: octocat\nstate: OPEN".to_string(),
            ),
            pr_diff_out: Ok(
                "--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -1 +1 @@\n-old\n+new"
                    .to_string(),
            ),
        });
        let handler = ReviewHandler::with_runner(mock);
        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[String::from("42")])
            .unwrap();
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
                    s.contains("--- a/src/auth.rs"),
                    "prompt must contain diff; got: {}",
                    s
                );
                assert!(
                    s.contains("octocat"),
                    "prompt must contain PR metadata; got: {}",
                    s
                );
                assert!(
                    s.to_lowercase().contains("do not post"),
                    "prompt must carry safety guardrail; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn gh_not_available_returns_err() {
        let mock = Arc::new(MockGh {
            pr_list_out: Err(
                "gh CLI not available: No such file or directory".to_string(),
            ),
            pr_view_out: Err("unused".to_string()),
            pr_diff_out: Err("unused".to_string()),
        });
        let handler = ReviewHandler::with_runner(mock);
        let (mut ctx, mut _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "expected Err when gh unavailable");
        let err_msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            err_msg.contains("gh") || err_msg.contains("git"),
            "error must mention gh or git; got: {}",
            err_msg
        );
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn review_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("review") must return Some(handler) because
        // default_registry() registers ReviewHandler::new() (with the real RealGh
        // runner). Execute may Ok or Err depending on whether `gh` is installed
        // in the smoke environment — both outcomes prove the registration +
        // dispatch wiring is correct. The real-gh failure mode is out of scope.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("review")
            .expect("review must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);

        match result {
            Ok(()) => {
                // Real gh ran OK — assert a single TextDelta carrying the guardrail.
                let events = drain_tui_events(&mut rx);
                assert_eq!(
                    events.len(),
                    1,
                    "expected exactly one TextDelta on Ok path; got: {:?}",
                    events
                );
                match &events[0] {
                    TuiEvent::TextDelta(s) => {
                        assert!(
                            s.to_lowercase().contains("do not post"),
                            "Ok-path TextDelta must carry safety guardrail; got: {}",
                            s
                        );
                    }
                    other => panic!("expected TextDelta on Ok path, got: {:?}", other),
                }
            }
            Err(e) => {
                // gh unavailable in smoke env — assert error mentions gh/git.
                let msg = format!("{:#}", e).to_lowercase();
                assert!(
                    msg.contains("gh") || msg.contains("git"),
                    "Err path must mention gh or git; got: {}",
                    msg
                );
                // No TextDelta should have been emitted on the Err path.
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
