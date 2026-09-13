use super::*;
use archon_tools::workflow_read_guard::WorkflowReadGuard;
use serde_json::json;
use std::sync::Arc;

fn fixture(limit: u32) -> (tempfile::TempDir, ToolRegistry, ToolContext) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn old() {}\nsecond\n").unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_read::ReadTool));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    registry.register(Box::new(archon_tools::file_edit::EditTool));
    registry.register(Box::new(archon_tools::glob_tool::GlobTool));
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        workflow_read_guard: Some(Arc::new(WorkflowReadGuard::new(limit, 20, false, false))),
        ..Default::default()
    };
    (temp, registry, ctx)
}

#[tokio::test]
async fn workflow_read_guard_dispatch_blocks_after_budget_until_real_write() {
    let (temp, registry, ctx) = fixture(1);
    let path = temp.path().join("a.rs");
    assert!(
        !registry
            .dispatch("Read", json!({"file_path": path}), &ctx)
            .await
            .is_error
    );
    let denied = registry
        .dispatch("Glob", json!({"pattern": "*.rs"}), &ctx)
        .await;
    assert!(denied.is_error && denied.content.contains("read budget exhausted"));
    for content in ["fn old() {}\nsecond\n", " fn old() {}\nsecond\n "] {
        assert!(
            !registry
                .dispatch(
                    "Write",
                    json!({"file_path": path, "content": content}),
                    &ctx
                )
                .await
                .is_error
        );
        assert!(
            registry
                .dispatch("Glob", json!({"pattern": "*.rs"}), &ctx)
                .await
                .is_error
        );
    }
    assert!(
        registry
            .dispatch(
                "Edit",
                json!({"file_path": path, "old_string": "missing", "new_string": "new"}),
                &ctx
            )
            .await
            .is_error
    );
    assert!(
        registry
            .dispatch("Read", json!({"file_path": path}), &ctx)
            .await
            .is_error
    );
    assert!(
        !registry
            .dispatch(
                "Edit",
                json!({"file_path": path, "old_string": "old", "new_string": "new"}),
                &ctx
            )
            .await
            .is_error
    );
    assert!(
        !registry
            .dispatch("Glob", json!({"pattern": "*.rs"}), &ctx)
            .await
            .is_error
    );
}

#[tokio::test]
async fn workflow_read_guard_dedup_is_range_and_hash_aware_with_force_refresh() {
    let (temp, registry, ctx) = fixture(10);
    let path = temp.path().join("a.rs");
    let input = json!({"file_path": path, "offset": 0, "limit": 1});
    let first = registry.dispatch("Read", input.clone(), &ctx).await;
    assert!(first.content.contains("fn old()"));
    let repeat = registry.dispatch("Read", input.clone(), &ctx).await;
    assert!(
        repeat.content.contains("unchanged since your read at call")
            && !repeat.content.contains("fn old()")
    );
    let next = registry
        .dispatch(
            "Read",
            json!({"file_path": path, "offset": 1, "limit": 1}),
            &ctx,
        )
        .await;
    assert!(next.content.contains("second"));
    let refreshed = registry
        .dispatch(
            "Read",
            json!({"file_path": path, "offset": 0, "limit": 1, "force_refresh": true}),
            &ctx,
        )
        .await;
    assert!(refreshed.content.contains("fn old()"));
    std::fs::write(&path, "fn changed() {}\nsecond\n").unwrap();
    assert!(
        registry
            .dispatch("Read", input, &ctx)
            .await
            .content
            .contains("changed")
    );
}

#[tokio::test]
async fn workflow_read_guard_absent_leaves_operator_reads_unchanged() {
    let (temp, registry, mut ctx) = fixture(0);
    ctx.workflow_read_guard = None;
    for _ in 0..3 {
        let result = registry
            .dispatch("Read", json!({"file_path": temp.path().join("a.rs")}), &ctx)
            .await;
        assert!(!result.is_error && result.content.contains("fn old()"));
    }
}

