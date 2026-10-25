//! Prompt rendering and task-derived candidate selection tests.

use super::*;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;
use std::path::Path;
fn scope(
    set: &crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
) -> AuthorScope {
    AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd)
        .expect("the task set's repository record is believed")
}
fn write_task_declaration(tasks: &Path, files: &str, implements: &str) {
    std::fs::create_dir_all(tasks).unwrap();
    std::fs::write(
        tasks.join("TASK-SET-001.md"),
        format!("# TASK-SET-001\n\n```yaml\ntask_id: TASK-SET-001\ntitle: Test task\ncomplexity: low\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [{implements}]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- {files}\n"),
    )
    .unwrap();
}

#[test]
#[rustfmt::skip]
fn baseline_prompt_leads_with_guard_template_and_verified_candidates() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repository");
    let project = root.path().join("project");
    std::fs::create_dir_all(repository.join("tests")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(repository.join("tests/guard.txt"), "guard").unwrap();
    std::fs::write(project.join("src/core.txt"), "core").unwrap();
    std::fs::write(project.join("src/unrelated.txt"), "other").unwrap();
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];
    let mut scope = scope(&set);
    scope.repository_root = repository.clone();
    std::fs::create_dir_all(project.join("tests")).unwrap();
    std::fs::write(project.join("tests/guard.txt"), "guard").unwrap();
    scope.project_root = project;
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![
            WorkflowV2TaskUniverseTask {
                implements: vec!["REQ-A".into()],
                files_expected_to_change: vec!["src/core.txt".into(), "src/missing.txt".into()],
                focused_tests: vec!["run tests/guard.txt".into()],
                ..Default::default()
            },
            WorkflowV2TaskUniverseTask {
                implements: vec!["REQ-B".into()],
                files_expected_to_change: vec!["src/unrelated.txt".into()],
                ..Default::default()
            },
        ],
    };
    let note = "check 'AC-F-001': the criterion already holds on the pre-implementation tree; write a regression guard; data the check writes itself is never evidence";
    let prompt = super::author::author_prompt(&scope, &frozen, &[note.into()], 1);
    let first = prompt.lines().next().unwrap();
    assert!(first.starts_with("The criterion already holds on the pre-implementation tree"));
    assert!(first.contains("Write a regression guard"));
    assert!(first.contains("name the deciding file by its literal path relative to the check's working directory"));
    assert!(first.contains("The check is a valid regression guard only when its pass or fail is decided by that test and it fails whenever the test fails"));
    assert!(first.contains("Naming a file or asserting that it exists does not make it the deciding assertion."));
    assert!(first.contains("Its content is shown to the judge only when it is supplied in named_files"));
    assert!(first.contains("A duplicated or relocated implementation can refute the guard"));
    assert!(first.contains("The host may prove the guard by moving aside a named input it actually moves and requiring the check to fail"));
    assert!(!prompt.contains("the named file's content is shown to the judge, which judges the check"));
    assert!(!prompt.contains("the host moves the named input aside"));
    let template = prompt.find("Template:").unwrap();
    let entry = prompt.find("Re-author exactly one").unwrap();
    assert!(template < entry, "{prompt}");
    assert!(prompt.contains("project root: src/core.txt"), "{prompt}");
    assert!(prompt.contains("project root: tests/guard.txt"), "{prompt}");
    assert!(!prompt.contains("src/missing.txt"), "{prompt}");
    assert!(!prompt.contains("src/unrelated.txt"), "{prompt}");
    assert!(prompt.contains(
        "Do not repeat the refused approach: it relied on data created during the check"
    ));
    let replaced = prompt
        .lines()
        .find(|line| line.starts_with("The entry being replaced"))
        .unwrap();
    assert!(!replaced.contains("\"check\":"), "{replaced}");
    assert!(prompt.contains("\"covers\":[\"REQ-A\"]"));
}

