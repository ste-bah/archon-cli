use super::*;

#[test]
fn site_parity_existence_operands_are_not_inline_programs() {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for command in [
            "test -e ../missing/input".to_owned(),
            "[ -e ../missing/input ]".to_owned(),
            "[[ -e ../missing/input ]]".to_owned(),
            "timeout 5 test -e ../missing/input".to_owned(),
            "cat -e ../missing/input".to_owned(),
            format!("test -e ../{name}/input"),
            format!("test -e ../missing/input && test -e ../{name}/input"),
        ] {
            let actual = decisions(&set, root, &command, cwd);
            if actual != [false; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected refusal"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn assert_command_site_parity(commands: &[&str]) {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::create_dir_all(root.join("checks")).unwrap();
    std::fs::write(root.join("input"), "ready").unwrap();
    std::fs::write(root.join("checks/entry"), "ready").unwrap();
    std::fs::write(root.join("checks/output.log"), "../missing/input\n").unwrap();
    std::fs::write(
        root.join("checks/script.sh"),
        "#!/bin/sh\ncd checks; exec ./verify \"$1\"\n",
    )
    .unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let controls = [
        "timeout 5 cat ../missing/input".to_owned(),
        format!("nice cat ../{name}/input"),
        "nohup cat ../missing/input".to_owned(),
        "env A=ready cat ../missing/input".to_owned(),
        "stdbuf -oL cat ../missing/input".to_owned(),
        "timeout 5 sh -c 'cat ../missing/input'".to_owned(),
        "nice -n 2 sh -c 'cat ../missing/input'".to_owned(),
        "env A=ready sh -c 'cat ../missing/input'".to_owned(),
        "env -C checks cat ../input; cat ../missing/input".to_owned(),
    ];
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for (command, valid) in commands
            .iter()
            .map(|command| (*command, true))
            .chain(controls.iter().map(|command| (command.as_str(), false)))
        {
            let actual = decisions(&set, root, command, cwd);
            if actual != [valid; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected {valid}"));
            }
        }
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: controls[0].clone(),
            cwd,
        };
        let defect = archon_workflow::task_set_contract::acceptance_site_path_defect(
            &candidate, root, root, false,
        )
        .unwrap();
        assert!(defect.contains("../missing/input"), "{defect}");
        assert!(defect.contains("site does not create"), "{defect}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn site_parity_environment_directory_options_defer_relative_operands() {
    assert_command_site_parity(&[
        "env -C checks ./verify ../input",
        "env --chdir=checks sh -c './verify ../input'",
        "env -Cchecks ./verify ../input",
    ]);
}

#[test]
fn site_parity_repository_directory_options_defer_relative_operands() {
    assert_command_site_parity(&[
        "git -C checks hash-object ../input",
        "git --git-dir=checks hash-object ../input",
        "probe --cwd checks ../input",
        "probe --work-tree=checks ../input",
        "probe --prefix checks ../input",
        "probe --manifest-path checks ../input",
        "probe --root=checks ../input",
    ]);
}

#[test]
fn site_parity_build_directory_options_defer_relative_operands() {
    assert_command_site_parity(&[
        "make -C checks -f ../input",
        "make --directory checks -f ../input",
    ]);
}

#[test]
fn site_parity_script_consumed_arguments_have_unknown_directory() {
    assert_command_site_parity(&[
        "sh checks/script.sh ../input",
        "python3 checks/script.sh ../input",
        "node checks/script.sh ../input",
        "./checks/script.sh ../input",
        "timeout 5 sh checks/script.sh ../input",
        "nice env A=ready sh checks/script.sh ../input",
    ]);
}

#[test]
fn site_parity_directory_dispatch_defers_child_operands_and_programs() {
    assert_command_site_parity(&[
        "find checks/entry -maxdepth 0 -execdir ./verify ../input \\;",
        "find checks/entry -maxdepth 0 -execdir sh -c './verify ../input' \\;",
        "find checks/entry -maxdepth 0 -okdir ./verify ../input \\;",
        "xargs ./verify ../input",
        "parallel sh -c './verify ../input'",
        "watch sh -c './verify ../input'",
    ]);
}

#[test]
fn site_parity_pattern_text_is_not_a_path_requirement() {
    assert_command_site_parity(&[
        "grep -qF '../missing/input' checks/output.log",
        "rg '../missing/input' checks/output.log",
        "find checks -name ../missing/input",
        "head -n ../missing/input checks/output.log && test -f input",
        "test ../missing/input = text",
        "./verify ../missing/input",
        "./verify ../home/input",
        "./cat ../missing/input && test -f input",
        "./env cat ../missing/input && test -f input",
        "./sh -c 'cat ../missing/input' && test -f input",
        "sh -c './verify ../missing/input'",
    ]);
}

#[test]
fn site_parity_container_operands_are_not_host_path_requirements() {
    assert_command_site_parity(&[
        "docker run --rm -w /checks image ./verify ../input",
        "docker run --rm -v data:/data -w /checks image ./verify ../input",
    ]);
}

fn assert_path_reader_site_parity(allowed: &str, refused: &str, path: &str) {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let allowed = allowed.replace("workspace", &name);
    let refused = refused.replace("workspace", &name);
    let path = path.replace("workspace", &name);
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        assert_eq!(decisions(&set, root, &allowed, cwd), [true; 3], "{allowed}");
        assert_eq!(
            decisions(&set, root, &refused, cwd),
            [false; 3],
            "{refused}"
        );
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: refused.clone(),
            cwd,
        };
        let defect = archon_workflow::task_set_contract::acceptance_site_path_defect(
            &candidate, root, root, false,
        )
        .unwrap();
        assert!(defect.contains(&path), "{defect}");
        assert!(defect.contains("site does not create"), "{defect}");
    }
}