// A harmless sentinel named Bash proves the dispatch guard runs BEFORE execution;
// this test never invokes cargo or any subprocess.
struct ShellSentinel;
#[async_trait::async_trait]
impl Tool for ShellSentinel {
    fn name(&self) -> &str {
        "Bash"
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn description(&self) -> &str {
        "sentinel"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({})
    }
    fn permission_level(&self, _: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        ToolResult::success("executed")
    }
}

#[tokio::test]
async fn workflow_read_guard_shell_inspection_and_release_build_are_not_word_matches() {
    let (_, mut registry, mut ctx) = fixture(0);
    registry.register(Box::new(ShellSentinel));
    for cmd in [
        "cat file",
        "sed -n '1,20p' file",
        "cd src && head -20 lib.rs",
        "ls | head",
        "git diff --stat",
        "rg foo src",
        "cat src 2>/dev/null",
        "rg foo src || true",
        "git -C /repo diff",
        "cat src && echo \"$?\"",
    ] {
        assert!(
            registry
                .dispatch("Bash", json!({"command": cmd}), &ctx)
                .await
                .is_error,
            "{cmd}"
        );
    }
    for cmd in [
        "echo cat",
        "printf 'cargo build --release'",
        "sed -i 's/a/b/' file",
        "cat input > output",
        "cargo check -p demo",
        "echo ls && touch file",
    ] {
        assert_eq!(
            registry
                .dispatch("Bash", json!({"command": cmd}), &ctx)
                .await
                .content,
            "executed",
            "{cmd}"
        );
    }
    for cmd in [
        "cargo build --release",
        "cargo build --release && echo \"$?\"",
        "cd src && cargo build --release",
        "env FOO=1 cargo +stable build --release",
        "cargo build --profile release",
        "cargo build -r",
        "cargo build --release > build.log 2>&1",
    ] {
        assert!(
            registry
                .dispatch("Bash", json!({"command": cmd}), &ctx)
                .await
                .is_error,
            "{cmd}"
        );
    }
    ctx.workflow_read_guard = Some(Arc::new(WorkflowReadGuard::new(0, 20, true, false)));
    assert_eq!(
        registry
            .dispatch("Bash", json!({"command": "cargo build --release"}), &ctx)
            .await
            .content,
        "executed"
    );
    ctx.workflow_read_guard = None;
    assert_eq!(
        registry
            .dispatch("Bash", json!({"command": "cargo build --release"}), &ctx)
            .await
            .content,
        "executed"
    );
}

#[tokio::test]
async fn workflow_read_guard_parallel_calls_share_one_budget_and_clone_retains_it() {
    let (_temp, registry, ctx) = fixture(1);
    let cloned = ctx.clone();
    let (a, b) = tokio::join!(
        registry.dispatch("Glob", json!({"pattern": "*.rs"}), &ctx),
        registry.dispatch("Glob", json!({"pattern": "*.rs"}), &cloned)
    );
    assert_ne!(a.is_error, b.is_error);
    assert!(
        registry
            .dispatch("Glob", json!({"pattern": "*.rs"}), &cloned)
            .await
            .is_error
    );
}

#[test]
fn workflow_read_guard_configuration_defaults_and_overrides_drive_budget() {
    let default: crate::config::GeneratedWorkflowConfig =
        serde_json::from_value(json!({})).unwrap();
    assert_eq!(default.reads_per_write, 20);
    let guard = WorkflowReadGuard::new(
        default.max_reads_before_first_write,
        default.reads_per_write,
        default.allow_release_builds,
        default.allow_git_mutation,
    );
    for _ in 0..40 {
        assert!(guard.before_tool("Glob", &json!({})).is_none());
    }
    assert!(guard.before_tool("Glob", &json!({})).is_some());
    assert!(
        guard
            .before_tool("Bash", &json!({"command":"cargo build --release"}))
            .is_some()
    );
    let custom: crate::config::GeneratedWorkflowConfig = serde_json::from_value(
        json!({"max_reads_before_first_write":1,"reads_per_write":3,"allow_release_builds":true}),
    )
    .unwrap();
    assert_eq!(custom.reads_per_write, 3);
    let guard = WorkflowReadGuard::new(
        custom.max_reads_before_first_write,
        custom.reads_per_write,
        custom.allow_release_builds,
        custom.allow_git_mutation,
    );
    assert!(guard.before_tool("Glob", &json!({})).is_none());
    assert!(guard.before_tool("Glob", &json!({})).is_some());
    assert!(
        guard
            .before_tool("Bash", &json!({"command":"cargo build --release"}))
            .is_none()
    );
}

#[tokio::test]
async fn workflow_read_guard_persists_ranges_outside_workspace_and_reports_io_failure() {
    let (temp, registry, mut ctx) = fixture(10);
    let sidecar = temp.path().join("evidence/reads.jsonl");
    let guard = archon_tools::workflow_read_guard::scope_read_set(sidecar.clone(), async {
        Arc::new(WorkflowReadGuard::new(10, 20, false, false))
    })
    .await;
    ctx.workflow_read_guard = Some(guard);
    let result = registry
        .dispatch(
            "Read",
            json!({"file_path":temp.path().join("a.rs"), "offset":1,"limit":1}),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let record: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&sidecar).unwrap().trim()).unwrap();
    assert_eq!(record["path"], "a.rs");
    assert_eq!(record["offset"], 1);
    assert_eq!(record["limit"], 1);
    let bad_sink = temp.path().join("a.rs/reads.jsonl");
    ctx.workflow_read_guard = Some(
        archon_tools::workflow_read_guard::scope_read_set(bad_sink, async {
            Arc::new(WorkflowReadGuard::new(10, 20, false, false))
        })
        .await,
    );
    let result = registry
        .dispatch("Read", json!({"file_path":temp.path().join("a.rs")}), &ctx)
        .await;
    assert!(
        result.is_error && result.content.contains("read-set"),
        "{}",
        result.content
    );
}

