//! Issue-21: a read-only workflow call gets the shell admissions and nothing
//! else; the write-capable guard is unchanged.
use super::{
    FocusedTestPlan, GuardMode, REFUSAL_RECORD_KIND, TOOL_CALL_RECORD_KIND, WorkflowReadGuard,
    WorkflowReadGuardSettings,
};
use crate::tool::ToolContext;
use serde_json::{Value, json};

fn bash(guard: &WorkflowReadGuard, command: &str) -> Option<String> {
    guard.before_tool("Bash", &json!({"command": command}))
}

/// Zero reads before the first write, so a write-capable guard refuses the
/// very first inspection; a read-only guard must never notice.
fn tight() -> WorkflowReadGuardSettings {
    WorkflowReadGuardSettings {
        max_reads_before_first_write: 0,
        reads_per_write: 0,
        ..Default::default()
    }
}

#[test]
fn read_only_guard_refuses_release_builds_git_mutation_and_tree_wide_mutators() {
    let guard = WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings::default());
    assert_eq!(guard.mode(), GuardMode::ReadOnly);
    let release =
        bash(&guard, "cargo build --release --bin archon").expect("release build refused");
    assert_eq!(
        release,
        "Release builds are disabled for workflow calls. Use cargo check -p <crate> and focused tests; the operator may enable workflow.generated.allow_release_builds."
    );
    assert!(!release.contains("write-capable"), "{release}");
    let stash = bash(&guard, "git stash").expect("git stash refused");
    assert!(
        stash.contains("git stash is refused") && stash.contains("allow_git_mutation"),
        "{stash}"
    );
    for command in [
        "git checkout -- .",
        "git clean -fd",
        "cd /repo && git stash pop",
    ] {
        assert!(bash(&guard, command).is_some(), "{command} was allowed");
    }
    let fmt = bash(&guard, "cargo fmt --all").expect("tree-wide formatter refused");
    assert!(
        fmt.contains("cargo fmt -p <crate>") && fmt.contains("allow_tree_wide_mutators"),
        "{fmt}"
    );
}

#[test]
fn read_only_guard_honours_the_operator_switches() {
    let guard = WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings {
        allow_release_builds: true,
        allow_git_mutation: true,
        allow_tree_wide_mutators: true,
        ..Default::default()
    });
    for command in ["cargo build --release", "git stash", "cargo fmt --all"] {
        assert!(bash(&guard, command).is_none(), "{command} was refused");
    }
}

#[test]
fn read_only_guard_admits_unlimited_inspection_with_no_budget_message() {
    let guard = WorkflowReadGuard::shell_only(&tight());
    for call in 0..500 {
        for (tool, input) in [
            ("Read", json!({"file_path": "/repo/src/lib.rs"})),
            ("Grep", json!({"pattern": "fn main", "path": "/repo"})),
            ("Glob", json!({"pattern": "**/*.rs"})),
            ("read-own-evidence", json!({"offset": 0, "limit": 3})),
            ("Bash", json!({"command": "cargo test -p archon-tools"})),
            ("Bash", json!({"command": "git status --porcelain"})),
            ("Bash", json!({"command": "sed -n '1,40p' src/lib.rs"})),
            (
                "Bash",
                json!({"command": "cargo check -p archon-tools && cargo test -p archon-tools guard"}),
            ),
        ] {
            assert_eq!(
                guard.before_tool(tool, &input),
                None,
                "call {call}: {tool} {input}"
            );
        }
    }
}

#[test]
fn read_only_guard_never_dedups_reads_credits_writes_orients_or_nudges() {
    let guard = WorkflowReadGuard::shell_only(&tight()).with_focused_tests(FocusedTestPlan::new(
        vec!["cargo test -p archon-tools".into()],
        0,
    ));
    let ctx = ToolContext::default();
    let path = std::path::Path::new("/repo/src/lib.rs");
    for _ in 0..3 {
        assert_eq!(
            guard.read_result(&ctx, path, 0, 10, b"same bytes", false),
            Ok(None)
        );
    }
    guard.record_write(b"before", b"after");
    assert_eq!(guard.orientation(), "");
    guard.after_tool(
        "Bash",
        &json!({"command": "cargo test -p archon-tools"}),
        true,
        "exit 0",
    );
    assert_eq!(guard.completion_message(), None);
    // Past the (zero) grace allowance a write-capable guard would refuse
    // inspection; the read-only guard has no completion to enforce.
    for _ in 0..5 {
        assert_eq!(bash(&guard, "cargo test -p archon-tools"), None);
        assert_eq!(guard.before_tool("Read", &json!({"file_path": path})), None);
    }
}

fn records(sidecar: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(sidecar)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn read_only_guard_records_shell_refusals_and_nothing_else() {
    let temp = tempfile::tempdir().unwrap();
    let sidecar = temp.path().join("read-set.jsonl");
    let guard = super::scope_read_set(sidecar.clone(), async {
        WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings::default())
    })
    .await;
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    assert!(
        guard
            .before_tool("Read", &json!({"file_path": "/repo/src/lib.rs"}))
            .is_none()
    );
    assert_eq!(
        guard.read_result(
            &ctx,
            std::path::Path::new("src/lib.rs"),
            0,
            5,
            b"secret",
            false
        ),
        Ok(None)
    );
    guard.after_tool(
        "Read",
        &json!({"file_path": "/repo/src/lib.rs"}),
        true,
        "ok",
    );
    assert!(bash(&guard, "cargo test -p archon-tools").is_none());
    guard.after_tool(
        "Bash",
        &json!({"command": "cargo test -p archon-tools"}),
        true,
        "exit 0",
    );
    assert!(
        !sidecar.exists(),
        "admitted calls of a read-only guard leave no sidecar"
    );
    assert!(bash(&guard, "cargo build --release --bin archon").is_some());
    let records = records(&sidecar);
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records[0]["kind"], REFUSAL_RECORD_KIND);
    assert_eq!(records[0]["tool"], "Bash");
    assert_eq!(records[0]["head"], "cargo build --release --bin archon");
    assert!(
        records[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Release builds are disabled for workflow calls.")
    );
    assert_eq!(records[1]["kind"], TOOL_CALL_RECORD_KIND);
    assert!(
        records[1]["status"]
            .as_str()
            .unwrap()
            .starts_with("refused: Release builds are disabled")
    );
    assert!(
        !std::fs::read_to_string(&sidecar)
            .unwrap()
            .contains("secret")
    );
}

#[test]
fn write_capable_guard_keeps_its_wording_budget_and_mode() {
    let guard = WorkflowReadGuard::from_settings(&tight());
    assert_eq!(guard.mode(), GuardMode::WriteCapable);
    assert_eq!(
        bash(&guard, "cargo build --release").unwrap(),
        "Release builds are disabled for this write-capable workflow call. Use cargo check -p <crate> and focused tests; the operator may enable workflow.generated.allow_release_builds."
    );
    let budget = guard
        .before_tool("Read", &json!({"file_path": "/repo/src/lib.rs"}))
        .unwrap();
    assert!(budget.contains("read budget exhausted"), "{budget}");
}
