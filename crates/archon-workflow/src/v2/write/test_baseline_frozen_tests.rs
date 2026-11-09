use super::*;
#[test]
fn membership_is_tool_free_and_normalizes_only_whitespace() {
    for command in [
        "python3 -m unittest test_fail",
        "mvn -Dtest=CalculatorTest test",
        "check --scope a && other 'x'",
    ] {
        let frozen = FrozenCommands::from_commands(&[command.into()]);
        assert!(host_runnable(command, &frozen));
        assert!(host_runnable(&command.replace(' ', "  "), &frozen));
        assert!(!host_runnable(&format!("{command} extra"), &frozen));
    }
}
#[test]
fn absent_universe_is_distinct_from_an_empty_frozen_set() {
    assert!(matches!(
        FrozenCommands::default().check("x"),
        Err(CommandNotRun::NoUniverse)
    ));
    assert!(matches!(
        FrozenCommands::from_commands(&[]).check("x"),
        Err(CommandNotRun::NotFrozen)
    ));
    assert!(!host_runnable("", &FrozenCommands::from_commands(&[])));
}
#[test]
fn every_tasks_declarations_supply_the_frozen_set() {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: ["check-a --one", "check-b --two", "check-c --three"]
            .iter()
            .map(|command| {
                let mut task = crate::task_universe::WorkflowV2TaskUniverseTask {
                    focused_tests: vec![format!("`{command}`")],
                    required_tools: vec![command.split_whitespace().next().unwrap().into()],
                    ..Default::default()
                };
                task.freeze_focused_commands(std::path::Path::new(""), std::ffi::OsStr::new(""));
                task
            })
            .collect(),
    };
    let frozen = FrozenCommands::from_universe(Some(&universe));
    for command in ["check-a --one", "check-b --two", "check-c --three"] {
        assert!(host_runnable(command, &frozen));
    }
}

#[test]
fn whitespace_matches_resolve_to_the_literal_frozen_text() {
    for command in ["check-a 'one two'", "check-b --one", "MODE=x check-c --two"] {
        let frozen = FrozenCommands::from_commands(&[command.into()]);
        let reported = command.replace(' ', "  ");
        assert_eq!(frozen.command(&reported).unwrap(), command);
    }
    let commands = ["check 'one two'", "check 'one  two'", "check 'one   two'"].map(str::to_string);
    let frozen = FrozenCommands::from_commands(&commands);
    for command in &commands {
        assert_eq!(frozen.command(command).unwrap(), command);
    }
}

#[path = "test_baseline_old_host_tests.rs"]
mod old_host;
#[test]
#[ignore = "requires external recorded run environment"]
fn recorded_host_commands_keep_their_frozen_authorization() {
    let root = std::env::var("S8_RUN_ROOT").expect("external corpus environment must be supplied");
    let root = std::path::Path::new(&root);
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("generated-metadata.json")).unwrap())
            .unwrap();
    let universe: WorkflowV2TaskUniverse =
        serde_json::from_value(metadata["task_universe"].clone()).unwrap();
    let frozen = FrozenCommands::from_universe(Some(&universe));
    let mut commands = BTreeSet::new();
    for file in std::fs::read_dir(root.join("baseline-tests/run-base")).unwrap() {
        let file = file.unwrap().path();
        let run: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        let command = run["command"].as_str().unwrap();
        if old_host::host_runnable(command) {
            commands.insert(command.to_string());
        }
    }
    let after = commands
        .iter()
        .filter(|command| host_runnable(command, &frozen))
        .count();
    println!(
        "RECORDED HOST: {} unique formerly runnable commands; {after} frozen matches; drop {}",
        commands.len(),
        commands.len() - after
    );
    assert!(!commands.is_empty());
    assert_eq!(commands.len(), after);
}

#[test]
fn frozen_decision_survives_a_different_process_path() {
    if let Some(raw) = std::env::var_os("S8_FROZEN_SET") {
        let universe: WorkflowV2TaskUniverse =
            serde_json::from_str(&raw.to_string_lossy()).unwrap();
        let frozen = FrozenCommands::from_universe(Some(&universe));
        assert!(host_runnable("check --one", &frozen));
        assert!(!host_runnable("missing --two", &frozen));
        return;
    }
    let site = tempfile::tempdir().unwrap();
    let executable = site.path().join("check");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let mut task = crate::task_universe::WorkflowV2TaskUniverseTask {
        focused_tests: vec!["`check --one`".into(), "`missing --two`".into()],
        ..Default::default()
    };
    let path = std::env::join_paths([site.path()]).unwrap();
    task.freeze_focused_commands(site.path(), &path);
    std::fs::remove_file(executable).unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![task],
    };
    let name = format!(
        "{}::frozen_decision_survives_a_different_process_path",
        module_path!().split_once("::").unwrap().1
    );
    for path in [std::ffi::OsString::new(), std::env::var_os("PATH").unwrap()] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name])
            .env("PATH", path)
            .env("S8_FROZEN_SET", serde_json::to_string(&universe).unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn a_declared_program_to_be_created_remains_authorized() {
    let root = tempfile::tempdir().unwrap();
    let mut task = crate::task_universe::WorkflowV2TaskUniverseTask {
        focused_tests: vec!["`./checks/run`".into()],
        files_expected_to_change: vec!["checks/run".into()],
        required_tools: vec!["./checks/run".into()],
        ..Default::default()
    };
    task.freeze_focused_commands(root.path(), std::ffi::OsStr::new(""));
    assert_eq!(task.declared_focused_test_commands(), ["./checks/run"]);
    assert!(!root.path().join("checks/run").exists());
}