#[test]
fn non_baseline_prompt_keeps_the_refused_check_verbatim() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = set.contract().acceptance[0].clone();
    let author_scope = scope(&set);
    let notes = &["judge rejection: failed to prove behavior".into()];
    let prompt = super::author::author_prompt(&author_scope, &frozen, notes, 1);
    let current = serde_json::json!({
        "id": frozen.id,
        "criterion": frozen.criterion,
        "check": frozen.check,
        "gap_permitted": frozen.gap_permitted,
    });
    let expected = [
        format!(
            "Re-author exactly one acceptance entry of a frozen acceptance contract: {}. The findings below explain why the current entry needs revision. Author ONLY this entry, not the whole contract.",
            frozen.id
        ),
        format!("Read the PRD at {}.", author_scope.prd_path.display()),
        format!(
            "The code repository is {}. It is the ONLY place to verify source paths, test names, module layout and whether a file exists: a repository path you name must be one you observed there. Never descend into dependency, build-output or earlier-run directories.",
            author_scope.repository_root.display()
        ),
        format!(
            "{} is the project root. It holds the PRD and the task root; read source under the repository root alone.",
            author_scope.project_root.display()
        ),
        crate::command::workflow_task_set::live_root::check_path_rule(
            &author_scope.repository_root.display().to_string(),
            &author_scope.project_root.display().to_string(),
        ),
        "Return one JSON object with id, criterion, check, gap_permitted, judgment. The two examples below are ENTRIES showing the two check shapes; your reply is one such entry and nothing around it.".to_string(),
        super::ENTRY_SHAPES.to_string(),
        "For a command check, choose EXACTLY ONE command form. Block form: the JSON check object holds kind, cwd and \"command_block\": true, with NO \"command\" field, followed by exactly one non-empty fence whose info string is exactly 'check'. Inline form: the JSON check object holds kind, cwd and \"command\" only; command is one short line without quotes or backslashes. Include no command_block and no fence.".to_string(),
        "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges the entry adversarially against the criterion, then runs it once against the current tree: a check whose own script crashes (a syntax error, an undefined name, a call that does not match a helper it defines) is returned to you. Repairing a crash never licenses weakening the check: keep every assertion, fix only the script defect.".to_string(),
        "The host also runs the check on the tree before any implementation, where it must fail. If it passes there, data the check writes itself is never evidence; movable inputs must exist under the check's selected root before the run. To guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name the deciding file by its literal path relative to the check's working directory. The check is a valid regression guard only when its pass or fail is decided by that test and it fails whenever the test fails. The host moves aside only inputs it actually moves, never the check's own script, a program it runs, a directory it changes into or a build manifest; the check must still fail on its own assertion, not by crashing, and a move-aside failure proves the guard and is never a counterexample. The named file's assertions are known to the judge only when its content is supplied in named_files. A named deciding input that is absent is behaviour not implemented yet and is judged by the general rules; a named deciding input whose content is not supplied has unknown assertions, so the check cannot be accepted as a regression guard on assumed content. Read the deciding file by its named path.".to_string(),
        format!(
            "Use the exact id {}. Criterion and judgment are host-owned placeholders; gap_permitted stays {}.",
            frozen.id, frozen.gap_permitted
        ),
        format!("The entry being replaced: {current}"),
        format!(
            "Attempt 1; the host stops after {} consecutive attempts that repair no check. Findings to fix, oldest first:\n- {}",
            super::REAUTHOR_ATTEMPTS,
            notes.join("\n- ")
        ),
        "Return exactly the entry: for inline form, output only the JSON entry (raw or in one `json` fence); for block form, output the JSON entry (raw or in one `json` fence) followed by exactly one non-empty fence labelled `check`. Emit no prose, explanations, headings or other fences.".to_string(),
        "Do not run commands or write files.".to_string(),
    ]
    .join("\n");
    assert_eq!(prompt, expected);
    let replaced = prompt
        .lines()
        .find(|line| line.starts_with("The entry being replaced"))
        .unwrap();
    assert!(replaced.contains("\"check\":"), "{replaced}");
    let entry: serde_json::Value = serde_json::from_str(
        replaced
            .strip_prefix("The entry being replaced: ")
            .expect("the replacement entry prefix"),
    )
    .expect("the replacement entry is JSON");
    assert_eq!(entry["check"], serde_json::to_value(&frozen.check).unwrap());
}

