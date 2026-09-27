//! Issue-54: a coder that thrashes after the read wall is stopped, not left
//! to burn its call budget.
use super::{
    MAX_NON_WRITING_CALLS_AFTER_WALL, READ_WALL_THRASH_MARKER, REFUSAL_RECORD_KIND,
    TOOL_CALL_RECORD_KIND, WorkflowReadGuard,
};
use serde_json::{Value, json};

const TERMINAL: &str = "read-wall thrash: 16 non-writing calls after the read budget was exhausted; 0 substantive writes in this session";

fn bash(guard: &WorkflowReadGuard, command: &str) -> Option<String> {
    guard.before_tool("Bash", &json!({"command": command}))
}

fn read(guard: &WorkflowReadGuard) -> Option<String> {
    guard.before_tool("Read", &json!({"file_path": "/repo/src/lib.rs"}))
}

/// A guard with `reads` inspections before the first write, driven to the
/// wall: the last call is the refused read that discovered the exhausted
/// budget, which is the first non-writing call after it.
fn at_the_wall(reads: u32) -> WorkflowReadGuard {
    let guard = WorkflowReadGuard::new(reads, 20, false, false);
    for _ in 0..reads {
        assert_eq!(read(&guard), None);
    }
    let wall = read(&guard).expect("the budget is exhausted");
    assert!(wall.starts_with("read budget exhausted"), "{wall}");
    assert_eq!(guard.terminal_failure(), None);
    guard
}

#[test]
fn fifteen_non_writing_calls_after_the_wall_end_in_a_terminal_refusal() {
    let guard = at_the_wall(3);
    // The live shape: refused reads, then echo / grep -c / echo. Each is
    // one non-writing call; the first fourteen after the wall are answered
    // as before.
    for call in 1..MAX_NON_WRITING_CALLS_AFTER_WALL {
        let verdict = if call % 3 == 0 {
            bash(&guard, "grep -c \"\" src/lib.rs").expect("an inspection stays refused")
        } else {
            assert_eq!(bash(&guard, "echo uu"), None, "call {call}");
            continue;
        };
        assert!(
            verdict.starts_with("read budget exhausted"),
            "call {call}: {verdict}"
        );
        assert_eq!(guard.terminal_failure(), None, "call {call}");
    }
    assert_eq!(bash(&guard, "echo uv").as_deref(), Some(TERMINAL));
    assert_eq!(guard.terminal_failure().as_deref(), Some(TERMINAL));
}

#[test]
fn every_call_after_the_terminal_refusal_is_refused_with_it() {
    let guard = at_the_wall(0);
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    assert_eq!(bash(&guard, "echo uv").as_deref(), Some(TERMINAL));
    for (tool, input) in [
        (
            "Write",
            json!({"file_path": "/repo/src/lib.rs", "content": "x"}),
        ),
        ("Edit", json!({"file_path": "/repo/src/lib.rs"})),
        ("Bash", json!({"command": "cargo test -p archon-tools"})),
        ("Read", json!({"file_path": "/repo/src/lib.rs"})),
    ] {
        assert_eq!(
            guard.before_tool(tool, &input).as_deref(),
            Some(TERMINAL),
            "{tool}"
        );
    }
}

#[test]
fn a_substantive_write_after_the_wall_resets_the_count() {
    let guard = at_the_wall(2);
    for _ in 0..10 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    guard.record_write(b"before", b"after");
    // Twice the cutoff with nothing counted: the wall is lifted, and the
    // twenty post-write reads are the fresh allowance.
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL * 2 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    for _ in 0..20 {
        assert_eq!(read(&guard), None);
    }
    assert_eq!(guard.terminal_failure(), None);
    // Exhausted again: the count starts over at this wall, not at the first.
    assert!(read(&guard).unwrap().starts_with("read budget exhausted"));
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    assert_eq!(
        bash(&guard, "echo uv").as_deref(),
        Some(
            "read-wall thrash: 16 non-writing calls after the read budget was exhausted; 1 substantive write in this session"
        )
    );
}

