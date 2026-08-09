use super::*;

const TASK: &str = concat!(
    "# TASK-TDL-080 — Coverage Matrix Command\n",
    "\n",
    "```yaml\n",
    "task_id: TASK-TDL-080\n",
    "status: ready\n",
    "implements: [REQ-DL-040, REQ-DL-041, REQ-DL-042]\n",
    "deliverable_contracts:\n",
    "  - kind: required_universe_registry\n",
    "    artifact_path: .archon/coverage/latest.json\n",
    "    typed_verifier_command: archon trading data verify-coverage {artifact_path}\n",
    "```\n",
    "\n",
    "## Files Expected to Change\n",
    "\n",
    "- Existing files only unless implementation requires a new module.\n",
    "- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `src/command/trading_data.rs`.\n",
    "\n",
    "## Focused Tests\n",
    "\n",
    "- Coverage matrix generation test.\n",
    "- CLI parse tests for `data list --json`.\n",
    "- `cargo test -p archon-trading   coverage` when code is touched.\n",
    "\n",
    "## Adversarial Review Notes\n",
    "\n",
    "- unrelated bullet\n",
);

fn binding() -> TaskBinding {
    parse_task_binding(TASK, "tests/TASK-TDL-080.md").expect("parses")
}

#[test]
fn reads_task_id_and_implements_flow_sequence() {
    let b = binding();
    assert_eq!(b.task_id, "TASK-TDL-080");
    assert_eq!(b.implements, ["REQ-DL-040", "REQ-DL-041", "REQ-DL-042"]);
    assert_eq!(b.source_path, "tests/TASK-TDL-080.md");
}

#[test]
fn empty_flow_sequence_is_a_claim_of_nothing() {
    let raw = "```yaml\ntask_id: TASK-X\nimplements: []\n```\n";
    let b = parse_task_binding(raw, "x.md").expect("parses");
    assert!(b.implements.is_empty());
}

#[test]
fn absent_implements_is_not_an_error() {
    let raw = "```yaml\ntask_id: TASK-X\nstatus: ready\n```\n";
    let b = parse_task_binding(raw, "x.md").expect("parses");
    assert!(b.implements.is_empty());
}

#[test]
fn unreadable_implements_fails_closed_naming_the_file() {
    let raw = "```yaml\ntask_id: TASK-X\nimplements:\n  - REQ-DL-001\n```\n";
    let err = parse_task_binding(raw, "tests/broken.md").expect_err("block sequence refused");
    let message = err.to_string();
    assert!(message.contains("tests/broken.md"), "{message}");
    assert!(message.contains("flow sequence"), "{message}");
}

#[test]
fn missing_task_id_fails_closed() {
    let err = parse_task_binding("```yaml\nstatus: ready\n```\n", "n.md").expect_err("refused");
    assert!(err.to_string().contains("task_id"), "{err}");
}

#[test]
fn path_scopes_are_lifted_out_of_prose_bullets() {
    let b = binding();
    assert_eq!(
        b.path_scopes,
        [
            "crates/archon-trading/src/data_lake.rs",
            "src/command/trading_data.rs"
        ]
    );
}

#[test]
fn focused_tests_separate_commands_from_descriptions() {
    let b = binding();
    assert_eq!(
        b.focused_tests,
        vec![
            FocusedTestEntry::Prose("Coverage matrix generation test.".into()),
            // A backticked CLI fragment is a description, not an invocation:
            // `data` is not a runner.
            FocusedTestEntry::Prose("CLI parse tests for `data list --json`.".into()),
            FocusedTestEntry::Command("cargo test -p archon-trading coverage".into()),
        ]
    );
    assert_eq!(b.prose_focused_tests().len(), 2);
}

#[test]
fn verifier_commands_carry_both_origins_and_are_whitespace_normalised() {
    let b = binding();
    assert_eq!(
        b.verifier_commands,
        vec![
            VerifierCommand {
                command: "archon trading data verify-coverage {artifact_path}".into(),
                origin: VerifierOrigin::TypedVerifier,
            },
            VerifierCommand {
                // The declared span had three interior spaces.
                command: "cargo test -p archon-trading coverage".into(),
                origin: VerifierOrigin::FocusedTests,
            },
        ]
    );
}

#[test]
fn sections_after_focused_tests_do_not_leak_in() {
    let b = binding();
    assert!(
        !b.focused_tests
            .iter()
            .any(|e| matches!(e, FocusedTestEntry::Prose(t) if t.contains("unrelated"))),
        "adversarial-review bullet leaked into focused tests"
    );
}

/// A tool the task declared is a runner *for that task*.
///
/// The live failure: five generated specs enforced their own function-length
/// and cyclomatic-complexity budgets with `lizard`, declared it in
/// `required_tools`, and had every one of those entries classified as prose —
/// so the complexity half of the guidance went unverified while the file-size
/// half, which ran through `bash`, was enforced.
#[test]
fn a_declared_tool_counts_as_a_runner_for_that_task() {
    let raw = "\
# TASK-X

```yaml
task_id: TASK-X
implements: [REQ-1]
required_tools: [cargo, bash, lizard]
```

## Focused Tests

- `lizard -l rust -C 15 -L 50 -a 5 src/a.rs`
- `cargo test -p thing`
";
    let binding = parse_task_binding(raw, "tasks/TASK-X.md").expect("parses");
    assert_eq!(binding.required_tools, vec!["cargo", "bash", "lizard"]);
    assert!(
        binding.prose_focused_tests().is_empty(),
        "a declared tool must not be read as prose: {:?}",
        binding.prose_focused_tests()
    );
    assert!(
        binding
            .verifier_commands
            .iter()
            .any(|verifier| verifier.command.starts_with("lizard ")),
        "the declared-tool command must become a matchable verifier"
    );
}

/// Declaring a tool does not make every backticked word a command. The span
/// still has to *start* with the declared tool.
#[test]
fn a_declared_tool_mentioned_mid_sentence_is_still_prose() {
    let raw = "\
# TASK-Y

```yaml
task_id: TASK-Y
implements: [REQ-2]
required_tools: [lizard]
```

## Focused Tests

- Review the `report.md lizard` summary by hand
";
    let binding = parse_task_binding(raw, "tasks/TASK-Y.md").expect("parses");
    assert_eq!(
        binding.prose_focused_tests().len(),
        1,
        "only a leading declared tool makes a span an invocation"
    );
}

/// An unknown tool that the task never declared stays prose — the classifier
/// must not become a heuristic that accepts any first word.
#[test]
fn an_undeclared_unknown_runner_is_still_prose() {
    let raw = "\
# TASK-Z

```yaml
task_id: TASK-Z
implements: [REQ-3]
required_tools: [cargo]
```

## Focused Tests

- `lizard -l rust src/a.rs`
";
    let binding = parse_task_binding(raw, "tasks/TASK-Z.md").expect("parses");
    assert_eq!(
        binding.prose_focused_tests().len(),
        1,
        "a tool the task never declared is not evidence of an invocation"
    );
}

/// A malformed tool list must not fail the file: the requirement claims are the
/// thing traceability exists to read, and losing them costs far more than a
/// little classification precision.
#[test]
fn a_malformed_required_tools_list_does_not_fail_the_task() {
    let raw = "\
# TASK-W

```yaml
task_id: TASK-W
implements: [REQ-4]
required_tools:
  - cargo
  - lizard
```

## Focused Tests

- `cargo test`
";
    let binding = parse_task_binding(raw, "tasks/TASK-W.md").expect("a bad tool list is tolerated");
    assert_eq!(binding.implements, vec!["REQ-4"]);
    assert!(binding.required_tools.is_empty());
}