#[test]
fn workflow_read_guard_assignment_chains_consume_budget() {
    for command in ["ROOT=/x; cd $ROOT; sed -n 1,5p f", "export ROOT=/x; cd $ROOT; cat f 2>/dev/null",
        "unset OLD; local ROOT=/x; grep pattern f", "X=1 cat f"] {
        let guard = WorkflowReadGuard::new(1, 20, true, false);
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_none());
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_some(), "{command}");
    }
}
#[test]
fn workflow_read_guard_fallback_blocks_only_inspection_not_progress() {
    let guard = WorkflowReadGuard::new(2, 20, true, true);
    for _ in 0..5 { assert!(guard.before_tool("Bash", &json!({"command":"cargo check"})).is_none()); }
    assert!(guard.before_tool("Bash", &json!({"command":"find . -type f"})).is_some());
    assert!(guard.before_tool("Read", &json!({"file_path":"f"})).is_some());
    for name in ["Write", "Edit", "ApplyPatch", "LargeEditBegin", "LargeEditCommit", "NotebookEdit"] {
        assert!(guard.before_tool(name, &json!({})).is_none(), "{name}");
    }
    for command in ["ROOT=/x; cd $ROOT; sed -n 1,5p f; cargo check", "cargo test", "cargo build --release",
        "python script.py", "tee f", "cat > f", "sed -i 's/old/new/' f", "npm test", "go test ./...", "git apply change.patch"] {
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_none(), "{command}");
    }
}

fn read_ok(guard: &WorkflowReadGuard) -> bool {
    guard.before_tool("Read", &json!({"file_path":"f"})).is_none()
}
fn substantive_write(guard: &WorkflowReadGuard, n: u8) {
    guard.record_write(b"before", format!("after {n}").as_bytes());
}

