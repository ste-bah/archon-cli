//! Issue #124 — self-monitoring: the cheap gate runs on the file just edited,
//! and the verdict comes back as the agent's own observation.
//!
//! These tests drive the real `scripts/self-check-file.sh` through the real
//! hook executor, wired by the real shipped `.archon/hooks.toml`, and assert on
//! the resulting `ToolResult`. That last part is the load-bearing one: if the
//! verdict does not survive into the tool result it never reaches the
//! transcript, and the feature is invisible even when every other piece works.
//! Asserting that the code path exists would prove nothing.

use super::tool_postprocess_steps::PostprocessFlow;
use super::tool_types::PreflightResult;
use super::*;
use archon_tools::tool::{PermissionLevel, Tool};
use std::path::{Path, PathBuf};

const HOOK_CONTEXT_MARKER: &str = "[Hook Context]\n";

/// Placeholder tool. `run_post_tool_hooks` only re-executes it on a hook-driven
/// retry, which none of these hooks request.
struct StubTool(String);

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.0
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "stub"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success("stub")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root")
}

/// A throwaway checkout holding a copy of the real script plus whatever
/// allowlist the test needs. The script derives the repo root from its own
/// location, so copying it is what scopes the check to this temp tree.
fn fake_repo(allowlist: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let scripts = dir.path().join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    std::fs::copy(
        repo_root().join("scripts").join("self-check-file.sh"),
        scripts.join("self-check-file.sh"),
    )
    .expect("copy self-check-file.sh");
    std::fs::write(scripts.join("check-file-sizes.allowlist"), allowlist).expect("allowlist");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
    dir
}

fn write_lines(path: &Path, count: usize) {
    let body: String = (1..=count).map(|i| format!("// line {i}\n")).collect();
    std::fs::write(path, body).expect("write source file");
}

fn registry_from_toml(toml: &str) -> Arc<crate::hooks::HookRegistry> {
    let settings = crate::hooks::parse_hooks_toml(toml).expect("hooks toml parses");
    let registry = crate::hooks::HookRegistry::new();
    for (event, matchers) in settings {
        registry.register_matchers(event, matchers, Some("project"));
    }
    Arc::new(registry)
}

/// The registry the repo actually ships, loaded the way the binary loads it.
/// `home_dir` is a temp dir so a developer's own `~/.archon/hooks.toml` cannot
/// leak into the assertions.
fn shipped_registry(home: &Path) -> Arc<crate::hooks::HookRegistry> {
    Arc::new(crate::hooks::HookRegistry::load_all(&repo_root(), home))
}

fn agent_in(working_dir: &Path, hooks: Arc<crate::hooks::HookRegistry>) -> Agent {
    let mut agent = super::tests::test_agent();
    agent.config.working_dir = working_dir.to_path_buf();
    agent.set_hook_registry(hooks);
    agent
}

