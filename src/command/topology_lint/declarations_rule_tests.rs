use super::*;
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;

#[test]
fn focused_commands_use_the_stored_decision() {
    for command in [
        "cargo test -p a --lib",
        "npx jest -t a",
        "MODE=unit custom-check x",
        "! test -f x",
    ] {
        let mut task = WorkflowV2TaskUniverseTask {
            focused_tests: vec![command.into()],
            ..Default::default()
        };
        task.required_tools.extend([
            "cargo".into(),
            "npx".into(),
            "custom-check".into(),
            "test".into(),
        ]);
        task.freeze_focused_commands(Path::new(""), std::ffi::OsStr::new(""));
        assert!(task_has_runnable_test(&task), "{command}");
    }
    for entry in [
        "`mcp__server__tool` with `{}`",
        "The operator checks the output",
    ] {
        let task = WorkflowV2TaskUniverseTask {
            focused_tests: vec![entry.into()],
            ..Default::default()
        };
        assert!(!task_has_runnable_test(&task), "{entry}");
    }
}

fn compare(raw: &str, path: &Path) {
    let site = super::declarations_tests::command_site();
    let path_at_site = std::env::join_paths([site.path()]).unwrap();
    let task = archon_workflow::task_universe::parsing::parse_task_file_with_path(
        path,
        raw,
        Some(&path_at_site),
    )
    .unwrap();
    let binding = archon_knowledge::traceability::tasks::parse_task_binding_with_path(
        raw,
        &path.to_string_lossy(),
        &path_at_site,
    )
    .unwrap();
    let normalized = |commands: Vec<String>| {
        commands
            .iter()
            .map(|s| archon_knowledge::traceability::tasks::normalize_command(s))
            .collect::<BTreeSet<_>>()
    };
    let expected = normalized(task.declared_focused_test_commands());
    let actual = normalized(
        binding
            .focused_tests
            .into_iter()
            .filter_map(|entry| match entry {
                archon_knowledge::traceability::tasks::FocusedTestEntry::Command(s) => Some(s),
                _ => None,
            })
            .collect(),
    );
    assert_eq!(actual, expected, "{}", path.display());
}

#[test]
fn readers_agree_on_inline_fenced_continued_and_nested_commands() {
    let raw = "```yaml\ntask_id: TASK-X-001\ntitle: t\ncomplexity: low\nstatus: ready\nimplements: []\ndepends_on: []\nblocks: []\nrequired_tools: []\nrequired_env_keys: []\ndeliverable_contracts: []\n```\n## Focused Tests\n- `cargo test -p x`\n- `mcp__server__tool`\n### Cases\n```sh\nMODE=x npx jest -t a \\\n --runInBand\n! test -f x\n```\n";
    compare(raw, Path::new("TASK-X-001.md"));
}

#[test]
#[ignore = "requires external corpus environment"]
fn readers_agree_on_r8_when_supplied() {
    let root =
        std::env::var("S8_R8_ROOT").expect("external task corpus environment must be supplied");
    for path in task_files_under(Path::new(&root)).unwrap() {
        compare(&fs::read_to_string(&path).unwrap(), &path);
    }
}

#[test]
fn readers_agree_on_literal_comments_and_blank_continuation_lines() {
    for body in [
        "custom-check \"\n# literal data\n\nvalue\n\"",
        "custom-check --one \\\n# command comment\n--two",
        "custom-check '\n## literal heading\nvalue\n'",
    ] {
        let raw = format!(
            "```yaml\ntask_id: TASK-X-001\ntitle: t\ncomplexity: low\nstatus: ready\nimplements: []\ndepends_on: []\nblocks: []\nrequired_tools: []\nrequired_env_keys: []\ndeliverable_contracts: []\n```\n## Focused Tests\n```sh\n{body}\n```\n"
        );
        compare(&raw, Path::new("TASK-X-001.md"));
    }
}

#[test]
fn knowledge_reader_uses_the_shared_classifier_source() {
    let reader = include_str!("../../../crates/archon-knowledge/src/traceability/tasks.rs");
    let relative = reader
        .lines()
        .find_map(|line| {
            line.strip_prefix("#[path = \"")
                .and_then(|rest| rest.strip_suffix("\"]"))
                .filter(|path| path.ends_with("task_universe_focused_commands.rs"))
        })
        .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let reader_dir = root.join("crates/archon-knowledge/src/traceability");
    assert_eq!(
        reader_dir.join(relative).canonicalize().unwrap(),
        root.join("crates/archon-workflow/src/task_universe_focused_commands.rs")
            .canonicalize()
            .unwrap()
    );
    assert!(!reader_dir.join("tasks/focused_commands.rs").exists());
}