#[test]
fn workflow_read_guard_each_substantive_write_grants_a_bounded_allowance() {
    let guard = WorkflowReadGuard::new(40, 20, true, false);
    for _ in 0..40 { assert!(read_ok(&guard)); }
    let refused = guard.before_tool("Read", &json!({"file_path":"f"})).unwrap();
    assert!(refused.contains("40 reads, 0 substantive writes") && refused.contains("grants 20 further reads"), "{refused}");
    substantive_write(&guard, 1);
    for _ in 0..20 { assert!(read_ok(&guard)); }
    let refused = guard.before_tool("Read", &json!({"file_path":"f"})).unwrap();
    assert!(refused.contains("20 reads since your last substantive write; 1 write so far"), "{refused}");
    assert!(guard.before_tool("Bash", &json!({"command":"cd src && sed -n 1,5p f"})).is_some());
    assert!(guard.before_tool("Bash", &json!({"command":"ROOT=/x; cd $ROOT; grep pattern f"})).is_some());
    substantive_write(&guard, 2);
    for _ in 0..20 { assert!(read_ok(&guard)); }
    let refused = guard.before_tool("Glob", &json!({"pattern":"*"})).unwrap();
    assert!(refused.contains("2 writes so far"), "{refused}");
    // Unchanged and whitespace-only writes grant nothing.
    guard.record_write(b"same", b"same");
    guard.record_write(b"a b", b" ab ");
    assert!(!read_ok(&guard));
}

#[test]
fn workflow_read_guard_write_class_tools_are_never_refused_in_any_phase() {
    let guard = WorkflowReadGuard::new(1, 1, true, false);
    let writers = ["Write", "Edit", "ApplyPatch", "LargeEditBegin", "LargeEditCommit", "NotebookEdit", "MultiEdit"];
    for phase in 0..3 {
        // Exhaust the phase's allowance, then push calls past the 2x fallback threshold.
        while read_ok(&guard) {}
        for _ in 0..8 { assert!(guard.before_tool("Bash", &json!({"command":"cargo check"})).is_none()); }
        assert!(!read_ok(&guard));
        for name in writers { assert!(guard.before_tool(name, &json!({})).is_none(), "phase {phase} {name}"); }
        substantive_write(&guard, phase);
    }
}

#[test]
fn workflow_read_guard_non_default_reads_per_write_is_honoured() {
    let guard = WorkflowReadGuard::new(2, 3, true, false);
    assert!(read_ok(&guard) && read_ok(&guard) && !read_ok(&guard));
    substantive_write(&guard, 1);
    assert!(read_ok(&guard) && read_ok(&guard) && read_ok(&guard) && !read_ok(&guard));
    let zero = WorkflowReadGuard::new(1, 0, true, false);
    assert!(read_ok(&zero) && !read_ok(&zero));
    substantive_write(&zero, 1);
    assert!(!read_ok(&zero));
}

