#[path = "declarations_old_commands_tests.rs"]
mod old_commands;
#[path = "../../../crates/archon-workflow/src/v2/write/test_baseline_old_host_tests.rs"]
mod old_host;
use super::*;
const OLD: &[&str] = &[
    // Language toolchains and shells.
    "archon",
    "bash",
    "cargo",
    "deno",
    "go",
    "gradle",
    "just",
    "make",
    "mvn",
    "node",
    "npm",
    "pnpm",
    "pytest",
    "python",
    "python3",
    "sh",
    "tox",
    "yarn",
    // Portable POSIX checks.
    "awk",
    "cmp",
    "diff",
    "find",
    "grep",
    "head",
    "jq",
    "od",
    "printf",
    "sed",
    "sha256sum",
    "shasum",
    "stat",
    "tail",
    "test",
    "wc",
    "[",
];
#[test]
#[ignore = "requires external corpus environment"]
fn r8_before_after() {
    let root =
        std::env::var("S8_R8_ROOT").expect("external task corpus environment must be supplied");
    bench(Path::new(&root), Some(14));
}

#[test]
#[ignore = "requires external corpus environment"]
fn prd017_before_after() {
    let root =
        std::env::var("S8_SECOND_ROOT").expect("external corpus environment must be supplied");
    bench(Path::new(&root), None);
}

fn bench(root: &Path, expected: Option<usize>) {
    let paths = task_files_under(root).unwrap();
    let site_path = crate::command::acceptance_check_policy::command_site_path(root).unwrap();
    if let Some(expected) = expected {
        assert_eq!(paths.len(), expected);
    }
    let mut totals = (0, 0, 0, 0, 0, 0);
    for path in &paths {
        let raw = fs::read_to_string(&path).unwrap();
        let mut task = match parse_task_file(path, &raw) {
            Ok(task) => task,
            Err(error) if expected.is_none() => {
                println!(
                    "{}: command-section bench only; current schema parser: {error}",
                    path.file_name().unwrap().to_string_lossy()
                );
                let yaml = raw
                    .split_once("```yaml")
                    .unwrap()
                    .1
                    .split_once("```")
                    .unwrap()
                    .0;
                let metadata: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
                archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
                    canonical_task_id: metadata["task_id"].as_str().unwrap().into(),
                    source_path: path.to_string_lossy().into(),
                    focused_tests:
                        archon_workflow::task_universe::parsing::declared_focused_test_items(&raw),
                    required_tools: metadata["required_tools"]
                        .as_sequence()
                        .into_iter()
                        .flatten()
                        .map(|tool| tool.as_str().unwrap().to_string())
                        .collect(),
                    ..Default::default()
                }
            }
            Err(error) => panic!("{error}"),
        };
        task.freeze_focused_commands(
            &crate::command::topology_lint::scope_declarations::repository_root(root, root),
            &site_path,
        );
        let commands = task.declared_focused_test_commands();
        let old_commands: Vec<String> = task
            .focused_tests
            .iter()
            .filter_map(|entry| old_commands::focused_test_command(entry))
            .collect();
        let strong = |s: &str| {
            archon_workflow::verifier_strength::verifier_strength_defect(Some(s), None, None)
                .is_none()
        };
        let before = old_commands.iter().any(|s| {
            OLD.contains(
                &s.trim_start_matches('!')
                    .split_whitespace()
                    .next()
                    .unwrap_or_default(),
            ) && strong(s)
        });
        let after = commands.iter().any(|s| strong(s));
        let missing = |old: bool| -> BTreeSet<String> {
            (if old { &old_commands } else { &commands })
                .iter()
                .filter_map(|s| {
                    if old {
                        s.split_whitespace().next()
                    } else {
                        archon_workflow::task_universe::focused_commands::leading_program(s)
                    }
                })
                .filter(|first| {
                    (!old || OLD.contains(first)) && !task.required_tools.iter().any(|t| t == first)
                })
                .map(str::to_string)
                .collect()
        };
        let old_host: Vec<_> = old_commands
            .iter()
            .filter(|command| old_host::host_runnable(command))
            .collect();
        let host_matches = old_host
            .iter()
            .filter(|before| {
                commands
                    .iter()
                    .any(|after| before.split_whitespace().eq(after.split_whitespace()))
            })
            .count();
        assert_eq!(
            old_host.len(),
            host_matches,
            "{} lost formerly host-runnable commands",
            task.canonical_task_id
        );
        totals.4 += old_host.len();
        totals.5 += host_matches;
        totals.0 += usize::from(before);
        totals.1 += usize::from(after);
        totals.2 += old_commands.len();
        totals.3 += commands.len();
        println!(
            "{} runnable {before}->{after}; undeclared {:?}->{:?}",
            task.canonical_task_id,
            missing(true),
            missing(false)
        );
        assert!(
            !before || after,
            "{} lost runnable tests",
            task.canonical_task_id
        );
    }
    println!(
        "TOTAL {} tasks: runnable {}->{}; commands {}->{}; formerly host-runnable frozen {}->{}",
        paths.len(),
        totals.0,
        totals.1,
        totals.2,
        totals.3,
        totals.4,
        totals.5
    );
}