#[test]
fn candidate_selection_returns_none_found_without_a_matching_existing_path() {
    use archon_workflow::task_universe::WorkflowV2TaskUniverse;

    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let mut scope = scope(&set);
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![],
    };
    assert_eq!(
        super::author::candidate_files(&scope, &set.contract().acceptance[0]),
        "none found"
    );
}

#[test]
fn resume_scope_builder_carries_frozen_task_candidates() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let candidate = set.project.path().join("src/declared.rs");
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::write(&candidate, "declared").unwrap();
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];
    write_task_declaration(&set.tasks, "src/declared.rs", "REQ-A");
    let run_metadata = set
        .project
        .path()
        .join(".archon/workflows/run/v2/generated-metadata.json");
    std::fs::create_dir_all(run_metadata.parent().unwrap()).unwrap();
    std::fs::write(&run_metadata, r#"{"schema_version":"test"}"#).unwrap();
    let scope = crate::command::workflow_decompose_remediation::resume_author_scope(
        set.project.path(),
        &set.tasks,
        &set.prd,
    )
    .unwrap();
    assert_eq!(
        super::author::candidate_files(&scope, &frozen),
        "project root: src/declared.rs"
    );
}

#[test]
fn task_set_scope_loads_candidates_from_frozen_task_files() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let candidate = set.project.path().join("src/from-task-file.rs");
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::write(&candidate, "declared").unwrap();
    write_task_declaration(&set.tasks, "src/from-task-file.rs", "REQ-A");
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];

    assert_eq!(
        super::author::candidate_files(&scope(&set), &frozen),
        "project root: src/from-task-file.rs"
    );
}

#[test]
fn empty_frozen_task_set_has_no_candidates() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];
    assert_eq!(
        super::author::candidate_files(&scope(&set), &frozen),
        "none found"
    );
}

#[test]
fn repository_candidate_only_in_live_tree_is_dropped_from_baseline_candidates() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repository");
    std::fs::create_dir_all(repository.join("src")).unwrap();
    std::fs::write(repository.join("src/base.rs"), "base").unwrap();
    let git = |args: &[&str]| {
        let output = archon_shell::spawn::command("git")
            .arg("-C")
            .arg(&repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "baseline"]);
    let commit = git(&["rev-parse", "HEAD"]);
    std::fs::write(repository.join("src/only-live.rs"), "live").unwrap();

    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let mut scope = scope(&set);
    scope.repository_root = repository.clone();
    scope.baseline =
        Some(crate::command::workflow_task_set::executability::Baseline { commit, repository });
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            implements: vec!["REQ-A".into()],
            files_expected_to_change: vec!["src/only-live.rs".into()],
            ..Default::default()
        }],
    };
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];
    assert_eq!(
        super::author::candidate_files(&scope, &frozen),
        "none found"
    );
}

#[test]
fn absolute_repository_candidate_is_relative_and_absolute_outside_roots_is_dropped() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repository");
    std::fs::create_dir_all(repository.join("src")).unwrap();
    std::fs::write(repository.join("src/base.rs"), "base").unwrap();
    let git = |args: &[&str]| {
        let output = archon_shell::spawn::command("git")
            .arg("-C")
            .arg(&repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "baseline"]);
    let commit = git(&["rev-parse", "HEAD"]);
    let outside = root.path().join("outside.rs");
    std::fs::write(&outside, "outside").unwrap();

    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let mut scope = scope(&set);
    std::fs::create_dir_all(scope.project_root.join("src")).unwrap();
    std::fs::write(scope.project_root.join("src/project.rs"), "project").unwrap();
    scope.repository_root = repository.clone();
    scope.baseline =
        Some(crate::command::workflow_task_set::executability::Baseline { commit, repository });
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            implements: vec!["REQ-A".into()],
            files_expected_to_change: vec![
                format!(
                    "{} — exists (1 lines)",
                    scope.repository_root.join("src/base.rs").display()
                ),
                format!(
                    "{} — exists (1 lines)",
                    scope.project_root.join("src/project.rs").display()
                ),
                outside.display().to_string(),
            ],
            ..Default::default()
        }],
    };
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];

    assert_eq!(
        super::author::candidate_files(&scope, &frozen),
        "project root: src/project.rs, repository root: src/base.rs"
    );
}