#[test]
fn workflow_read_guard_refuses_git_mutation_and_keeps_read_only_git() {
    let guard = WorkflowReadGuard::new(40, 20, true, false);
    for (command, verb) in [
        ("git stash push -m x", "git stash push"), ("git stash pop", "git stash pop"), ("cd /tmp && git stash pop -q", "git stash pop"),
        ("git -C /x reset --hard HEAD", "git reset --hard"), ("git checkout -- .", "git checkout"), ("git switch main", "git switch"),
        ("git clean -fd", "git clean"), ("git merge other", "git merge"), ("git commit -am x", "git commit"), ("git branch -D foo", "git branch -D"),
        ("git add -A", "git add"), ("cargo check && git stash pop", "git stash pop"), ("git config user.name x", "git config"),
        ("git stash", "git stash"), ("git rebase -i HEAD~3", "git rebase"), ("git push origin main", "git push"), ("git remote add o u", "git remote add"),
        ("ROOT=/x; cd $ROOT; git reset HEAD~1", "git reset"), ("git apply change.patch", "git apply"), ("git worktree remove w", "git worktree remove"),
        ("git worktree add ../x", "git worktree add"), ("git worktree", "git worktree"), ("git worktree prune", "git worktree prune"),
        ("git worktree list 2>/dev/null | head -3 && git worktree lock w", "git worktree lock"),
    ] {
        let refused = guard.before_tool("Bash", &json!({"command":command})).unwrap_or_else(|| panic!("{command} was allowed"));
        assert!(refused.contains(&format!("{verb} is refused")) && refused.contains("allow_git_mutation"), "{command}: {refused}");
    }
    let reads = ["git status --porcelain", "git diff --stat", "git diff HEAD -- crates/x.rs", "git show HEAD:crates/x.rs", "git log --oneline -3", "git ls-files",
        "git worktree list", "git worktree list --porcelain 2>/dev/null | head -3"];
    for command in reads { assert!(guard.before_tool("Bash", &json!({"command":command})).is_none(), "{command}"); }
    let allowed = ["git rev-parse HEAD", "git stash list", "git stash show -p stash@{0}", "git branch --show-current", "git branch",
        "git config --get user.name", "git config --list", "git remote -v", "git remote show origin", "git reflog", "git", "git -C /x"];
    for command in allowed { assert!(guard.before_tool("Bash", &json!({"command":command})).is_none(), "{command}"); }
    // Read-only git that is inspection-shaped counts toward the read budget; the refused commands never did.
    let counted = WorkflowReadGuard::new(reads.len() as u32, 20, true, false);
    assert!(counted.before_tool("Bash", &json!({"command":"git stash pop"})).is_some());
    for command in reads { assert!(counted.before_tool("Bash", &json!({"command":command})).is_none(), "{command}"); }
    assert!(counted.before_tool("Bash", &json!({"command":"git status"})).unwrap().contains("read budget exhausted"));
    let permitted = WorkflowReadGuard::new(40, 20, true, true);
    assert!(permitted.before_tool("Bash", &json!({"command":"git stash pop"})).is_none());
    assert!(permitted.before_tool("Bash", &json!({"command":"cargo check && git stash pop"})).is_none());
}

/// Budget 1, spent by a Read; the probe is call 2, under the 2x fallback threshold,
/// so only `shell::inspection` decides. Git mutation is permitted so a refusal means "read".
fn bash_is_inspection(command: &str) -> bool {
    let guard = WorkflowReadGuard::new(1, 20, true, true);
    assert!(read_ok(&guard));
    guard.before_tool("Bash", &json!({"command":command})).is_some_and(|r| r.contains("read budget exhausted"))
}