#[test]
fn an_unchanged_or_whitespace_only_write_does_not_reset_the_count() {
    let guard = at_the_wall(0);
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    guard.record_write(b"same", b"same");
    guard.record_write(b"a b", b"a\n  b");
    assert_eq!(bash(&guard, "echo uv").as_deref(), Some(TERMINAL));
}

#[test]
fn a_build_or_test_command_after_the_wall_is_not_counted_as_thrash() {
    let guard = at_the_wall(1);
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    for command in [
        "cargo test -p archon-tools read_guard",
        "pytest tests/test_guard.py -q",
        "npm test",
        "cargo check -p archon-tools",
        "CARGO_TARGET_DIR=/tmp/t cargo test --bin archon",
    ] {
        assert_eq!(bash(&guard, command), None, "{command}");
        assert_eq!(guard.terminal_failure(), None, "{command}");
    }
    // Progress resets nothing: the next non-writing call is still the one
    // past the cutoff.
    assert_eq!(bash(&guard, "echo uv").as_deref(), Some(TERMINAL));
}

#[test]
fn the_terminal_message_text_is_pinned() {
    assert_eq!(MAX_NON_WRITING_CALLS_AFTER_WALL, 15);
    assert_eq!(READ_WALL_THRASH_MARKER, "read-wall thrash:");
    assert_eq!(
        super::thrash::terminal_message(16, 0, None),
        "read-wall thrash: 16 non-writing calls after the read budget was exhausted; 0 substantive writes in this session"
    );
    assert!(TERMINAL.starts_with(READ_WALL_THRASH_MARKER));
    // The retry classifier must never read this as a transient provider
    // failure or a host timeout.
    for needle in [
        "timed out",
        "timeout",
        "temporar",
        "rate limit",
        "429",
        "500",
        "502",
        "503",
        "504",
    ] {
        assert!(!TERMINAL.contains(needle), "{needle}");
    }
}