#[test]
fn candidate_paths_drop_curdir_and_deduplicate_covering_tasks() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let candidate = set.project.path().join("src/x.rs");
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::write(candidate, "source").unwrap();
    let mut scope = scope(&set);
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![
            WorkflowV2TaskUniverseTask {
                implements: vec!["REQ-A".into()],
                files_expected_to_change: vec!["./src/x.rs".into()],
                ..Default::default()
            },
            WorkflowV2TaskUniverseTask {
                implements: vec!["REQ-A".into()],
                files_expected_to_change: vec!["src/x.rs".into()],
                ..Default::default()
            },
        ],
    };
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];

    assert_eq!(
        super::author::candidate_files(&scope, &frozen),
        "project root: src/x.rs"
    );
}

#[test]
fn nested_repository_candidate_uses_forward_slashes() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repository");
    std::fs::create_dir_all(repository.join("a/b")).unwrap();
    std::fs::write(repository.join("a/b/c.rs"), "source").unwrap();
    let git = |args: &[&str]| {
        let output = archon_shell::spawn::command("git")
            .arg("-C")
            .arg(&repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "baseline"]);
    let commit = git(&["rev-parse", "HEAD"]);

    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let mut scope = scope(&set);
    scope.repository_root = repository.clone();
    scope.baseline =
        Some(crate::command::workflow_task_set::executability::Baseline { commit, repository });
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            implements: vec!["REQ-A".into()],
            files_expected_to_change: vec!["a/b/c.rs".into()],
            ..Default::default()
        }],
    };
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];

    assert_eq!(
        super::author::candidate_files(&scope, &frozen),
        "repository root: a/b/c.rs"
    );
}

#[cfg(unix)]
#[test]
fn project_candidate_with_a_symlinked_directory_is_dropped() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let target = set.project.path().join("actual");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("declared.rs"), "source").unwrap();
    std::os::unix::fs::symlink("actual", set.project.path().join("src-link")).unwrap();
    let mut scope = scope(&set);
    scope.task_universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            implements: vec!["REQ-A".into()],
            files_expected_to_change: vec!["src-link/declared.rs".into()],
            ..Default::default()
        }],
    };
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.covers = vec!["REQ-A".into()];
    assert_eq!(
        super::author::candidate_files(&scope, &frozen),
        "none found"
    );
}

#[test]
fn baseline_prompt_instruction_has_no_tool_or_language_names() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = set.contract().acceptance[0].clone();
    let prompt = super::author::author_prompt(
        &scope(&set),
        &frozen,
        &["the criterion already holds on the pre-implementation tree".into()],
        1,
    );
    let guidance = prompt
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    for forbidden in [
        "cargo",
        "rustc",
        "cargo.toml",
        "npm",
        "pnpm",
        "yarn",
        "pytest",
        "python",
        "gradle",
        "mvn",
        "anthropic",
        "claude",
        "openai",
        "trading",
        "prd-",
    ] {
        assert!(
            !guidance.contains(forbidden),
            "found {forbidden:?} in {guidance}"
        );
    }
    assert!(
        guidance.contains("content is shown to the judge only when it is supplied in named_files")
    );
    assert!(guidance.contains("content is not supplied, its assertions are unknown"));
}
