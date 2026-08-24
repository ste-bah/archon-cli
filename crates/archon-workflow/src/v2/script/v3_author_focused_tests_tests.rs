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
        ("curated_lessons", ""),
        (
            "reference",
            &crate::v2::script::render_dialect_reference(Some(&real_universe())),
        ),
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
        ("curated_lessons", ""),
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
    // The stamped reference, because the raw constant now carries the
    // `{example_waves}` placeholder the host fills with this run's real wave
    // groups — the author never sees the unstamped text.
    let reference = crate::v2::script::render_dialect_reference(Some(&real_universe()));
    let reference = reference.as_str();

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

/// The brief must not call the deliverable by the name of a different file.
///
/// The v3 author writes `authored-workflow.js`. The brief opened by asking for
/// "the complete workflow.js orchestration script" — and `workflow.js` is the
/// legacy harness name, which for decomposed runs held a YAML plan record. An
/// author looking for an example of the thing it had just been asked to write
/// searched the project for that name, found plan records under a `.js`
/// extension, read one three times, pulled in the 767 KB metadata sitting
/// beside it, and lost the context it needed to finish.
#[test]
fn the_brief_names_the_file_the_author_actually_writes() {
    let brief = super::V3_AUTHOR_TASK_TEMPLATE;

    assert!(
        brief.contains("authored-workflow.js orchestration script"),
        "the brief must ask for the file the author actually writes"
    );
    assert!(
        !brief.contains(" workflow.js orchestration script"),
        "the legacy harness name must not stand in for the authored script"
    );
}

/// Hunting for a dialect example is what walks the author into finished runs.
#[test]
fn the_brief_says_the_reference_is_the_only_example_and_keeps_the_author_out_of_run_dirs() {
    let brief = super::V3_AUTHOR_TASK_TEMPLATE;

    assert!(
        brief.contains("THE DIALECT REFERENCE BELOW IS THE ONLY EXAMPLE THERE IS"),
        "the author must be told not to go searching for an example"
    );
    assert!(
        brief.contains(".archon/workflows/"),
        "the brief must name the directory that holds previous runs, so the author knows what to avoid"
    );
    // `.archon/` also holds agent and skill definitions, docs, tools, and the
    // project artifact root the tasks write into. Warning the author off the
    // whole tree would blind it to the target it is orchestrating work against.
    assert!(
        brief.contains("The REST of `.archon/` is ordinary project material"),
        "the exclusion must be scoped to run directories, never to all of .archon/"
    );
}

/// A write agent must never be handed an absolute repository path.
///
/// The host already scopes a write branch to its own git checkout:
/// `run_one_worktree_branch` sets `repository_root` to the branch's
/// `workspace_root` in preference to the run's canonical root. The prompt used
/// to contradict it — the worked example carried a literal `Repository root:
/// <repo root>`, the author substituted the canonical tree from its brief, and
/// the agent obeyed the prompt over its stage input.
///
/// The consequence is not a wrong path, it is silent loss of isolation. Live
/// run wf-7c86a0af: TASK-TDL-010 edited `crates/archon-trading/tests/*` in the
/// real repository, so its worktree patch was 0 bytes, the write coordinator
/// had nothing to inspect, and the task was accepted with no ownership check,
/// no overlap guard, and no record of what changed. The sibling item in the
/// same batch happened to work inside its worktree, had its patch inspected,
/// and was correctly rejected for one out-of-scope path.
#[test]
fn the_worked_example_never_hands_a_write_agent_an_absolute_repository_root() {
    let reference = crate::v2::script::render_dialect_reference(Some(&real_universe()));

    assert!(
        !reference.contains("Repository root: <repo root>"),
        "the example must not invite a literal repository path into a write prompt"
    );
    assert!(
        reference.contains("repository_root in YOUR OWN stage input"),
        "the example must send the agent to its host-stamped checkout instead"
    );
    assert!(
        reference.contains("NEVER paste an absolute repository path"),
        "the prohibition must be explicit, not implied by the positive instruction"
    );
}

/// The rule states the consequence, because the mechanism is not guessable.
#[test]
fn the_brief_explains_why_an_absolute_repository_path_breaks_confinement() {
    let brief = super::V3_AUTHOR_TASK_TEMPLATE;

    assert!(
        brief.contains("DO NOT write an absolute repository path into the prompt"),
        "the implement rule must carry the prohibition"
    );
    for consequence in ["isolated git checkout", "patch is empty", "bypassed"] {
        assert!(
            brief.contains(consequence),
            "the rule must say what goes wrong, missing: {consequence}"
        );
    }
}