#[test]
fn site_parity_own_root_alias_requires_a_path_reader() {
    assert_path_reader_site_parity(
        "./verify ../workspace/input",
        "test -f ../workspace/input",
        "../workspace/input",
    );
}

#[test]
fn site_parity_missing_input_requires_a_path_reader() {
    for command in [
        "cat", "ls", "stat", "head", "tail", "wc", "diff", "cmp", "cp", "mv", "ln", "readlink",
        "realpath", "source", ".", "file", "du", "find", "test -f", "test -r", "test -L",
        "test -h", "[ -f", "[[ -f",
    ] {
        let suffix = match command {
            "[ -f" => " ]",
            "[[ -f" => " ]]",
            _ => "",
        };
        assert_path_reader_site_parity(
            "./verify ../missing/input",
            &format!("{command} ../missing/input{suffix}"),
            "../missing/input",
        );
    }
}

#[test]
fn site_parity_redirect_target_is_a_path_requirement() {
    for command in [
        "< ../missing/input ./verify",
        "./verify > ../missing/input",
        "./verify >> ../missing/input",
        "./verify 2> ../missing/input",
    ] {
        assert_path_reader_site_parity("./verify ../missing/input", command, "../missing/input");
    }
}

fn assert_program_file_read_site_parity(heredoc: bool) {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::create_dir(root.join("checks")).unwrap();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for (prefix, program, valid) in [
            ("", "open('../external/item')".to_owned(), false),
            ("", "open(\"../external/item\")".to_owned(), false),
            ("", format!("open('../{name}/input')"), false),
            ("cd checks; ", "open('../input')".to_owned(), true),
            ("cd $place; ", "open('../external/item')".to_owned(), true),
            ("", "relocate(); open('../external/item')".to_owned(), true),
        ] {
            let command = if heredoc {
                format!("{prefix}runner - <<'PROGRAM'\n{program}\nPROGRAM")
            } else {
                format!("{prefix}python -c '{}'", program.replace('\'', "'\\''"))
            };
            let actual = decisions(&set, root, &command, cwd);
            if actual != [valid; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected {valid}"));
            }
            if !valid && actual == [false; 3] {
                let mut candidate = set.contract().acceptance[0].clone();
                candidate.check = AcceptanceCheck::Command { command, cwd };
                let defect = archon_workflow::task_set_contract::acceptance_site_path_defect(
                    &candidate, root, root, false,
                )
                .unwrap();
                let named = if program.contains("external") {
                    "../external/item".to_owned()
                } else {
                    format!("../{name}/input")
                };
                assert!(defect.contains(&named), "{defect}");
                assert!(defect.contains("site does not create"), "{defect}");
                if program.contains("external") {
                    assert!(defect.contains("<site>/external/item"), "{defect}");
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn site_parity_inline_program_file_reads_keep_the_shared_refusal() {
    assert_program_file_read_site_parity(false);
}

#[test]
fn site_parity_heredoc_program_file_reads_keep_the_shared_refusal() {
    assert_program_file_read_site_parity(true);
}

#[test]
fn site_parity_quoted_redirect_symbols_are_data() {
    assert_command_site_parity(&[
        "printf '%s\\n' '<' '../missing/input' && test -f input",
        "printf '%s\\n' '>' '../missing/input' && test -f input",
        "printf '%s\\n' '>>' '../missing/input' && test -f input",
        "printf '%s\\n' '2>' '../missing/input' && test -f input",
        "printf '%s\\n' \\< '../missing/input' && test -f input",
    ]);
}

#[test]
fn site_parity_known_data_consumers_keep_heredocs_as_data() {
    assert_command_site_parity(&[
        "cat <<'EOF'\n../missing/input\nEOF\ntest -f input",
        "cat - <<'EOF'\nopen('../missing/input')\nEOF\ntest -f input",
        "tee <<'EOF'\nopen('../missing/input')\nEOF\ntest -f input",
        "grep input <<'EOF'\nopen('../missing/input')\nEOF\ntest -f input",
    ]);
}

#[test]
fn site_parity_non_read_program_literals_are_data() {
    assert_command_site_parity(&[
        "python3 -B -c 'print(\"../missing/input\")' && test -f input",
        "python3 -c 'value = \"../missing/input\"; print(value)' && test -f input",
        "python3 -c 'print(\"../missing/input\"); open(\"input\")' && test -f input",
        "python3 -c 'open(str(\"../missing/input\"))' && test -f input",
        "python3 -c 'print(\"open(\\\"../missing/input\\\")\")' && test -f input",
        "python3 -c 'Path(\"../missing/input\")' && test -f input",
        "python3 -c 'open(\"input\", \"../missing/input\")' && test -f input",
    ]);
    for program in [
        "open(\"../missing/input\")",
        "io.open(\"../missing/input\")",
        "Path(\"../missing/input\").read_text()",
        "pathlib.Path(\"../missing/input\").read_bytes()",
        "readFileSync(\"../missing/input\")",
        "fs.readFileSync(\"../missing/input\", \"utf8\")",
    ] {
        assert_path_reader_site_parity(
            "python3 -B -c 'print(\"../missing/input\")' && test -f input",
            &format!("python3 -c '{program}' && test -f input"),
            "../missing/input",
        );
    }
}

#[test]
fn site_parity_unknown_heredoc_receiver_keeps_file_reads() {
    assert_command_site_parity(&[
        "./verify --stdin-program 'open(\"../missing/input\")' && test -f input",
        "./verify '--stdin'-program 'open(\"../missing/input\")' && test -f input",
        "./verify --stdin-pro\\\ngram 'open(\"../missing/input\")' && test -f input",
    ]);
    for receiver in ["runner -", "./verify"] {
        assert_path_reader_site_parity(
            "cat <<'EOF'\nopen('../missing/input')\nEOF\ntest -f input",
            &format!("{receiver} <<'EOF'\nopen('../missing/input')\nEOF"),
            "../missing/input",
        );
    }
}

#[test]
fn site_parity_program_comments_do_not_prove_file_reads() {
    for program in [
        "// fs.readFileSync(\"../missing/input\")",
        "/* fs.readFileSync(\"../missing/input\") */",
        "# open(\"../missing/input\")",
    ] {
        assert_path_reader_site_parity(
            &format!("node -e '{program}' && test -f input"),
            "node -e 'fs.readFileSync(\"../missing/input\")'",
            "../missing/input",
        );
    }
}

#[test]
fn site_parity_directory_targets_are_filesystem_inputs() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let root = set.project.path();
    let name = root.file_name().unwrap().to_string_lossy();
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for program in ["cd", "pushd"] {
            for path in [format!("../{name}"), "../missing".into()] {
                let command = format!("{program} {path}");
                let actual = decisions(&set, root, &command, cwd);
                if actual != [false; 3] {
                    failures.push(format!("{cwd:?} {command}: {actual:?}, expected refusal"));
                }
                if let Some(defect) =
                    archon_workflow::task_set_contract::acceptance_site_path_defect(
                        &archon_workflow::task_set_contract::AcceptanceCriterion {
                            check: AcceptanceCheck::Command { command, cwd },
                            ..set.contract().acceptance[0].clone()
                        },
                        root,
                        root,
                        false,
                    )
                {
                    assert!(defect.contains(&path), "{defect}");
                    assert!(
                        defect.contains("leaves the") || defect.contains("site does not create"),
                        "{defect}"
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