/// Run the PostToolUse stage exactly as `postprocess_single_tool` does and
/// return the (possibly hook-mutated) result.
async fn post_tool_result(
    agent: &mut Agent,
    tool_name: &str,
    file_path: Option<&Path>,
    input: serde_json::Value,
) -> ToolResult {
    let pre = PreflightResult {
        tool_name: tool_name.to_string(),
        tool_id: "tool-use-1".into(),
        input,
        tool_arc: Arc::new(StubTool(tool_name.to_string())),
        file_path: file_path.map(|p| p.to_string_lossy().into_owned()),
        filesystem_effect: archon_tools::tool::WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    let mut result = ToolResult::success("The file has been updated.");
    let mut raw_result = result.clone();
    let ctx = ToolContext::default();
    let mut flow = PostprocessFlow::default();
    agent
        .run_post_tool_hooks(
            &pre,
            &mut raw_result,
            &mut result,
            &ctx,
            "test-model",
            &mut flow,
        )
        .await;
    result
}

fn edit_input(path: &Path) -> serde_json::Value {
    serde_json::json!({
        "file_path": path.to_string_lossy(),
        "old_string": "a",
        "new_string": "b",
    })
}

// ---------------------------------------------------------------------------
// The production change: PostToolUse now says which file was written.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_tool_use_payload_carries_the_edited_file_and_lands_in_the_tool_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("src").join("lib.rs");
    // `cat` echoes the payload back as additional_context, so the assertion is
    // on the bytes the hook actually received, not on a reconstruction of them.
    let hooks = registry_from_toml(
        r#"
[hooks.PostToolUse]
matchers = [{ hooks = [{ type = "prompt", command = "cat", if_condition = "Edit", timeout = 5 }] }]
"#,
    );
    let mut agent = agent_in(dir.path(), hooks);

    let result = post_tool_result(&mut agent, "Edit", Some(&target), edit_input(&target)).await;

    let echoed = result
        .content
        .split_once(HOOK_CONTEXT_MARKER)
        .expect("hook output appended to the tool result")
        .1;
    let payload: serde_json::Value =
        serde_json::from_str(echoed.trim()).expect("payload echoed verbatim");
    assert_eq!(payload["hook_event"], "PostToolUse");
    assert_eq!(payload["tool_name"], "Edit");
    assert_eq!(payload["file_path"], target.to_string_lossy().as_ref());
    assert_eq!(
        payload["tool_input"]["file_path"],
        target.to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn post_tool_use_payload_reports_no_file_for_a_non_file_tool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let hooks = registry_from_toml(
        r#"
[hooks.PostToolUse]
matchers = [{ hooks = [{ type = "prompt", command = "cat", if_condition = "Bash", timeout = 5 }] }]
"#,
    );
    let mut agent = agent_in(dir.path(), hooks);

    let result = post_tool_result(
        &mut agent,
        "Bash",
        None,
        serde_json::json!({"command": "echo hi"}),
    )
    .await;

    let echoed = result
        .content
        .split_once(HOOK_CONTEXT_MARKER)
        .expect("hook output appended to the tool result")
        .1;
    let payload: serde_json::Value =
        serde_json::from_str(echoed.trim()).expect("payload echoed verbatim");
    assert_eq!(payload["file_path"], serde_json::Value::Null);
    assert_eq!(payload["tool_input"]["command"], "echo hi");
}

// ---------------------------------------------------------------------------
// The verdict, produced by the real script under the real shipped config.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn over_limit_file_produces_a_verdict_the_agent_can_read() {
    let home = tempfile::tempdir().expect("home");
    let repo = fake_repo("");
    let target = repo.path().join("src").join("big.rs");
    write_lines(&target, 501);
    let mut agent = agent_in(repo.path(), shipped_registry(home.path()));

    let result = post_tool_result(&mut agent, "Edit", Some(&target), edit_input(&target)).await;

    assert!(
        result.content.contains(HOOK_CONTEXT_MARKER),
        "verdict never reached the tool result: {}",
        result.content
    );
    assert!(
        result.content.contains("src/big.rs") && result.content.contains("501"),
        "verdict did not name the offending file and size: {}",
        result.content
    );
    assert!(
        result.content.starts_with("The file has been updated."),
        "the original tool output must survive the append: {}",
        result.content
    );
}

#[tokio::test]
async fn verdict_is_recorded_in_the_conversation_transcript() {
    // The full postprocess stage, not just the hook step: a verdict that stops
    // at the local `ToolResult` is a verdict the model never sees. This is the
    // assertion the whole feature rests on.
    let home = tempfile::tempdir().expect("home");
    let repo = fake_repo("");
    let target = repo.path().join("src").join("big.rs");
    write_lines(&target, 501);
    let mut agent = agent_in(repo.path(), shipped_registry(home.path()));
    let filesystem_effect = archon_tools::tool::WorkingTreeEffect::DeclaredPaths;
    let filesystem_before = agent
        .observe_filesystem_before_mutation(filesystem_effect)
        .expect("filesystem baseline");
    let pre = PreflightResult {
        tool_name: "Edit".into(),
        tool_id: "tool-use-1".into(),
        input: edit_input(&target),
        tool_arc: Arc::new(StubTool("Edit".into())),
        file_path: Some(target.to_string_lossy().into_owned()),
        filesystem_effect,
        filesystem_before,
        sandbox_prechecked: true,
    };

    agent
        .postprocess_single_tool(
            &pre,
            ToolResult::success("The file has been updated."),
            &ToolContext::default(),
            "test-model",
            &mut PostprocessFlow::default(),
        )
        .await;

    let transcript = serde_json::to_string(&agent.state.messages).expect("transcript");
    assert!(
        transcript.contains("FileSizeGuard") && transcript.contains("src/big.rs"),
        "verdict never entered the transcript: {transcript}"
    );
}

#[tokio::test]
async fn allowlisted_over_limit_file_stays_quiet() {
    let home = tempfile::tempdir().expect("home");
    // Written the way the real allowlist is: comments and padding included, so
    // the normalisation path is exercised rather than assumed.
    let repo = fake_repo("# grandfathered\n  src/big.rs  \n");
    let target = repo.path().join("src").join("big.rs");
    write_lines(&target, 501);
    let mut agent = agent_in(repo.path(), shipped_registry(home.path()));

    let result = post_tool_result(&mut agent, "Edit", Some(&target), edit_input(&target)).await;

    assert!(
        !result.content.contains(HOOK_CONTEXT_MARKER),
        "the hook cried wolf on a grandfathered file: {}",
        result.content
    );
}

