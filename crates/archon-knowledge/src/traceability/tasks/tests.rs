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