#[test]
fn workflow_read_guard_classifies_common_read_only_shell_forms_as_inspection() {
    for command in [
        "awk '/fn build_report/,/^}/' crates/x.rs | head -70",
        "sed -n \"$(grep -n 'fn x' crates/x.rs | head -1 | cut -d: -f1),+20p\" crates/x.rs",
        "sed -n '/pat/,/pat/p' crates/x.rs", "sed -ne '1,5p' crates/x.rs", "sed -n 's/new /old/p' crates/x.rs",
        "ps aux | grep -i cargo | grep -v grep | head -3", "pgrep -fl rustc | head -3",
        "git rev-parse HEAD", "git stash list", "git stash show -p stash@{0}", "git config --get user.name", "git config -l",
        "git branch", "git branch --show-current", "git remote -v", "git remote show origin", "git blame -L 1,5 crates/x.rs",
        "git rev-list --count HEAD", "git cat-file -p HEAD", "git describe --tags", "git grep -n x -- crates",
        "wc -l < crates/x.rs", "cut -d: -f1 f | sort | uniq -c | tr -d ' '", "sort -u f",
        "stat crates/x.rs", "diff a.rs b.rs", "cmp a.rs b.rs", "file crates/x.rs", "du -sh target", "df -h", "which cargo",
        "type cargo", "date", "basename $PWD", "dirname crates/x.rs", "realpath .", "readlink -f .",
        "find . -name '*.rs'", "find .archon/data -mindepth 2 -maxdepth 2 -type d",
        "env", "printenv HOME", "jq .name package.json", "tree -L 2 crates", "nl f", "column -t f", "xxd f | head", "od -c f", "strings f",
    ] { assert!(bash_is_inspection(command), "{command}"); }
    for command in [
        "awk '{print > \"out.txt\"}' f", "awk -i inplace '{print}' f", "awk '{system(\"touch x\")}' f",
        "sed -i 's/a/b/' f", "sed -n '/x/w out.txt' f", "sed -n '1,3w out' f", "sed -n 's/a/b/w out' f", "sed -n -f script.sed f", "sed 's/a/b/' f",
        "find . -delete", "find . -name '*.rs' -exec rm {} \\;",
        "find .archon/data -mindepth 2 -maxdepth 2 -type d -exec sh -c 'echo \"== $1:\"; ls \"$1\"' _ {} \\;",
        "git stash pop", "git stash", "git config user.name x", "git branch -D x", "git remote add o u", "git checkout -- .", "git diff --output=x",
        "cargo check", "rustc x.rs", "python3 -c 'print(1)'", "node -e '1'", "npm test", "make", "xargs ls", "sh -c 'ls'", "bash -c 'ls'",
        "sort -o out f", "pkill -f cargo", "kill 1", "tee f", "mv a b", "cp a b", "rm f", "mkdir d", "touch f", "chmod +x f",
        "wc -l <(cat f)", "cat <<EOF\nfoo\nEOF", "grep x f && cargo check",
    ] { assert!(!bash_is_inspection(command), "{command}"); }
}

#[test]
fn workflow_read_guard_scratch_redirects_still_count_as_reads() {
    for command in [
        "grep -n x f.rs > /tmp/q.txt; cat /tmp/q.txt",
        "sed -n '1,5p' f.rs > /private/tmp/a.txt 2>&1; cat /private/tmp/a.txt",
        "cat f > \"$TMPDIR/x\"; cat \"$TMPDIR/x\"", "cat f > ${TMPDIR}/x", "grep x f > /dev/null",
        "grep -n x f.rs > /tmp/q.txt", "cat f >> /var/folders/zz/q.log; ls", "cat f 1> /private/var/folders/zz/q; cat f &> /tmp/q",
    ] { assert!(bash_is_inspection(command), "{command}"); }
    for command in [
        "grep -n x f.rs > notes.txt", "grep -n x f.rs > crates/x/out.txt", "cat f > $WT/out.txt", "cat f > ~/out.txt",
        "cat f > /tmpfs/x", "cat f | tee /tmp/x",
    ] { assert!(!bash_is_inspection(command), "{command}"); }
}

#[test]
fn workflow_read_guard_post_write_fallback_bounds_calls_the_classifier_missed() {
    let guard = WorkflowReadGuard::new(40, 20, true, false);
    for _ in 0..40 { assert!(read_ok(&guard)); }
    substantive_write(&guard, 1);
    // `whereis` is fallback-shaped only: `inspection()` never counts it.
    let probe = json!({"command":"whereis cargo"});
    for _ in 0..60 { assert!(guard.before_tool("Bash", &probe).is_none()); }
    let refused = guard.before_tool("Bash", &probe).unwrap();
    assert!(refused.contains("1 write so far") && refused.contains("(61 tool calls since your last substantive write)"), "{refused}");
    assert!(!read_ok(&guard));
    for command in ["cargo test", "python3 script.py", "npm test", "cat > f", "sed -i 's/a/b/' f"] {
        assert!(guard.before_tool("Bash", &json!({"command":command})).is_none(), "{command}");
    }
    for name in ["Write", "Edit", "ApplyPatch", "LargeEditBegin", "LargeEditCommit", "NotebookEdit"] {
        assert!(guard.before_tool(name, &json!({})).is_none(), "{name}");
    }
    substantive_write(&guard, 2);
    assert!(read_ok(&guard));
    assert!(guard.before_tool("Bash", &probe).is_none());
}