#[tokio::test]
async fn under_limit_file_stays_quiet() {
    let home = tempfile::tempdir().expect("home");
    let repo = fake_repo("");
    let target = repo.path().join("src").join("small.rs");
    write_lines(&target, 12);
    let mut agent = agent_in(repo.path(), shipped_registry(home.path()));

    let result = post_tool_result(&mut agent, "Edit", Some(&target), edit_input(&target)).await;

    assert!(
        !result.content.contains(HOOK_CONTEXT_MARKER),
        "a passing check must not spend transcript budget: {}",
        result.content
    );
}

#[tokio::test]
async fn non_file_tool_produces_no_verdict() {
    let home = tempfile::tempdir().expect("home");
    let repo = fake_repo("");
    write_lines(&repo.path().join("src").join("big.rs"), 501);
    let mut agent = agent_in(repo.path(), shipped_registry(home.path()));

    let result = post_tool_result(
        &mut agent,
        "Bash",
        None,
        serde_json::json!({"command": "echo hi"}),
    )
    .await;

    assert!(
        !result.content.contains(HOOK_CONTEXT_MARKER),
        "a Bash call must not be self-checked as an edit: {}",
        result.content
    );
}

#[tokio::test]
async fn script_stays_quiet_when_the_payload_has_no_file_even_if_it_runs() {
    // `if_condition = "*"` bypasses the shipped tool filter, so this asserts the
    // script's own guard rather than the config's.
    let repo = fake_repo("");
    write_lines(&repo.path().join("src").join("big.rs"), 501);
    let hooks = registry_from_toml(
        r#"
[hooks.PostToolUse]
matchers = [{ hooks = [
  { type = "prompt", command = "bash scripts/self-check-file.sh Bash", if_condition = "*", timeout = 5 },
] }]
"#,
    );
    let mut agent = agent_in(repo.path(), hooks);

    let result = post_tool_result(
        &mut agent,
        "Bash",
        None,
        serde_json::json!({"command": "echo hi"}),
    )
    .await;

    assert!(
        !result.content.contains(HOOK_CONTEXT_MARKER),
        "script spoke without a file to check: {}",
        result.content
    );
}

// ---------------------------------------------------------------------------
// The shipped config itself.
// ---------------------------------------------------------------------------

#[test]
fn shipped_hooks_toml_filters_on_if_condition_with_an_explicit_timeout() {
    let path = repo_root().join(".archon").join("hooks.toml");
    let settings = crate::hooks::load_hooks_from_toml(&path).expect(".archon/hooks.toml parses");
    let matchers = settings
        .get(&crate::hooks::HookEvent::PostToolUse)
        .expect("PostToolUse hooks");
    let hooks: Vec<_> = matchers.iter().flat_map(|m| m.hooks.iter()).collect();

    let mut conditions: Vec<&str> = hooks
        .iter()
        .filter(|h| h.command.contains("self-check-file.sh"))
        .map(|h| {
            // `matcher` is a stubbed placeholder that never runs, so a hook
            // relying on it would fire on every tool call.
            assert!(
                h.if_condition.is_some(),
                "self-check hook must filter with if_condition, not matcher"
            );
            assert_eq!(
                h.timeout,
                Some(5),
                "timeout must be explicit, not the 60s default"
            );
            assert!(h.on_failure.is_none(), "a self-check must not block a turn");
            h.if_condition.as_deref().expect("if_condition")
        })
        .collect();
    conditions.sort_unstable();
    assert_eq!(conditions, ["Edit", "NotebookEdit", "Write"]);
}

#[test]
fn shipped_hooks_survive_registry_deduplication() {
    // The registry dedupes by (type, command) and keeps only the last, so three
    // hooks sharing one command string would silently collapse into one.
    let home = tempfile::tempdir().expect("home");
    let registry = crate::hooks::HookRegistry::load_all(&repo_root(), home.path());
    let surviving: Vec<_> = registry
        .summaries()
        .into_iter()
        .filter(|s| s.command.contains("self-check-file.sh"))
        .collect();

    assert_eq!(
        surviving.len(),
        3,
        "expected one surviving self-check per file-writing tool, got {surviving:?}"
    );
    assert!(surviving.iter().all(|s| s.enabled));
    assert!(
        surviving
            .iter()
            .all(|s| s.event == crate::hooks::HookEvent::PostToolUse),
        "PostToolUse is the only event whose output the model reads"
    );
}
