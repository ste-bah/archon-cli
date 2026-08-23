//! One path, tested end to end: a task file declares its focused tests, the
//! parser keeps them, and the composed author brief hands them over verbatim.
//!
//! The halves are worthless apart. Parsing commands nobody reads changes
//! nothing, and a brief section fed by an empty field is the same blank page
//! the author guessed from before. So both are asserted against the same real
//! task file rather than against each other's fixtures.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::task_universe::parsing::parse_task_file;

/// A real decomposed-PRD task file, with a `## Focused Tests` section written
/// by its author against the repository the task targets.
fn real_task_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/prd-trading-data-lake-ahdm-001")
        .join("TASK-TDL-020-ohlcv-validation-reports.md")
}

fn real_universe() -> WorkflowV2TaskUniverse {
    let path = real_task_path();
    let raw = std::fs::read_to_string(&path).expect("read fixture task file");
    let task = parse_task_file(&path, &raw).expect("fixture task file parses");
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![task],
    }
}

#[test]
fn declared_focused_tests_parse_out_of_the_task_file() {
    let path = real_task_path();
    let raw = std::fs::read_to_string(&path).expect("read fixture task file");
    let task = parse_task_file(&path, &raw).expect("fixture task file parses");

    assert!(
        !task.focused_tests.is_empty(),
        "the task file declares a `## Focused Tests` section"
    );
    assert!(
        task.focused_tests
            .iter()
            .any(|item| item.contains("ohlcv_invalid_timestamp")),
        "declared commands must survive parsing verbatim: {:?}",
        task.focused_tests
    );
}

#[test]
fn composed_author_brief_carries_a_declared_command() {
    let declared = render_declared_focused_tests(&real_universe());
    let brief = compose_author_brief(&[
        ("repo_root", "/repo"),
        ("source_roots", "/repo/prd"),
        ("task_paths", "- TASK-TDL-020: /repo/task.md"),
        ("declared_focused_tests", &declared),
        ("task_waves", "- wave 1: TASK-TDL-020"),
        ("retry_feedback", ""),
        ("learning_context", "{}"),
        ("reference", V3_PRIMITIVE_REFERENCE),
    ]);

    assert!(
        brief.contains("cargo test -p archon-trading ohlcv_invalid_timestamp"),
        "the brief must carry the declared command itself, not a description of it"
    );
    assert!(
        !brief.contains("{declared_focused_tests}"),
        "the placeholder must be declared in the template and substituted"
    );
    // The prose that follows a command in the task file is not runnable.
    assert!(
        !brief.contains("invalid timestamp fixture fails"),
        "only the command belongs in the brief's declared-command list"
    );
}

/// The instruction that produced the invented filter told the author to verify
/// commands it had no way to verify. Its replacement must not reintroduce the
/// demand, and must not name a toolchain: task files declare their own.
#[test]
fn author_brief_never_asks_for_commands_the_author_cannot_verify() {
    let brief = compose_author_brief(&[
        ("repo_root", "/repo"),
        ("source_roots", "/repo/prd"),
        ("task_paths", ""),
        ("declared_focused_tests", ""),
        ("task_waves", ""),
        ("retry_feedback", ""),
        ("learning_context", "{}"),
        ("reference", ""),
    ]);

    assert!(
        !brief.contains("only add focusedTests commands you verified against the repo"),
        "the contradictory instruction must be gone"
    );
    assert!(
        brief.contains("DECLARED FOCUSED TESTS"),
        "the brief must point at the declared commands instead"
    );
    // Whole words: "distrust" is not a toolchain reference.
    let words = brief
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for toolchain in ["cargo", "rust", "rustc", "npm", "pytest", "gradle"] {
        assert!(
            !words.iter().any(|word| word == toolchain),
            "the instruction must stay language-neutral, but names {toolchain}"
        );
    }
}

/// The worked example must BATCH, because a model copies the example over the
/// instruction.
///
/// The brief has always told the author to batch by wave and has always handed
/// it correct wave data — and two live runs emitted one `agent()` per task
/// anyway, `peak_parallelism: 1` on every call. The reason was in the reference
/// itself: its only complete example was `for (const t of tasks) { await
/// agent(...) }`, and `agents([...])` appeared solely as an API signature. An
/// instruction contradicted by the worked example loses.
#[test]
fn the_worked_example_implements_by_wave_rather_than_one_task_at_a_time() {
    let reference = super::V3_PRIMITIVE_REFERENCE;

    assert!(
        reference.contains("await agents("),
        "the reference must show a real batched call, not just its signature"
    );
    assert!(
        reference.contains("const waves = ["),
        "the example must carry the host-computed waves it is told to honour"
    );
    assert!(
        reference.contains("const implOf = {}"),
        "the batch fills a per-task envelope map the follow-up loop reads; \
         without the declaration every authored script throws on its first wave"
    );
    // The evidence the remediation prompt quotes comes from `items[]`, and the
    // task identity from `outcomes[]`. Taking only one of them silently thins
    // the remediation prompt or loses the mapping.
    assert!(
        reference.contains("batch.data.outcomes") && reference.contains("batch.data.items"),
        "the example must read BOTH per-item arrays: outcomes for identity, items for evidence"
    );
    assert!(
        reference.contains("canonical_task_ids"),
        "wave results must be matched by task id, never by array position"
    );
}

/// A batch big enough to look like id-stuffing fails validation, so the
/// reference has to say where the ceiling is.
#[test]
fn the_worked_example_warns_that_an_oversized_wave_must_be_split() {
    let reference = super::V3_PRIMITIVE_REFERENCE;

    assert!(
        reference.contains("HALF OR MORE"),
        "the reference must state the umbrella-claim bound a large wave would trip"
    );
}