#[test]
fn a_read_only_guard_has_no_wall_and_never_turns_terminal() {
    let guard = WorkflowReadGuard::shell_only(&super::WorkflowReadGuardSettings {
        max_reads_before_first_write: 0,
        ..Default::default()
    });
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL * 4 {
        assert_eq!(read(&guard), None);
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    assert_eq!(guard.terminal_failure(), None);
}

fn records(sidecar: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(sidecar)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The terminal refusal reaches the sidecar as an ordinary refusal record,
/// which is how `session_memory` carries it to the next attempt.
#[tokio::test]
async fn the_terminal_refusal_is_recorded_for_the_next_session() {
    let temp = tempfile::tempdir().unwrap();
    let sidecar = temp.path().join("read-set.jsonl");
    let guard = super::scope_read_set(sidecar.clone(), async {
        WorkflowReadGuard::new(0, 20, false, false)
    })
    .await;
    assert!(read(&guard).is_some());
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    assert_eq!(
        bash(&guard, "grep -c \"\" src/lib.rs").as_deref(),
        Some(TERMINAL)
    );
    let records = records(&sidecar);
    let refusals: Vec<&Value> = records
        .iter()
        .filter(|r| r["kind"] == REFUSAL_RECORD_KIND)
        .collect();
    assert_eq!(refusals.len(), 2, "{records:?}");
    assert_eq!(refusals[1]["tool"], "Bash");
    assert_eq!(refusals[1]["head"], "grep -c \"\" src/lib.rs");
    assert_eq!(refusals[1]["reason"], TERMINAL);
    let last = records
        .iter()
        .rev()
        .find(|r| r["kind"] == TOOL_CALL_RECORD_KIND)
        .unwrap();
    assert_eq!(last["status"], format!("refused: {TERMINAL}"));
}

/// Issue-115: a session whose declared focused tests have all passed, past
/// the read wall, that keeps re-running tests instead of returning. Every
/// refusal it is given says to return the envelope, and the cut names what
/// happened: real writes, tests passed, no envelope.
#[test]
fn a_session_done_with_its_tests_is_told_to_return_before_the_cut_names_it() {
    const DECLARED: &str = "cargo test -p x --test a";
    let guard = WorkflowReadGuard::new(2, 3, false, false)
        .with_focused_tests(super::FocusedTestPlan::new(vec![DECLARED.into()], 2));
    guard.record_write(b"fn a() {}", b"fn a() { 1 }");
    for _ in 0..3 {
        assert_eq!(read(&guard), None);
    }
    let wall = read(&guard).unwrap();
    assert!(wall.starts_with("read budget exhausted"), "{wall}");
    assert_eq!(bash(&guard, DECLARED), None);
    guard.after_tool("Bash", &json!({"command": DECLARED}), true, "exit 0");
    assert!(guard.completion_message().is_some());
    // Within the grace allowance, but past the wall: the budget refusal now
    // leads with the instruction instead of "write a deliverable file".
    let over = read(&guard).unwrap();
    assert!(
        over.starts_with("All declared focused tests have passed in this session (1 of 1 at tool call 5). Return the result envelope now."),
        "{over}"
    );
    assert!(
        over.contains("1 substantive write in this session"),
        "{over}"
    );
    assert!(!over.contains("Write a deliverable"), "{over}");
    assert_eq!(bash(&guard, DECLARED), None, "the last grace call");
    let mut refusals = vec![wall, over];
    let terminal = loop {
        let verdict = bash(&guard, DECLARED).expect("past the grace allowance");
        if guard.terminal_failure().is_some() {
            break verdict;
        }
        refusals.push(verdict);
    };
    assert_eq!(refusals.len(), MAX_NON_WRITING_CALLS_AFTER_WALL as usize);
    for refusal in &refusals[1..] {
        assert!(
            refusal.contains("Return the result envelope now."),
            "{refusal}"
        );
    }
    assert_eq!(
        terminal,
        "read-wall thrash: 16 non-writing calls after the read budget was exhausted; 1 substantive write in this session; all 1 declared focused tests had passed at tool call 5 and the result envelope was not returned"
    );
    assert!(terminal.starts_with(READ_WALL_THRASH_MARKER));
}

/// The cut stays as strict for a session that never wrote and never passed
/// its tests: the focused plan alone changes nothing.
#[test]
fn a_declared_plan_that_never_passed_does_not_soften_the_cut() {
    let guard = WorkflowReadGuard::new(0, 20, false, false).with_focused_tests(
        super::FocusedTestPlan::new(vec!["cargo test -p x --test a".into()], 2),
    );
    assert!(read(&guard).unwrap().starts_with("read budget exhausted"));
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    assert_eq!(bash(&guard, "echo uv").as_deref(), Some(TERMINAL));
}

/// Issue-120: a build/test runner behind a process wrapper, and a run of the
/// branch's own declared check (whatever program it names), are progress,
/// not thrash; an idle read or echo after the wall still counts.
#[test]
fn wrapped_runners_and_declared_checks_are_not_thrash_but_idle_calls_are() {
    const DECLARED: &str = "bash scripts/check-lane.sh --fast";
    let guard = WorkflowReadGuard::new(0, 20, false, false)
        .with_focused_tests(super::FocusedTestPlan::new(vec![DECLARED.into()], 2));
    assert!(read(&guard).unwrap().starts_with("read budget exhausted"));
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    for command in [
        "timeout 900 cargo test -p shared --lib",
        "cd /repo && timeout -k 5 600 cargo nextest run -j4",
        "nice -n 10 cargo clippy -p shared",
        "time cargo build -p shared",
        DECLARED,
        "cd /repo && bash scripts/check-lane.sh --fast 2>&1 | tail -5",
    ] {
        assert_eq!(bash(&guard, command), None, "{command}");
        assert_eq!(guard.terminal_failure(), None, "{command}");
    }
    // A wrapper around an idle command is still idle.
    assert_eq!(bash(&guard, "timeout 5 echo uv").as_deref(), Some(TERMINAL));
}

/// A runner is seen past every transparent wrapper spelling: an assignment
/// whose value ends in a wrapper's name, `env` with assignments or options,
/// and GNU `time` options that take a value.
#[test]
fn runners_behind_env_assignments_and_valued_wrapper_options_are_not_thrash() {
    let guard = WorkflowReadGuard::new(0, 20, false, false);
    assert!(read(&guard).unwrap().starts_with("read budget exhausted"));
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 1 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    for command in [
        "PATH=/opt/tools/timeout timeout 900 cargo test -p shared",
        "env RUST_LOG=debug cargo test -p shared",
        "env -i -u TERM HOME=/h cargo test -p shared",
        "time -f %e cargo build -p shared",
        "/usr/bin/time -o /tmp/t --format=%e nice -n 5 make check",
        "timeout --signal=KILL 60 env -- A=1 go test ./...",
    ] {
        assert_eq!(bash(&guard, command), None, "{command}");
        assert_eq!(guard.terminal_failure(), None, "{command}");
    }
    // Wrappers around an idle command are still idle.
    assert_eq!(
        bash(&guard, "env A=1 time -f %e echo uv").as_deref(),
        Some(TERMINAL)
    );
}

/// The declared-check exemption is token-exact: text that merely contains
/// the declared command (a longer argument, an echo of it) is not a run of
/// it and still counts.
#[test]
fn only_a_token_exact_run_of_a_declared_check_is_exempt() {
    const DECLARED: &str = "bash scripts/check-lane.sh --fast";
    let guard = WorkflowReadGuard::new(0, 20, false, false)
        .with_focused_tests(super::FocusedTestPlan::new(vec![DECLARED.into()], 2));
    assert!(read(&guard).unwrap().starts_with("read budget exhausted"));
    // Counted: 1 (the read) + 3 near-misses + 11 echoes = 15, the limit.
    for command in [
        "bash scripts/check-lane.sh --fast-path",
        "echo bash scripts/check-lane.sh --fast",
        "bash scripts/check-lane.sh --fast --extra",
    ] {
        assert_eq!(bash(&guard, command), None, "{command}");
    }
    for _ in 0..MAX_NON_WRITING_CALLS_AFTER_WALL - 4 {
        assert_eq!(bash(&guard, "echo uu"), None);
    }
    // Exact runs, wrapped or chained, are not counted.
    for command in [
        "cd /repo && timeout 60 bash scripts/check-lane.sh --fast 2>&1 | tail -5",
        "env CI=1 bash  scripts/check-lane.sh   --fast > /tmp/out",
    ] {
        assert_eq!(bash(&guard, command), None, "{command}");
    }
    assert_eq!(bash(&guard, "echo uv").as_deref(), Some(TERMINAL));
}

/// The focused-pass tracking reads the same token-exact rule: a run of a
/// longer package name does not mark the declared test passed.
#[test]
fn a_substring_run_does_not_mark_a_declared_focused_test_passed() {
    let guard = WorkflowReadGuard::new(40, 20, false, false).with_focused_tests(
        super::FocusedTestPlan::new(vec!["cargo test -p shared".into()], 2),
    );
    for command in ["cargo test -p shared-extras", "echo cargo test -p shared"] {
        guard.after_tool("Bash", &json!({"command": command}), true, "exit 0");
        assert_eq!(guard.completion_message(), None, "{command}");
    }
    guard.after_tool(
        "Bash",
        &json!({"command": "cd /repo && nice -n 5 cargo test -p shared 2>&1 | tail"}),
        true,
        "exit 0",
    );
    assert!(guard.completion_message().is_some());
}

/// A declared command is matched as declared: a multi-line quoted argument
/// is not whitespace-collapsed into a string no run could ever equal.
#[test]
fn a_multi_line_declared_command_run_verbatim_is_marked_passed() {
    const DECLARED: &str = "python3 -c \"\nimport json\nprint(1)\n\" /tmp/x";
    let guard = WorkflowReadGuard::new(40, 20, false, false)
        .with_focused_tests(super::FocusedTestPlan::new(vec![DECLARED.into()], 2));
    guard.after_tool("Bash", &json!({"command": DECLARED}), true, "exit 0");
    assert!(guard.completion_message().is_some());
}
